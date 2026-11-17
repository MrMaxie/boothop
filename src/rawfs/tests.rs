use super::*;
use std::io::{Cursor, Write};

fn fixture(name: &str) -> std::fs::File {
    let compressed = std::fs::File::open(format!(
        "{}/tests/fixtures/{name}-boot.img.gz",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap();
    let mut image = tempfile::tempfile().unwrap();
    std::io::copy(&mut flate2::read::GzDecoder::new(compressed), &mut image).unwrap();
    image.rewind().unwrap();
    image
}

#[test]
fn linux_filesystems_expose_actual_boot_files_without_mounting() {
    for name in [
        "ext2",
        "ext3",
        "ext4",
        "btrfs-zlib",
        "btrfs-lzo",
        "btrfs-zstd",
        "xfs",
    ] {
        let image = fixture(name);
        let length = image.metadata().unwrap().len();
        let inventory = discover(
            Partition::new(image, 0, length, 4096).unwrap(),
            "fixture-partition",
        )
        .unwrap_or_else(|error| panic!("{name}: {error:#}"))
        .unwrap();
        assert!(
            inventory.warnings.is_empty(),
            "{name}: {:?}",
            inventory.warnings
        );
        assert_eq!(
            inventory
                .items
                .iter()
                .filter(|t| t.label == "Fixture Linux")
                .count(),
            2,
            "{name}: {:?}",
            inventory.items
        );
        assert_eq!(
            inventory
                .items
                .iter()
                .filter(|t| t.label == "Fixture BLS")
                .count(),
            1,
            "{name}"
        );
        assert!(
            inventory
                .items
                .iter()
                .all(|t| !t.current && t.unavailable.is_some())
        );
        assert_eq!(
            inventory
                .items
                .iter()
                .filter(|t| t.label == "Fixture UKI")
                .count(),
            1,
            "{name}"
        );
        assert!(boot::resolve(inventory.clone(), &inventory.items[0].id).is_err());
    }
}

#[test]
fn partition_window_enforces_alignment_bounds_and_read_only_access() {
    struct Aligned(Cursor<Vec<u8>>);
    impl Read for Aligned {
        fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
            assert!(self.0.position().is_multiple_of(4096));
            assert!(bytes.len().is_multiple_of(4096));
            assert!(self.0.position() >= 4096 && self.0.position() + bytes.len() as u64 <= 12288);
            self.0.read(bytes)
        }
    }
    impl Seek for Aligned {
        fn seek(&mut self, at: SeekFrom) -> std::io::Result<u64> {
            self.0.seek(at)
        }
    }
    let mut partition =
        Partition::new(Aligned(Cursor::new(vec![7; 16384])), 4096, 8192, 4096).unwrap();
    partition.seek(SeekFrom::Start(4095)).unwrap();
    let mut result = [0; 3];
    partition.read_exact(&mut result).unwrap();
    assert_eq!(result, [7; 3]);
    assert!(partition.write_all(b"forbidden").is_err());
    partition.seek(SeekFrom::End(1)).unwrap();
    assert_eq!(partition.read(&mut result).unwrap(), 0);
    partition.seek(SeekFrom::Start(8193)).unwrap();
    assert_eq!(partition.read(&mut result).unwrap(), 0);
    partition.seek(SeekFrom::End(-1)).unwrap();
    assert_eq!(partition.read(&mut result).unwrap(), 1);
    assert_eq!(partition.read(&mut result).unwrap(), 0);
}

#[test]
fn unknown_and_truncated_filesystems_are_not_confused() {
    let empty = vec![0; 131072];
    assert!(
        discover(
            Partition::new(Cursor::new(empty), 0, 131072, 512).unwrap(),
            "empty"
        )
        .unwrap()
        .is_none()
    );
    for (offset, signature) in [
        (0, b"XFSB".as_slice()),
        (1080, &[0x53, 0xef]),
        (65600, b"_BHRfS_M"),
    ] {
        let mut corrupt = vec![0; 131072];
        corrupt[offset..offset + signature.len()].copy_from_slice(signature);
        assert!(
            discover(
                Partition::new(Cursor::new(corrupt), 0, 131072, 512).unwrap(),
                "broken"
            )
            .is_err()
        );
    }
}

#[test]
fn file_limits_prevent_unbounded_boot_config_reads() {
    assert!(bounded(Cursor::new(vec![0; 1025]), 1024).is_err());
    assert!(Partition::new(Cursor::new(vec![]), u64::MAX - 511, 1024, 512).is_err());
}

#[test]
fn partition_identity_disambiguates_cloned_filesystems() {
    let mut ids = Vec::new();
    for identity in [
        "disk-a:partition-a",
        "disk-b:partition-b",
        "disk-a:partition-a",
    ] {
        let image = fixture("ext4");
        let length = image.metadata().unwrap().len();
        ids.push(
            discover(Partition::new(image, 0, length, 512).unwrap(), identity)
                .unwrap()
                .unwrap()
                .items[0]
                .id
                .clone(),
        );
    }
    assert_ne!(ids[0], ids[1]);
    assert_eq!(ids[0], ids[2]);
}

#[test]
fn filesystem_inside_a_partition_uses_only_partition_relative_offsets() {
    let mut image = fixture("ext4");
    let size = image.metadata().unwrap().len();
    let mut disk = tempfile::tempfile().unwrap();
    disk.write_all(&[0xa5; 4096]).unwrap();
    std::io::copy(&mut image, &mut disk).unwrap();
    disk.write_all(&[0x5a; 4096]).unwrap();
    let inventory = discover(
        Partition::new(disk, 4096, size, 4096).unwrap(),
        "offset-partition",
    )
    .unwrap()
    .unwrap();
    assert!(inventory.warnings.is_empty(), "{:?}", inventory.warnings);
    assert!(inventory.items.iter().any(|t| t.label == "Fixture Linux"));
}

#[test]
fn one_unreadable_boot_file_does_not_hide_other_entries() {
    struct Partial;
    impl Filesystem for Partial {
        fn identity(&self) -> String {
            "fixture".into()
        }
        fn read(&mut self, _: u64, path: &str, _: usize) -> Result<Option<Vec<u8>>> {
            if path == "/grub/grub.cfg" {
                anyhow::bail!("Simulated checksum mismatch");
            }
            if path == "/boot/grub/grub.cfg" {
                return Ok(Some(
                    b"menuentry 'Readable Linux' --id readable {\nlinux /kernel\n}\n".to_vec(),
                ));
            }
            Ok(None)
        }
        fn list(&mut self, _: u64, _: &str) -> Result<Option<Vec<String>>> {
            Ok(None)
        }
    }
    let mut inventory = Inventory {
        items: vec![],
        warnings: vec![],
    };
    scan(&mut Partial, 0, "fixture", &mut inventory);
    assert_eq!(inventory.items.len(), 1);
    assert_eq!(inventory.items[0].label, "Readable Linux");
    assert_eq!(inventory.warnings.len(), 1);
    assert!(inventory.warnings[0].contains("Simulated checksum mismatch"));
}
