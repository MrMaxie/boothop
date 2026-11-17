use super::firmware;
use crate::boot::{Selection, Target};
use anyhow::{Context, Result, ensure};
use std::path::{Path, PathBuf};

struct Volume {
    partition: uuid::Uuid,
    filesystem: String,
    root: PathBuf,
    kind: String,
    #[cfg(not(windows))]
    source: String,
}

pub fn loader(target: &Target, roots: &[PathBuf]) -> Result<u16> {
    let (kind, path) = match &target.selection {
        Selection::SystemdBoot(_) => ("systemd", Path::new(&target.location)),
        Selection::OfflineBcd { store, .. } => ("windows", Path::new(store)),
        Selection::Grub { config, .. } => ("grub", Path::new(config)),
        Selection::Bcd(_) => {
            let current = firmware::read("BootCurrent", firmware::EFI_GUID)?
                .context("Current firmware path is unavailable")?;
            ensure!(current.len() == 2, "Invalid current firmware path");
            let number = u16::from_le_bytes([current[0], current[1]]);
            ensure!(
                firmware::loader_candidates("windows")?
                    .iter()
                    .any(|candidate| candidate.0 == number),
                "The live BCD store cannot be linked to the current firmware path"
            );
            return Ok(number);
        }
        Selection::Firmware(number) => return Ok(*number),
    };
    let source = volume(path)?;
    let mut matches = Vec::new();
    for (number, firmware_path) in firmware::loader_candidates(kind)? {
        if kind != "grub" {
            if firmware_path.partition == Some(source.partition) {
                matches.push(number);
            }
            continue;
        }
        let relative = path
            .strip_prefix(&source.root)
            .context("Boot configuration is outside its volume")?;
        let relative = format!("/{}", relative.to_string_lossy().replace('\\', "/"));
        for root in roots {
            let Ok(esp) = volume(root) else {
                continue;
            };
            if firmware_path.partition != Some(esp.partition) {
                continue;
            }
            for file in &firmware_path.files {
                let executable = root.join(file.trim_start_matches('\\').replace('\\', "/"));
                let stub = executable.with_file_name("grub.cfg");
                if std::fs::read_to_string(stub).is_ok_and(|text| {
                    crate::boot::grub_redirect_matches(&text, &source.filesystem, &relative)
                }) {
                    matches.push(number);
                }
            }
        }
    }
    matches.sort();
    matches.dedup();
    ensure!(
        matches.len() == 1,
        "Cannot prove a unique firmware-to-{kind} path for this configuration ({} matches). Inspect its EFI partition and loader redirect.",
        matches.len()
    );
    Ok(matches[0])
}

#[cfg(not(windows))]
fn volume(path: &Path) -> Result<Volume> {
    use super::process::{find, run};
    let program =
        find("findmnt").context("Boot path validation requires findmnt from util-linux")?;
    let path = path.to_str().context("Boot path is not UTF-8")?;
    let result = run(
        &program,
        &[
            "--json",
            "--target",
            path,
            "--output",
            "TARGET,UUID,PARTUUID,FSTYPE,SOURCE",
        ],
        std::time::Duration::from_secs(5),
    )?;
    ensure!(
        result.code == 0,
        "Cannot identify the boot filesystem: {}",
        result.stderr
    );
    let value: serde_json::Value = serde_json::from_str(&result.stdout)?;
    let volumes = value["filesystems"]
        .as_array()
        .context("Invalid findmnt response")?;
    ensure!(
        volumes.len() == 1,
        "Ambiguous filesystem for boot configuration"
    );
    let info = &volumes[0];
    let partition = info["partuuid"]
        .as_str()
        .context("Boot filesystem has no GPT partition identity")?;
    Ok(Volume {
        partition: uuid::Uuid::parse_str(partition)?,
        #[cfg(not(windows))]
        kind: info["fstype"]
            .as_str()
            .context("Missing filesystem type")?
            .into(),
        #[cfg(not(windows))]
        source: info["source"]
            .as_str()
            .context("Missing filesystem source")?
            .into(),
        filesystem: info["uuid"].as_str().unwrap_or_default().into(),
        root: info["target"]
            .as_str()
            .context("Missing filesystem mount point")?
            .into(),
    })
}

#[cfg(windows)]
fn volume(path: &Path) -> Result<Volume> {
    use windows_sys::Win32::Storage::FileSystem::GetVolumeInformationW;
    let escaped = path
        .to_str()
        .context("Boot path is not UTF-8")?
        .replace('\'', "''");
    let script = format!(
        "$path='{escaped}'; $matches=@(Get-Partition | ForEach-Object {{ $p=$_; $_.AccessPaths | Where-Object {{$path.StartsWith($_,[StringComparison]::OrdinalIgnoreCase)}} | ForEach-Object {{[pscustomobject]@{{root=$_;guid=$p.Guid}}}} }} | Sort-Object {{ $_.root.Length }} -Descending); if($matches.Count -eq 0){{throw 'Cannot map boot path to a partition'}}; $matches[0] | ConvertTo-Json -Compress"
    );
    let info: serde_json::Value = serde_json::from_str(&super::windows::powershell(&script)?)?;
    let root = info["root"].as_str().context("Missing volume path")?;
    let mut serial = 0;
    let mut filesystem = [0_u16; 64];
    let wide = root.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
    let ok = unsafe {
        GetVolumeInformationW(
            wide.as_ptr(),
            std::ptr::null_mut(),
            0,
            &mut serial,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            filesystem.as_mut_ptr(),
            filesystem.len() as u32,
        )
    };
    let filesystem = String::from_utf16_lossy(&filesystem)
        .trim_end_matches('\0')
        .to_owned();
    let uuid = if ok != 0 && filesystem.starts_with("FAT") {
        format!("{:04X}-{:04X}", serial >> 16, serial & 0xffff)
    } else {
        String::new()
    };
    let Some(partition) = info["guid"].as_str() else {
        anyhow::bail!("Boot volume has no GPT identity")
    };
    Ok(Volume {
        partition: uuid::Uuid::parse_str(partition)?,
        #[cfg(not(windows))]
        kind: info["fstype"]
            .as_str()
            .context("Missing filesystem type")?
            .into(),
        #[cfg(not(windows))]
        source: info["source"]
            .as_str()
            .context("Missing filesystem source")?
            .into(),
        filesystem: uuid,
        root: root.into(),
        kind: filesystem,
    })
}

pub fn validate_grub_environment(path: &Path) -> Result<()> {
    let volume = volume(path)?;
    ensure!(
        ["ext2", "ext3", "ext4", "vfat", "FAT", "FAT32"].contains(&volume.kind.as_str()),
        "GRUB one-shot consumption cannot be established on {}",
        volume.kind
    );
    #[cfg(not(windows))]
    {
        let program =
            super::process::find("lsblk").context("GRUB environment validation requires lsblk")?;
        let output = super::process::run(
            &program,
            &[
                "--noheadings",
                "--nodeps",
                "--output",
                "TYPE",
                "--",
                &volume.source,
            ],
            std::time::Duration::from_secs(5),
        )?;
        ensure!(
            output.code == 0 && ["part", "disk"].contains(&output.stdout.trim()),
            "GRUB environment requires a directly accessible disk partition; mapped or encrypted storage is not verified"
        );
    }
    Ok(())
}
