use anyhow::{Context, Result, bail, ensure};
use semver::Version;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::io::{Cursor, Read};

pub const REPOSITORY: &str = "MrMaxie/boothop";
pub const API: &str = "https://api.github.com/repos/MrMaxie/boothop/releases/latest";
pub const MAX_DOWNLOAD: usize = 128 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Platform {
    Windows,
    Linux,
}

impl Platform {
    pub fn asset(self, version: &Version) -> String {
        let suffix = match self {
            Self::Windows => "windows-x86_64.zip",
            Self::Linux => "linux-x86_64.tar.gz",
        };
        format!("boothop-v{version}-{suffix}")
    }
    pub fn executable(self) -> &'static str {
        match self {
            Self::Windows => "boothop.exe",
            Self::Linux => "boothop",
        }
    }
}

pub struct Response {
    pub status: u16,
    pub body: Vec<u8>,
}
pub trait Transport {
    fn get(&mut self, url: &str, limit: usize) -> Result<Response>;
}
pub trait Installer {
    fn replace(&mut self, executable: &[u8]) -> Result<String>;
}

#[derive(Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
}
#[derive(Deserialize)]
struct Release {
    tag_name: String,
    draft: bool,
    prerelease: bool,
    assets: Vec<Asset>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UpdateStatus {
    Current,
    Available {
        version: Version,
        asset: String,
        url: String,
        checksums: String,
    },
    Unavailable(String),
}

impl std::fmt::Display for UpdateStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Current => write!(f, "No update is required."),
            Self::Available { version, .. } => write!(
                f,
                "Version {version} is available. Run 'boothop update --install'."
            ),
            Self::Unavailable(reason) => write!(f, "Update status undetermined: {reason}"),
        }
    }
}

pub fn check(
    http: &mut impl Transport,
    platform: Platform,
    installed: &str,
) -> Result<UpdateStatus> {
    let response = http.get(API, 2 * 1024 * 1024)?;
    if response.status == 404 {
        let repo = http.get("https://api.github.com/repos/MrMaxie/boothop", 1024 * 1024)?;
        return Ok(UpdateStatus::Unavailable(if repo.status == 200 {
            "No published stable release is available.".into()
        } else {
            format!(
                "Repository {REPOSITORY} is unavailable or inaccessible (HTTP {}).",
                repo.status
            )
        }));
    }
    ensure!(
        response.status == 200,
        "GitHub release API returned HTTP {}",
        response.status
    );
    let release: Release =
        serde_json::from_slice(&response.body).context("Invalid release response")?;
    ensure!(
        !release.draft && !release.prerelease,
        "GitHub returned a non-stable release"
    );
    let version = Version::parse(
        release
            .tag_name
            .strip_prefix('v')
            .unwrap_or(&release.tag_name),
    )?;
    ensure!(version.pre.is_empty(), "Release version is a prerelease");
    if version <= Version::parse(installed)? {
        return Ok(UpdateStatus::Current);
    }
    let name = platform.asset(&version);
    let Some(asset) = release.assets.iter().find(|asset| asset.name == name) else {
        return Ok(UpdateStatus::Unavailable(format!(
            "Release {version} is missing {name}."
        )));
    };
    let Some(checksums) = release
        .assets
        .iter()
        .find(|asset| asset.name == "SHA256SUMS")
    else {
        return Ok(UpdateStatus::Unavailable(format!(
            "Release {version} is missing SHA256SUMS."
        )));
    };
    let prefix = format!(
        "https://github.com/{REPOSITORY}/releases/download/{}/",
        release.tag_name
    );
    ensure!(
        asset.browser_download_url == format!("{prefix}{name}"),
        "Unexpected release asset URL"
    );
    ensure!(
        checksums.browser_download_url == format!("{prefix}SHA256SUMS"),
        "Unexpected checksum URL"
    );
    Ok(UpdateStatus::Available {
        version,
        asset: name,
        url: asset.browser_download_url.clone(),
        checksums: checksums.browser_download_url.clone(),
    })
}

pub fn install(
    http: &mut impl Transport,
    installer: &mut impl Installer,
    platform: Platform,
    status: &UpdateStatus,
) -> Result<String> {
    let UpdateStatus::Available {
        asset,
        url,
        checksums,
        ..
    } = status
    else {
        return Ok(status.to_string());
    };
    let manifest = http.get(checksums, 64 * 1024)?;
    ensure!(
        manifest.status == 200,
        "Cannot download checksums (HTTP {})",
        manifest.status
    );
    let text = std::str::from_utf8(&manifest.body)?;
    let mut hashes = text.lines().filter_map(|line| {
        let (hash, name) = line.split_once(char::is_whitespace)?;
        (name.trim().trim_start_matches('*') == asset).then_some(hash)
    });
    let expected = hashes.next().context("Archive checksum is missing")?;
    ensure!(
        hashes.next().is_none()
            && expected.len() == 64
            && expected.bytes().all(|byte| byte.is_ascii_hexdigit()),
        "Invalid or duplicate archive checksum"
    );
    let archive = http.get(url, MAX_DOWNLOAD)?;
    ensure!(
        archive.status == 200,
        "Cannot download archive (HTTP {})",
        archive.status
    );
    ensure!(
        archive.body.len() <= MAX_DOWNLOAD,
        "Archive exceeds download limit"
    );
    let actual: String = Sha256::digest(&archive.body)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    ensure!(
        actual.eq_ignore_ascii_case(expected),
        "Archive checksum mismatch. Installed executable was not changed."
    );
    let executable = extract(platform, &archive.body)?;
    installer.replace(&executable)
}

pub fn extract(platform: Platform, archive: &[u8]) -> Result<Vec<u8>> {
    let mut output = Vec::new();
    match platform {
        Platform::Windows => {
            let mut zip = zip::ZipArchive::new(Cursor::new(archive))?;
            ensure!(
                zip.len() == 1,
                "Archive must contain exactly one executable"
            );
            let mut file = zip.by_index(0)?;
            ensure!(
                file.name()? == platform.executable() && !file.is_dir() && !file.is_symlink(),
                "Unexpected archive entry"
            );
            ensure!(
                file.size() <= MAX_DOWNLOAD as u64,
                "Executable exceeds size limit"
            );
            (&mut file)
                .take(MAX_DOWNLOAD as u64 + 1)
                .read_to_end(&mut output)?;
        }
        Platform::Linux => {
            let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(Cursor::new(archive)));
            let mut entries = tar.entries()?;
            let mut file = entries.next().context("Empty archive")??;
            ensure!(
                file.path()?.as_ref() == std::path::Path::new(platform.executable())
                    && file.header().entry_type().is_file(),
                "Unexpected archive entry"
            );
            ensure!(
                file.size() <= MAX_DOWNLOAD as u64,
                "Executable exceeds size limit"
            );
            (&mut file)
                .take(MAX_DOWNLOAD as u64 + 1)
                .read_to_end(&mut output)?;
            ensure!(entries.next().is_none(), "Archive contains extra entries");
        }
    }
    ensure!(
        output.len() <= MAX_DOWNLOAD,
        "Executable exceeds size limit"
    );
    match platform {
        Platform::Linux => ensure!(
            output.starts_with(b"\x7fELF")
                && output.get(4) == Some(&2)
                && output.get(5) == Some(&1)
                && output.get(18..20) == Some(&[62, 0]),
            "Expected an x86_64 ELF executable"
        ),
        Platform::Windows => {
            ensure!(
                output.starts_with(b"MZ") && output.len() >= 64,
                "Expected a PE executable"
            );
            let offset = u32::from_le_bytes(output[60..64].try_into()?) as usize;
            ensure!(
                output.get(offset..offset.saturating_add(6)) == Some(b"PE\0\0\x64\x86"),
                "Expected an x86_64 PE executable"
            );
        }
    }
    Ok(output)
}

pub fn require_available(status: &UpdateStatus) -> Result<()> {
    if let UpdateStatus::Unavailable(reason) = status {
        bail!("{reason}");
    }
    Ok(())
}
