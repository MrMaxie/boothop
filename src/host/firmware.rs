use crate::boot::{Inventory, Target, firmware_target};
use anyhow::{Context, Result, ensure};

pub const EFI_GUID: &str = "8be4df61-93ca-11d2-aa0d-00e098032b8c";
pub const LOADER_GUID: &str = "4a67b082-0a4c-41cf-b6c7-440b29bb8c4f";

pub fn encode_text(text: &str) -> Vec<u8> {
    text.encode_utf16()
        .chain(Some(0))
        .flat_map(u16::to_le_bytes)
        .collect()
}
pub fn decode_text(bytes: &[u8]) -> Result<String> {
    ensure!(bytes.len().is_multiple_of(2), "Invalid firmware text");
    let words = bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .take_while(|word| *word != 0)
        .collect::<Vec<_>>();
    Ok(String::from_utf16(&words)?)
}

pub fn discover() -> Inventory<Target> {
    let mut inventory = Inventory {
        items: vec![],
        warnings: vec![],
    };
    let result = (|| -> Result<()> {
        let order = read("BootOrder", EFI_GUID)?.context("Firmware BootOrder unavailable")?;
        ensure!(order.len().is_multiple_of(2), "Invalid firmware BootOrder");
        let current = read("BootCurrent", EFI_GUID)?
            .filter(|value| value.len() == 2)
            .map(|value| u16::from_le_bytes([value[0], value[1]]));
        for pair in order.as_chunks::<2>().0.iter() {
            let number = u16::from_le_bytes([pair[0], pair[1]]);
            let name = format!("Boot{number:04X}");
            let result = read(&name, EFI_GUID).and_then(|bytes| {
                firmware_target(number, &bytes.context("Boot entry missing")?, current)
            });
            match result {
                Ok(target) => inventory.items.push(target),
                Err(error) => inventory.warnings.push(format!("{name}: {error:#}")),
            }
        }
        Ok(())
    })();
    if let Err(error) = result {
        inventory
            .warnings
            .push(format!("UEFI discovery: {error:#}"));
    }
    inventory
}

pub fn loader_candidates(loader: &str) -> Result<Vec<(u16, crate::boot::FirmwarePath)>> {
    let order = read("BootOrder", EFI_GUID)?.context("UEFI BootOrder unavailable")?;
    ensure!(order.len().is_multiple_of(2), "Invalid firmware BootOrder");
    let mut candidates = Vec::new();
    for pair in order.as_chunks::<2>().0.iter() {
        let number = u16::from_le_bytes(*pair);
        let Some(data) = read(&format!("Boot{number:04X}"), EFI_GUID)? else {
            continue;
        };
        let path = crate::boot::firmware_path(&data)?;
        let matching = path.files.iter().any(|file| match loader {
            "windows" => file.ends_with("\\microsoft\\boot\\bootmgfw.efi"),
            "systemd" => file.ends_with("\\systemd-bootx64.efi"),
            "grub" => file.ends_with("\\grubx64.efi") || file.ends_with("\\shimx64.efi"),
            _ => false,
        });
        if matching {
            candidates.push((number, path));
        }
    }
    Ok(candidates)
}

pub fn preflight() -> Result<()> {
    #[cfg(windows)]
    super::windows::require_privilege("SeSystemEnvironmentPrivilege")?;
    read("BootOrder", EFI_GUID)?.context("UEFI runtime variables unavailable")?;
    Ok(())
}

#[cfg(windows)]
pub fn read(name: &str, guid: &str) -> Result<Option<Vec<u8>>> {
    super::windows::firmware_read(name, guid)
}
#[cfg(windows)]
pub fn write(name: &str, guid: &str, value: Option<&[u8]>) -> Result<()> {
    super::windows::firmware_write(name, guid, value)
}

#[cfg(not(windows))]
fn variable_path(name: &str, guid: &str) -> Result<std::path::PathBuf> {
    ensure!(
        name.bytes().all(|byte| byte.is_ascii_alphanumeric())
            && [EFI_GUID, LOADER_GUID].contains(&guid),
        "Invalid firmware variable"
    );
    Ok(std::path::Path::new("/sys/firmware/efi/efivars").join(format!("{name}-{guid}")))
}
#[cfg(not(windows))]
pub fn read(name: &str, guid: &str) -> Result<Option<Vec<u8>>> {
    match std::fs::read(variable_path(name, guid)?) {
        Ok(bytes) => {
            ensure!(bytes.len() >= 4, "Truncated EFI attributes");
            Ok(Some(bytes[4..].to_vec()))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}
#[cfg(not(windows))]
pub fn write(name: &str, guid: &str, value: Option<&[u8]>) -> Result<()> {
    use std::io::Write;
    let path = variable_path(name, guid)?;
    if let Some(value) = value {
        let attributes = std::fs::read(&path)
            .ok()
            .filter(|bytes| bytes.len() >= 4)
            .map(|bytes| bytes[..4].to_vec())
            .unwrap_or_else(|| 7_u32.to_le_bytes().to_vec());
        let mut data = attributes;
        data.extend_from_slice(value);
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(path)?;
        file.write_all(&data)?;
    } else if path.exists() {
        std::fs::remove_file(path)?;
    }
    Ok(())
}
