use crate::boot::{Inventory, Selection, Target, hex_id};
use anyhow::{Context, Result, ensure};
use regf_rs::{Hive, RegValue};

pub fn bcd_targets(bytes: &[u8], path: &str) -> Result<Inventory<Target>> {
    let hive = Hive::from_bytes(bytes.to_vec())?;
    ensure!(!hive.is_dirty(), "BCD has pending transaction logs");
    let mut result = Inventory {
        items: vec![],
        warnings: vec![],
    };
    for id in hive.list_subkeys("Objects")? {
        let description = format!("Objects\\{id}\\Description");
        let values = match hive.list_values(&description) {
            Ok(values) => values,
            Err(error) => {
                result.warnings.push(format!("BCD object {id}: {error}"));
                continue;
            }
        };
        if !values
            .iter()
            .any(|(key, value)| key == "Type" && *value == RegValue::Dword(0x10200003))
        {
            continue;
        }
        uuid::Uuid::parse_str(&id)?;
        let elements = format!("Objects\\{id}\\Elements\\12000004");
        let label = hive
            .list_values(&elements)
            .ok()
            .and_then(|values| values.into_iter().find(|(name, _)| name == "Element"))
            .map(|(_, value)| value.to_bytes())
            .and_then(|bytes| decode_utf16(&bytes).ok())
            .unwrap_or_else(|| "Windows".into());
        result.items.push(Target {
            id: format!("bcd:{}:{id}", hex_id(path.as_bytes())),
            label,
            current: false,
            location: path.into(),
            selection: Selection::OfflineBcd {
                store: path.into(),
                entry: id,
            },
            unavailable: None,
            fingerprint: hex_id(bytes),
        });
    }
    Ok(result)
}

pub fn select_bcd(bytes: &[u8], entry: &str) -> Result<Vec<u8>> {
    let guid = uuid::Uuid::parse_str(entry)?;
    let mut hive = Hive::from_bytes(bytes.to_vec())?;
    ensure!(!hive.is_dirty(), "BCD has pending transaction logs");
    hive.open(&format!("Objects\\{entry}"))?;
    let element = "Objects\\{9dea862c-5cdd-4e70-acc1-f32b344d4795}\\Elements\\24000002";
    hive.open("Objects\\{9dea862c-5cdd-4e70-acc1-f32b344d4795}")?;
    hive.create_key(element)?;
    hive.set_value(
        element,
        "Element",
        RegValue::Binary(guid.to_bytes_le().to_vec()),
    )?;
    Ok(hive.to_bytes())
}

pub fn select_grub(bytes: &[u8], entry: &str) -> Result<Vec<u8>> {
    ensure!(
        !entry.is_empty() && !entry.contains(['\n', '\r', '\0']),
        "Invalid GRUB selection"
    );
    let text = std::str::from_utf8(bytes)?;
    ensure!(
        text.starts_with("# GRUB Environment Block\n") && bytes.len() >= 1024,
        "Unsupported GRUB environment format"
    );
    let mut output = String::from("# GRUB Environment Block\n");
    for line in text.lines().skip(1) {
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        let (key, _) = line
            .split_once('=')
            .context("Invalid GRUB environment variable")?;
        if key != "next_entry" {
            output.push_str(line);
            output.push('\n');
        }
    }
    output.push_str(&format!("next_entry={entry}\n"));
    ensure!(
        output.len() <= bytes.len(),
        "GRUB environment has insufficient space"
    );
    let mut output = output.into_bytes();
    output.resize(bytes.len(), b'#');
    Ok(output)
}

fn decode_utf16(bytes: &[u8]) -> Result<String> {
    ensure!(bytes.len().is_multiple_of(2), "Invalid BCD string");
    Ok(String::from_utf16(
        &bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .take_while(|value| *value != 0)
            .collect::<Vec<_>>(),
    )?)
}
