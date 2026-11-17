use crate::{
    boot::{Inventory, Target},
    rawfs::{self, Partition},
};
use anyhow::{Context, Result};
use std::fs::File;

/// Only read handles are opened. No drive letter, mount point, filesystem
/// driver, repair operation, or raw write is involved in discovery.
pub fn discover() -> Result<Inventory<Target>> {
    let script = r#"
        $warnings=[Collections.Generic.List[string]]::new();
        $items=@(Get-Disk -ErrorAction Stop | ForEach-Object {
            $disk=$_;
            try { Get-Partition -DiskNumber $disk.Number -ErrorAction Stop | ForEach-Object {
                $p=$_; $v=$p | Get-Volume -ErrorAction SilentlyContinue;
                if ([string]$v.FileSystemType -notin @('NTFS','FAT','FAT32','exFAT','ReFS')) {
                    [pscustomobject]@{
                        disk=[uint32]$disk.Number; offset=[uint64]$p.Offset;
                        size=[uint64]$p.Size; sector=[uint32]$disk.LogicalSectorSize;
                        identity=([string]$disk.UniqueId+':'+[string]$p.Guid+':'+[string]$p.Offset)
                    }
                }
            }} catch { $warnings.Add(('Disk '+$disk.Number+': '+$_.Exception.Message)) }
        });
        [pscustomobject]@{items=$items;warnings=@($warnings.ToArray())} | ConvertTo-Json -Depth 4 -Compress
    "#;
    #[derive(serde::Deserialize)]
    struct Source {
        disk: u32,
        offset: u64,
        size: u64,
        sector: u32,
        identity: String,
    }
    #[derive(serde::Deserialize)]
    struct Report {
        items: Vec<Source>,
        warnings: Vec<String>,
    }
    let report: Report = serde_json::from_str(&super::windows::powershell(script)?)?;
    let mut out = Inventory {
        items: Vec::new(),
        warnings: report.warnings,
    };
    for source in report.items {
        let result = (|| -> Result<_> {
            let file = File::open(format!(r"\\.\PhysicalDrive{}", source.disk))
                .context("Cannot read partition. Run with administrator privileges")?;
            rawfs::discover(
                Partition::new(file, source.offset, source.size, source.sector)?,
                &source.identity,
            )
        })();
        match result {
            Ok(Some(mut entries)) => {
                out.items.append(&mut entries.items);
                out.warnings.append(&mut entries.warnings);
            }
            Ok(None) => {}
            Err(error) => out.warnings.push(format!(
                "Linux filesystem on disk {}, offset {}: {error:#}",
                source.disk, source.offset
            )),
        }
    }
    Ok(out)
}
