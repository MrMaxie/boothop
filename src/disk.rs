use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Hibernation {
    Detected,
    NotDetected,
    Undetermined(String),
}

impl std::fmt::Display for Hibernation {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Detected => write!(formatter, "hibernated"),
            Self::NotDetected => write!(formatter, "hibernation not detected"),
            Self::Undetermined(reason) => write!(formatter, "undetermined: {reason}"),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Disk {
    pub id: String,
    pub parent: Option<String>,
    pub size: u64,
    pub filesystem: String,
    pub label: String,
    pub mount: String,
    pub hibernation: Option<Hibernation>,
}

pub fn inspect_inventory(
    mut inventory: crate::boot::Inventory<Disk>,
    check: bool,
    mut probe: impl FnMut(&str) -> anyhow::Result<Hibernation>,
) -> crate::boot::Inventory<Disk> {
    for item in &mut inventory.items {
        if check && item.filesystem.eq_ignore_ascii_case("ntfs") {
            item.hibernation = Some(if !item.mount.is_empty() {
                Hibernation::Undetermined(
                    "Volume is mounted and may change during inspection".into(),
                )
            } else {
                probe(&item.id)
                    .unwrap_or_else(|error| Hibernation::Undetermined(format!("{error:#}")))
            });
        }
    }
    inventory
}

pub fn hibernation_header(header: &[u8]) -> Hibernation {
    if header.len() < 4096 {
        return Hibernation::Undetermined("Incomplete hibernation header".into());
    }
    if &header[..4] == b"hibr" || &header[..4] == b"HIBR" {
        Hibernation::Detected
    } else if header[..4096].iter().all(|byte| *byte == 0) {
        Hibernation::NotDetected
    } else {
        Hibernation::Undetermined("Unrecognized hibernation header".into())
    }
}

pub fn mount_points(mountinfo: &str) -> Vec<String> {
    mountinfo
        .lines()
        .filter_map(|line| {
            let (fields, filesystem) = line.split_once(" - ")?;
            if !matches!(
                filesystem.split_whitespace().next()?,
                "vfat" | "ext2" | "ext3" | "ext4" | "btrfs" | "xfs" | "ntfs" | "ntfs3" | "fuseblk"
            ) {
                return None;
            }
            let path = fields.split_whitespace().nth(4)?;
            Some(
                path.replace("\\040", " ")
                    .replace("\\011", "\t")
                    .replace("\\012", "\n")
                    .replace("\\134", "\\"),
            )
        })
        .collect()
}

/// Inspect only the supplied read/seek source; no mount or write interface is available.
pub fn inspect_ntfs<T: std::io::Read + std::io::Seek>(
    mut reader: T,
) -> anyhow::Result<Hibernation> {
    use anyhow::Context;
    use ntfs::{Ntfs, NtfsReadSeek, indexes::NtfsFileNameIndex};
    let mut sector = [0; 512];
    reader.seek(std::io::SeekFrom::Start(0))?;
    reader.read_exact(&mut sector)?;
    anyhow::ensure!(
        &sector[3..11] == b"NTFS    " && sector[510..512] == [0x55, 0xaa],
        "Invalid NTFS boot sector"
    );
    reader.seek(std::io::SeekFrom::Start(0))?;
    let mut ntfs = Ntfs::new(&mut reader)?;
    let dirty = ntfs
        .volume_info(&mut reader)?
        .flags()
        .contains(ntfs::structured_values::NtfsVolumeFlags::IS_DIRTY);
    ntfs.read_upcase_table(&mut reader)?;
    let root = ntfs.root_directory(&mut reader)?;
    let index = root.directory_index(&mut reader)?;
    let mut finder = index.finder();
    let Some(entry) = NtfsFileNameIndex::find(&mut finder, &ntfs, &mut reader, "hiberfil.sys")
    else {
        return Ok(ntfs_evidence(dirty, None));
    };
    let file = entry?.to_file(&ntfs, &mut reader)?;
    let data = file
        .data(&mut reader, "")
        .context("Hibernation file has no data")??;
    let attribute = data.to_attribute()?;
    let mut value = attribute.value(&mut reader)?;
    let mut bytes = vec![0_u8; 4096];
    let mut length = 0;
    while length < bytes.len() {
        let count = value.read(&mut reader, &mut bytes[length..])?;
        if count == 0 {
            break;
        }
        length += count;
    }
    Ok(ntfs_evidence(dirty, Some(&bytes[..length])))
}

pub fn ntfs_evidence(dirty: bool, header: Option<&[u8]>) -> Hibernation {
    let state = header
        .map(hibernation_header)
        .unwrap_or(Hibernation::NotDetected);
    if dirty && state != Hibernation::Detected {
        Hibernation::Undetermined(
            "NTFS volume is dirty; negative hibernation evidence is inconclusive".into(),
        )
    } else {
        state
    }
}

pub fn lsblk_json(text: &str) -> anyhow::Result<Vec<Disk>> {
    use anyhow::Context;
    let document: serde_json::Value = serde_json::from_str(text)?;
    let mut items = Vec::new();
    fn append(values: &[serde_json::Value], items: &mut Vec<Disk>) -> anyhow::Result<()> {
        for value in values {
            items.push(Disk {
                id: value["name"]
                    .as_str()
                    .filter(|name| name.starts_with('/'))
                    .context("Missing absolute disk path")?
                    .into(),
                parent: value["pkname"]
                    .as_str()
                    .filter(|name| !name.is_empty())
                    .map(str::to_owned),
                size: value["size"]
                    .as_u64()
                    .or_else(|| value["size"].as_str().and_then(|s| s.parse().ok()))
                    .context("Invalid disk size")?,
                filesystem: value["fstype"].as_str().unwrap_or_default().into(),
                label: value["label"].as_str().unwrap_or_default().into(),
                mount: value["mountpoint"].as_str().unwrap_or_default().into(),
                hibernation: None,
            });
            if let Some(children) = value["children"].as_array() {
                append(children, items)?;
            }
        }
        Ok(())
    }
    append(
        document["blockdevices"]
            .as_array()
            .context("Invalid lsblk response")?,
        &mut items,
    )?;
    Ok(items)
}

pub fn lsblk_pairs(text: &str) -> anyhow::Result<Vec<Disk>> {
    use anyhow::Context;
    let mut values = Vec::new();
    for line in text.lines().filter(|line| !line.is_empty()) {
        let mut object = serde_json::Map::new();
        for pair in shell_words::split(line)? {
            let (key, value) = pair.split_once('=').context("Invalid disk field")?;
            let mut bytes = Vec::new();
            let mut input = value.as_bytes();
            while !input.is_empty() {
                if input.starts_with(b"\\x") && input.len() >= 4 {
                    bytes.push(u8::from_str_radix(std::str::from_utf8(&input[2..4])?, 16)?);
                    input = &input[4..];
                } else {
                    bytes.push(input[0]);
                    input = &input[1..];
                }
            }
            object.insert(key.to_lowercase(), String::from_utf8(bytes)?.into());
        }
        values.push(serde_json::Value::Object(object));
    }
    lsblk_json(&serde_json::json!({"blockdevices": values}).to_string())
}
