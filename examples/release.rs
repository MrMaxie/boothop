//! Release tooling only. Never loads or executes the packaged application.
use anyhow::{Context, Result, bail, ensure};
use boothop::update::Platform;
use clap::{Arg, Command};
use semver::Version;
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
};

fn validate(version: &str, tag: &str) -> Result<Version> {
    let version = Version::parse(version)?;
    ensure!(
        version.pre.is_empty() && version.build.is_empty() && tag == format!("v{version}"),
        "Select an existing stable v<package-version> tag"
    );
    Ok(version)
}

fn package(root: &Path, version: &Version, platform: Platform, target: &str) -> Result<PathBuf> {
    let expected = match platform {
        Platform::Windows => "x86_64-pc-windows-msvc",
        Platform::Linux => "x86_64-unknown-linux-musl",
    };
    ensure!(target == expected, "Target does not match release platform");
    let name = platform.executable();
    let mut source = fs::File::open(root.join("target").join(target).join("release").join(name))?;
    fs::create_dir_all(root.join("dist"))?;
    let path = root.join("dist").join(platform.asset(version));
    let output = fs::File::create(&path)?;
    match platform {
        Platform::Windows => {
            let mut zip = zip::ZipWriter::new(output);
            zip.start_file(
                name,
                zip::write::SimpleFileOptions::default()
                    .compression_method(zip::CompressionMethod::Deflated),
            )?;
            std::io::copy(&mut source, &mut zip)?;
            zip.finish()?;
        }
        Platform::Linux => {
            let gzip = flate2::write::GzEncoder::new(output, flate2::Compression::default());
            let mut tar = tar::Builder::new(gzip);
            let mut header = tar::Header::new_ustar();
            header.set_size(source.metadata()?.len());
            header.set_mode(0o755);
            header.set_uid(0);
            header.set_gid(0);
            header.set_mtime(0);
            header.set_cksum();
            tar.append_data(&mut header, name, &mut source)?;
            tar.into_inner()?.finish()?;
        }
    }
    Ok(path)
}

fn checksums(root: &Path, version: &Version) -> Result<()> {
    let mut lines = String::new();
    for platform in [Platform::Windows, Platform::Linux] {
        let name = platform.asset(version);
        let mut file = fs::File::open(root.join("dist").join(&name))
            .context("Both platform archives are required before publication")?;
        let mut hash = Sha256::new();
        let mut buffer = [0; 65536];
        loop {
            let count = file.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            hash.update(&buffer[..count]);
        }
        let digest: String = hash
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        lines.push_str(&format!("{digest}  {name}\n"));
    }
    fs::write(root.join("dist/SHA256SUMS"), lines)?;
    Ok(())
}

fn word(data: &[u8], offset: usize, size: usize) -> Result<u32> {
    let bytes = data
        .get(offset..offset.checked_add(size).context("Invalid PE offset")?)
        .context("Truncated PE image")?;
    match size {
        2 => Ok(u32::from(u16::from_le_bytes(bytes.try_into()?))),
        4 => Ok(u32::from_le_bytes(bytes.try_into()?)),
        _ => bail!("Invalid PE field size"),
    }
}

fn windows_dependencies(data: &[u8]) -> Result<Vec<String>> {
    ensure!(data.starts_with(b"MZ"), "Expected a Windows PE executable");
    let pe = word(data, 60, 4)? as usize;
    ensure!(
        data.get(pe..pe.saturating_add(6)) == Some(b"PE\0\0\x64\x86"),
        "Expected an x86_64 Windows executable"
    );
    let sections = word(data, pe + 6, 2)? as usize;
    let optional_size = word(data, pe + 20, 2)? as usize;
    let optional = pe + 24;
    ensure!(
        optional_size >= 128 && word(data, optional, 2)? == 0x20b,
        "Expected a PE32+ optional header"
    );
    let section_table = optional + optional_size;
    let offset = |rva: u32| -> Result<usize> {
        for index in 0..sections {
            let base = section_table + index * 40;
            let address = word(data, base + 12, 4)?;
            let raw_size = word(data, base + 16, 4)?;
            if let Some(delta) = rva.checked_sub(address)
                && delta < raw_size
            {
                let at = u64::from(word(data, base + 20, 4)?) + u64::from(delta);
                ensure!(at < data.len() as u64, "Invalid PE import address");
                return Ok(at as usize);
            }
        }
        bail!("Invalid PE import address")
    };
    let imports = word(data, optional + 120, 4)?;
    let mut dependencies = Vec::new();
    if imports != 0 {
        let mut table = offset(imports)?;
        loop {
            let descriptor = data
                .get(table..table + 20)
                .context("Truncated PE import table")?;
            if descriptor.iter().all(|&byte| byte == 0) {
                break;
            }
            let start = offset(word(data, table + 12, 4)?)?;
            let name = &data[start..data.len().min(start + 256)];
            let end = name
                .iter()
                .position(|&byte| byte == 0)
                .context("Unterminated PE import name")?;
            dependencies.push(std::str::from_utf8(&name[..end])?.to_ascii_lowercase());
            table += 20;
        }
    }
    ensure!(
        !dependencies
            .iter()
            .any(|name| ["vcruntime", "msvcp", "msvcr"]
                .iter()
                .any(|prefix| name.starts_with(prefix))),
        "Windows release requires a separately installed Visual C++ runtime"
    );
    dependencies.sort();
    dependencies.dedup();
    Ok(dependencies)
}

fn main() -> Result<()> {
    let args = Command::new("release")
        .arg(Arg::new("action").required(true).value_parser([
            "validate",
            "package",
            "checksums",
            "verify-windows",
        ]))
        .arg(
            Arg::new("platform")
                .long("platform")
                .value_parser(["windows", "linux"]),
        )
        .arg(Arg::new("target").long("target"))
        .get_matches();
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let version = validate(
        env!("CARGO_PKG_VERSION"),
        &std::env::var("RELEASE_TAG").context("RELEASE_TAG is required")?,
    )?;
    match args.get_one::<String>("action").map(String::as_str) {
        Some("package") => {
            let platform = match args.get_one::<String>("platform").map(String::as_str) {
                Some("windows") => Platform::Windows,
                Some("linux") => Platform::Linux,
                _ => bail!("Packaging requires --platform"),
            };
            package(
                root,
                &version,
                platform,
                args.get_one::<String>("target")
                    .context("Packaging requires --target")?,
            )?;
        }
        Some("checksums") => checksums(root, &version)?,
        Some("verify-windows") => {
            let data = fs::read(root.join("target/x86_64-pc-windows-msvc/release/boothop.exe"))?;
            println!(
                "Windows imports: {}",
                windows_dependencies(&data)?.join(", ")
            );
        }
        Some("validate") => (),
        _ => bail!("Unknown release action"),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    #[test]
    fn version_tag_must_match_a_stable_package_version() {
        assert!(validate("0.1.0", "v0.1.0").is_ok());
        for tag in ["main", "v0.2.0", "v0.1.0-rc.1", "../v0.1.0"] {
            assert!(validate("0.1.0", tag).is_err());
        }
        assert!(validate("0.1.0-rc.1", "v0.1.0-rc.1").is_err());
    }

    #[test]
    fn publication_requires_both_archives() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("dist")).unwrap();
        fs::write(
            root.path().join("dist/boothop-v0.1.0-windows-x86_64.zip"),
            b"fixture",
        )
        .unwrap();
        assert!(checksums(root.path(), &Version::new(0, 1, 0)).is_err());
        assert!(!root.path().join("dist/SHA256SUMS").exists());
    }

    #[test]
    fn packages_preserve_contents_and_match_the_updater_contract() {
        let root = tempfile::tempdir().unwrap();
        let version = Version::new(0, 1, 0);
        for (platform, target) in [
            (Platform::Windows, "x86_64-pc-windows-msvc"),
            (Platform::Linux, "x86_64-unknown-linux-musl"),
        ] {
            let path = root.path().join("target").join(target).join("release");
            fs::create_dir_all(&path).unwrap();
            fs::write(path.join(platform.executable()), b"simulated executable").unwrap();
            let archive = package(root.path(), &version, platform, target).unwrap();
            let mut content = Vec::new();
            match platform {
                Platform::Windows => {
                    let mut zip = zip::ZipArchive::new(fs::File::open(archive).unwrap()).unwrap();
                    assert_eq!(zip.len(), 1);
                    let mut entry = zip.by_index(0).unwrap();
                    assert_eq!(entry.name().unwrap(), platform.executable());
                    entry.read_to_end(&mut content).unwrap();
                }
                Platform::Linux => {
                    let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(
                        fs::File::open(archive).unwrap(),
                    ));
                    let mut entries = tar.entries().unwrap();
                    let mut entry = entries.next().unwrap().unwrap();
                    assert_eq!(entry.path().unwrap(), Path::new(platform.executable()));
                    assert_eq!(entry.header().mode().unwrap(), 0o755);
                    entry.read_to_end(&mut content).unwrap();
                    assert!(entries.next().is_none());
                }
            }
            assert_eq!(content, b"simulated executable");
        }
        assert!(package(root.path(), &version, Platform::Linux, "../wrong").is_err());
        checksums(root.path(), &version).unwrap();
        let sums = fs::read_to_string(root.path().join("dist/SHA256SUMS")).unwrap();
        assert_eq!(sums.lines().count(), 2);
        for line in sums.lines() {
            let (hash, name) = line.split_once("  ").unwrap();
            let expected: String =
                Sha256::digest(fs::read(root.path().join("dist").join(name)).unwrap())
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect();
            assert_eq!(hash, expected);
        }
    }

    #[test]
    fn pe_imports_are_inspected_without_executing_the_image() {
        let mut data = vec![0; 1024];
        data[..2].copy_from_slice(b"MZ");
        data[60..64].copy_from_slice(&64u32.to_le_bytes());
        data[64..70].copy_from_slice(b"PE\0\0\x64\x86");
        for (at, value) in [(70, 1u16), (84, 240), (88, 0x20b)] {
            data[at..at + 2].copy_from_slice(&value.to_le_bytes());
        }
        for (at, value) in [
            (208, 4096u32),
            (336, 512),
            (340, 4096),
            (344, 512),
            (348, 512),
            (524, 4184),
        ] {
            data[at..at + 4].copy_from_slice(&value.to_le_bytes());
        }
        data[600..613].copy_from_slice(b"kernel32.dll\0");
        assert_eq!(windows_dependencies(&data).unwrap(), ["kernel32.dll"]);
        data[600..616].copy_from_slice(b"vcruntime140.dll");
        assert!(windows_dependencies(&data).is_err());
        for length in [0, 2, 64, 128, 530] {
            assert!(windows_dependencies(&data[..length]).is_err());
        }
    }
}
