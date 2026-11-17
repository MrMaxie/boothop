//! Read boot files from Linux partitions without mounting or modifying them.
mod device;
mod readers;
pub use device::Partition;

use crate::boot::{self, Inventory, Target};
use anyhow::{Result, ensure};
use std::io::{Read, Seek, SeekFrom};

const MAX_TEXT: usize = 4 * 1024 * 1024;
const MAX_UKI: usize = 128 * 1024 * 1024;
const MAX_ENTRIES: usize = 4096;

trait Filesystem {
    fn identity(&self) -> String;
    fn views(&self) -> Vec<u64> {
        vec![0]
    }
    fn read(&mut self, view: u64, path: &str, limit: usize) -> Result<Option<Vec<u8>>>;
    fn list(&mut self, view: u64, path: &str) -> Result<Option<Vec<String>>>;
}

/// `None` means no supported filesystem signature was found. Recognized but
/// unreadable filesystems return errors, never an empty successful inventory.
pub fn discover<R: Read + Seek + Send + 'static>(
    mut partition: Partition<R>,
    partition_id: &str,
) -> Result<Option<Inventory<Target>>> {
    let size = partition.seek(SeekFrom::End(0))?;
    partition.seek(SeekFrom::Start(0))?;
    let mut header = vec![0; size.min(128 * 1024) as usize];
    partition.read_exact(&mut header)?;
    partition.seek(SeekFrom::Start(0))?;
    let mut fs: Box<dyn Filesystem> = if header.get(1080..1082) == Some(&[0x53, 0xef]) {
        Box::new(readers::Ext(ext4_view::Ext4::load(Box::new(
            device::ExtSource(partition),
        ))?))
    } else if header.get(0..4) == Some(b"XFSB") {
        Box::new(readers::Xfs::open(partition)?)
    } else if header.get(65600..65608) == Some(b"_BHRfS_M") {
        Box::new(readers::Btrfs::open(partition)?)
    } else {
        return Ok(None);
    };
    let source = format!(
        "raw:{}:{}",
        boot::hex_id(partition_id.as_bytes()),
        fs.identity()
    );
    let mut inventory = Inventory {
        items: Vec::new(),
        warnings: Vec::new(),
    };
    for view in fs.views() {
        scan(&mut *fs, view, &format!("{source}:{view}"), &mut inventory);
    }
    Ok(Some(inventory))
}

fn scan(fs: &mut dyn Filesystem, view: u64, source: &str, out: &mut Inventory<Target>) {
    for path in [
        "/grub/grub.cfg",
        "/grub2/grub.cfg",
        "/boot/grub/grub.cfg",
        "/boot/grub2/grub.cfg",
    ] {
        let location = format!("{source}{path}");
        let result = (|| -> Result<()> {
            if let Some(bytes) = fs.read(view, path, MAX_TEXT)? {
                let mut parsed = boot::grub_entries(std::str::from_utf8(&bytes)?, &location);
                out.items.append(&mut parsed.items);
                out.warnings.append(&mut parsed.warnings);
            }
            Ok(())
        })();
        if let Err(error) = result {
            out.warnings.push(format!("{location}: {error:#}"));
        }
    }
    for (directory, extension) in [
        ("/loader/entries", ".conf"),
        ("/boot/loader/entries", ".conf"),
        ("/EFI/Linux", ".efi"),
        ("/boot/EFI/Linux", ".efi"),
    ] {
        let entries = match fs.list(view, directory) {
            Ok(Some(entries)) => entries,
            Ok(None) => continue,
            Err(error) => {
                out.warnings.push(format!("{source}{directory}: {error:#}"));
                continue;
            }
        };
        for name in entries {
            if name.starts_with('.') || !name.to_ascii_lowercase().ends_with(extension) {
                continue;
            }
            let path = format!("{directory}/{name}");
            let location = format!("{source}{path}");
            let result = (|| -> Result<Target> {
                ensure!(!name.contains(['/', '\\', '\0']), "Invalid boot filename");
                let bytes = fs
                    .read(
                        view,
                        &path,
                        if extension == ".efi" {
                            MAX_UKI
                        } else {
                            MAX_TEXT
                        },
                    )?
                    .ok_or_else(|| anyhow::anyhow!("Boot file disappeared during inspection"))?;
                if extension == ".efi" {
                    boot::unified_entry(&name, &bytes, &location)
                } else {
                    boot::loader_entry(&name, std::str::from_utf8(&bytes)?, &location)
                }
            })();
            match result {
                Ok(target) => out.items.push(target),
                Err(error) => out.warnings.push(format!("{location}: {error:#}")),
            }
        }
    }
    // Reading a GRUB menu is not permission to modify its filesystem, and a
    // BLS file alone does not prove that firmware will load that partition.
    for target in out
        .items
        .iter_mut()
        .filter(|target| target.location.starts_with(source))
    {
        target.unavailable.get_or_insert_with(|| "This entry was read from a Linux partition. Select its UEFI loader to open the boot menu; direct entry selection requires a writable, verified bootloader path.".into());
    }
}

fn bounded(mut reader: impl Read, limit: usize) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    reader
        .by_ref()
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() <= limit,
        "Boot file exceeds the {limit}-byte inspection limit"
    );
    Ok(bytes)
}

#[cfg(test)]
mod tests;
