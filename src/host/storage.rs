use crate::{
    boot::Inventory,
    disk::{Disk, Hibernation},
};
use anyhow::{Context, Result};
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
};

pub fn inventory(check: bool) -> Result<Inventory<Disk>> {
    Ok(crate::disk::inspect_inventory(enumerate()?, check, inspect))
}

fn inspect(path: &str) -> Result<Hibernation> {
    let file = File::open(path.trim_end_matches('\\'))
        .context("Cannot open volume read-only. Check access permissions.")?;
    let mut reader = SectorReader { file, position: 0 };
    crate::disk::inspect_ntfs(&mut reader)
}

// Raw Windows volumes require sector-aligned reads. Expose byte-oriented reads to NTFS.
struct SectorReader {
    file: File,
    position: u64,
}
impl Read for SectorReader {
    fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
        if output.is_empty() {
            return Ok(0);
        }
        let start = self.position / 4096 * 4096;
        self.file.seek(SeekFrom::Start(start))?;
        let mut sector = [0; 4096];
        let length = self.file.read(&mut sector)?;
        let offset = (self.position - start) as usize;
        if offset >= length {
            return Ok(0);
        }
        let count = output.len().min(length - offset);
        output[..count].copy_from_slice(&sector[offset..offset + count]);
        self.position += count as u64;
        Ok(count)
    }
}
impl Seek for SectorReader {
    fn seek(&mut self, position: SeekFrom) -> std::io::Result<u64> {
        let next = match position {
            SeekFrom::Start(value) => Some(value),
            SeekFrom::Current(offset) => self.position.checked_add_signed(offset),
            SeekFrom::End(offset) => self.file.seek(SeekFrom::End(0))?.checked_add_signed(offset),
        }
        .ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::InvalidInput, "Invalid volume seek")
        })?;
        self.position = next;
        Ok(next)
    }
}

#[cfg(windows)]
fn enumerate() -> Result<Inventory<Disk>> {
    let script = r#"$warnings=[Collections.Generic.List[string]]::new(); $items=@(Get-Disk | ForEach-Object {
        $disk=$_;
        [pscustomobject]@{id=('disk:'+$disk.Number);parent=$null;size=[uint64]$disk.Size;filesystem='';label=$disk.FriendlyName;mount='';hibernation=$null};
        try { Get-Partition -DiskNumber $disk.Number | ForEach-Object {
            $partition=$_; $volume=$partition | Get-Volume -ErrorAction SilentlyContinue;
            $id=if($volume.Path){$volume.Path}else{'partition:'+$disk.Number+':'+$partition.PartitionNumber};
            $mount=(@($partition.AccessPaths | Where-Object {$_ -notlike '\\?\Volume{*'}) -join ', ');
            [pscustomobject]@{id=$id;parent=('disk:'+$disk.Number);size=[uint64]$partition.Size;filesystem=[string]$volume.FileSystemType;label=[string]$volume.FileSystemLabel;mount=$mount;hibernation=$null}
        } } catch { $warnings.Add(('Disk '+$disk.Number+': '+$_.Exception.Message)) }
    }); [pscustomobject]@{items=$items;warnings=@($warnings.ToArray())} | ConvertTo-Json -Depth 5 -Compress"#;
    #[derive(serde::Deserialize)]
    struct Report {
        items: Vec<Disk>,
        warnings: Vec<String>,
    }
    let report: Report = serde_json::from_str(&super::windows::powershell(script)?)?;
    Ok(Inventory {
        items: report.items,
        warnings: report.warnings,
    })
}

#[cfg(not(windows))]
fn enumerate() -> Result<Inventory<Disk>> {
    use anyhow::ensure;
    let program =
        super::process::find("lsblk").context("Disk inventory requires lsblk from util-linux")?;
    let output = super::process::run(
        &program,
        &[
            "--json",
            "--bytes",
            "--paths",
            "--output",
            "NAME,PKNAME,SIZE,FSTYPE,LABEL,MOUNTPOINT",
        ],
        std::time::Duration::from_secs(10),
    )?;
    if output.code == 0
        && let Ok(items) = crate::disk::lsblk_json(&output.stdout)
    {
        return Ok(Inventory {
            items,
            warnings: vec![],
        });
    }
    let fallback = super::process::run(
        &program,
        &[
            "--pairs",
            "--bytes",
            "--paths",
            "--output",
            "NAME,PKNAME,SIZE,FSTYPE,LABEL,MOUNTPOINT",
        ],
        std::time::Duration::from_secs(10),
    )?;
    ensure!(
        fallback.code == 0,
        "lsblk supports neither the required JSON nor key/value inventory: {}",
        fallback.stderr
    );
    Ok(Inventory {
        items: crate::disk::lsblk_pairs(&fallback.stdout)?,
        warnings: vec![],
    })
}
