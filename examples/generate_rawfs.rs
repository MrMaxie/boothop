//! Regenerate synthetic filesystem images on Linux without mounting them.
#[cfg(not(target_os = "linux"))]
fn main() -> anyhow::Result<()> {
    anyhow::bail!("Fixture generation requires Linux and e2fsprogs, btrfs-progs, and xfsprogs")
}

#[cfg(target_os = "linux")]
fn main() -> anyhow::Result<()> {
    let output = std::env::args_os()
        .nth(1)
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures"));
    linux::generate(&output)
}

#[cfg(target_os = "linux")]
mod linux {
    use anyhow::{Context, Result, ensure};
    use std::{
        fs,
        path::Path,
        process::{Command, Stdio},
    };
    const UUID: &str = "12345678-1234-4321-8765-123456789abc";

    fn tree(root: &Path) -> Result<()> {
        for directory in ["boot/grub", "boot/loader/entries", "boot/EFI/Linux"] {
            fs::create_dir_all(root.join(directory))?;
        }
        fs::write(
            root.join("boot/grub/grub.cfg"),
            "menuentry 'Fixture Linux' --id fixture-linux {\n linux /vmlinuz\n}\n".to_owned()
                + &"# Synthetic boot configuration for read-only inspection.\n".repeat(2048),
        )?;
        fs::write(
            root.join("boot/loader/entries/fixture.conf"),
            format!("title Fixture BLS\nlinux /vmlinuz\noptions root=UUID={UUID}\n"),
        )?;
        std::os::unix::fs::symlink("grub", root.join("boot/grub2"))?;
        let mut image = vec![0; 2048];
        image[..2].copy_from_slice(b"MZ");
        image[60..64].copy_from_slice(&128u32.to_le_bytes());
        image[128..132].copy_from_slice(b"PE\0\0");
        for (at, value) in [
            (132, 0x8664u16),
            (134, 2),
            (148, 240),
            (152, 0x20b),
            (220, 10),
        ] {
            image[at..at + 2].copy_from_slice(&value.to_le_bytes());
        }
        for (index, name, offset, size) in
            [(0, b".osrel", 1024u32, 64u32), (1, b".linux", 1536, 128)]
        {
            let start = 392 + index * 40;
            image[start..start + 6].copy_from_slice(name);
            image[start + 16..start + 20].copy_from_slice(&size.to_le_bytes());
            image[start + 20..start + 24].copy_from_slice(&offset.to_le_bytes());
        }
        let text = b"PRETTY_NAME=\"Fixture UKI\"\n";
        image[1024..1024 + text.len()].copy_from_slice(text);
        fs::write(root.join("boot/EFI/Linux/fixture.efi"), image)?;
        Ok(())
    }

    fn proto(root: &Path) -> Result<String> {
        let mut children = fs::read_dir(root)?
            .map(|e| e.map(|e| e.path()))
            .collect::<std::io::Result<Vec<_>>>()?;
        children.sort();
        let mut output = String::new();
        for path in children {
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .context("Invalid fixture name")?;
            let kind = fs::symlink_metadata(&path)?.file_type();
            if kind.is_symlink() {
                output.push_str(&format!(
                    "{name} l--777 0 0 {}\n",
                    fs::read_link(&path)?.display()
                ));
            } else if kind.is_dir() {
                output.push_str(&format!("{name} d--755 0 0\n{}$\n", proto(&path)?));
            } else {
                output.push_str(&format!("{name} ---644 0 0 {}\n", path.display()));
            }
        }
        Ok(output)
    }

    pub fn generate(output: &Path) -> Result<()> {
        // A fixed temporary parent avoids spaces in XFS prototype paths.
        let work = tempfile::Builder::new()
            .prefix("boothop-fixtures-")
            .tempdir_in("/tmp")?;
        let source = work.path().join("root");
        tree(&source)?;
        fs::create_dir_all(output)?;
        for name in [
            "ext2",
            "ext3",
            "ext4",
            "btrfs-zlib",
            "btrfs-lzo",
            "btrfs-zstd",
            "xfs",
        ] {
            let image = work.path().join(format!("{name}.img"));
            let size = if name == "xfs" {
                320
            } else if name.starts_with("btrfs") {
                128
            } else {
                32
            } * 1024
                * 1024;
            let file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&image)?;
            file.set_len(size)?;
            drop(file);
            ensure!(
                fs::symlink_metadata(&image)?.file_type().is_file()
                    && image.parent() == Some(work.path()),
                "Fixture destination must be a new regular file"
            );
            let mut command = if name.starts_with("ext") {
                let mut command = Command::new(format!("/usr/sbin/mkfs.{name}"));
                command.args(["-q", "-U", UUID, "-d"]).arg(&source);
                command
            } else if let Some(algorithm) = name.strip_prefix("btrfs-") {
                let root = work.path().join(name);
                tree(&root.join("@"))?;
                let mut command = Command::new("/usr/sbin/mkfs.btrfs");
                command
                    .args(["-q", "--nodiscard", "-U", UUID, "--rootdir"])
                    .arg(root)
                    .args(["--subvol", "default:@", "--compress", algorithm]);
                command
            } else {
                let specification = work.path().join("xfs.proto");
                fs::write(
                    &specification,
                    format!("/dev/null\n0 0\nd--755 0 0\n{}$\n", proto(&source)?),
                )?;
                let mut command = Command::new("/usr/sbin/mkfs.xfs");
                command
                    .args(["-q", "-m", &format!("uuid={UUID}"), "-p"])
                    .arg(specification);
                command
            };
            ensure!(
                command
                    .arg(&image)
                    .stdout(Stdio::null())
                    .status()
                    .context("Cannot run fixture formatter")?
                    .success(),
                "Formatting {name} failed"
            );
            let partition = boothop::rawfs::Partition::new(fs::File::open(&image)?, 0, size, 4096)?;
            let inventory = boothop::rawfs::discover(partition, "generated-fixture")?
                .context("Fixture filesystem was not recognized")?;
            ensure!(
                inventory.warnings.is_empty(),
                "{name}: {:?}",
                inventory.warnings
            );
            for label in ["Fixture Linux", "Fixture BLS", "Fixture UKI"] {
                ensure!(
                    inventory.items.iter().any(|item| item.label == label),
                    "Missing {label} in {name}"
                );
            }
            let mut archive = flate2::GzBuilder::new().mtime(0).write(
                fs::File::create(output.join(format!("{name}-boot.img.gz")))?,
                flate2::Compression::default(),
            );
            std::io::copy(&mut fs::File::open(image)?, &mut archive)?;
            archive.finish()?;
        }
        Ok(())
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn fixture_tree_contains_boot_data_and_xfs_prototype() {
            let directory = tempfile::tempdir().unwrap();
            tree(directory.path()).unwrap();
            assert_eq!(
                fs::read_link(directory.path().join("boot/grub2")).unwrap(),
                Path::new("grub")
            );
            let image = fs::read(directory.path().join("boot/EFI/Linux/fixture.efi")).unwrap();
            assert_eq!(
                boothop::boot::unified_entry("fixture.efi", &image, "fixture")
                    .unwrap()
                    .label,
                "Fixture UKI"
            );
            let prototype = proto(directory.path()).unwrap();
            assert!(prototype.contains("grub2 l--777 0 0 grub\n"));
            assert!(prototype.contains("grub.cfg ---644 0 0 "));
        }
    }
}
