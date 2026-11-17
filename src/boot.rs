use anyhow::{Result, bail, ensure};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Selection {
    Firmware(u16),
    SystemdBoot(String),
    Grub { config: String, entry: String },
    Bcd(String),
    OfflineBcd { store: String, entry: String },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Target {
    pub id: String,
    pub label: String,
    pub current: bool,
    pub location: String,
    pub selection: Selection,
    pub unavailable: Option<String>,
    /// Detect replacement of an entry between discovery and shutdown.
    pub fingerprint: String,
}

#[derive(Clone, Debug, Default)]
pub struct Inventory<T> {
    pub items: Vec<T>,
    pub warnings: Vec<String>,
}

pub fn resolve(inventory: Inventory<Target>, id: &str) -> Result<Target> {
    let mut matches = inventory.items.into_iter().filter(|item| item.id == id);
    let Some(target) = matches.next() else {
        bail!("Target {id:?} is unavailable. Run 'boothop systems' again.");
    };
    ensure!(matches.next().is_none(), "Target ID {id:?} is ambiguous");
    if let Some(reason) = &target.unavailable {
        bail!("Cannot select {}: {reason}", target.label);
    }
    Ok(target)
}

/// Parse the UEFI load option without trusting device-path offsets.
pub fn firmware_target(number: u16, data: &[u8], current: Option<u16>) -> Result<Target> {
    ensure!(data.len() >= 8, "Truncated EFI load option");
    let path_size = u16::from_le_bytes([data[4], data[5]]) as usize;
    let mut label = Vec::new();
    let mut offset = 6;
    loop {
        ensure!(offset + 2 <= data.len(), "Unterminated EFI description");
        let character = u16::from_le_bytes([data[offset], data[offset + 1]]);
        offset += 2;
        if character == 0 {
            break;
        }
        label.push(character);
    }
    ensure!(
        offset + path_size <= data.len(),
        "Truncated EFI device path"
    );
    Ok(Target {
        id: format!("uefi:{number:04X}"),
        label: String::from_utf16(&label)?,
        current: current == Some(number),
        location: format!("Boot{number:04X}"),
        selection: Selection::Firmware(number),
        unavailable: None,
        fingerprint: hex_id(data),
    })
}

pub fn loader_entry(id: &str, contents: &str, location: &str) -> Result<Target> {
    ensure!(
        !id.is_empty() && !id.contains(['/', '\\', '\0']),
        "Invalid loader entry ID"
    );
    let mut title = None;
    let mut executable = false;
    for line in contents
        .lines()
        .map(str::trim)
        .filter(|line| !line.starts_with('#'))
    {
        let Some((key, value)) = line.split_once(char::is_whitespace) else {
            continue;
        };
        match key {
            "title" => title = Some(value.trim().to_owned()),
            "linux" | "efi" => executable = !value.trim().is_empty(),
            _ => {}
        }
    }
    ensure!(executable, "Loader entry has no linux or efi executable");
    Ok(Target {
        id: format!("systemd:{}:{id}", hex_id(location.as_bytes())),
        label: title.unwrap_or_else(|| id.into()),
        current: false,
        location: location.into(),
        selection: Selection::SystemdBoot(id.into()),
        unavailable: None,
        fingerprint: hex_id(contents.as_bytes()),
    })
}

/// Read UKI metadata as PE data; never load or execute the image.
pub fn unified_entry(id: &str, bytes: &[u8], location: &str) -> Result<Target> {
    use anyhow::Context;
    let word = |offset: usize| -> Result<u16> {
        let data = bytes.get(offset..offset + 2).context("Truncated PE word")?;
        Ok(u16::from_le_bytes([data[0], data[1]]))
    };
    let dword = |offset: usize| -> Result<usize> {
        let data = bytes
            .get(offset..offset + 4)
            .context("Truncated PE offset")?;
        Ok(u32::from_le_bytes([data[0], data[1], data[2], data[3]]) as usize)
    };
    ensure!(bytes.starts_with(b"MZ"), "Invalid UKI DOS header");
    let pe = dword(0x3c)?;
    ensure!(
        bytes.get(pe..pe + 4) == Some(b"PE\0\0"),
        "Invalid UKI PE header"
    );
    ensure!(word(pe + 4)? == 0x8664, "UKI is not an x86_64 image");
    let optional = pe + 24;
    ensure!(
        word(optional)? == 0x20b && word(optional + 68)? == 10,
        "UKI is not an EFI application"
    );
    let sections = optional + usize::from(word(pe + 20)?);
    let mut os_release = None;
    let mut linux = false;
    for index in 0..usize::from(word(pe + 6)?) {
        let start = sections + index * 40;
        let header = bytes
            .get(start..start + 40)
            .context("Truncated UKI section")?;
        let name = std::str::from_utf8(&header[..8])?.trim_end_matches('\0');
        let size = dword(start + 16)?;
        let offset = dword(start + 20)?;
        let section = bytes
            .get(
                offset
                    ..offset
                        .checked_add(size)
                        .context("Invalid UKI section size")?,
            )
            .context("UKI section is outside the image")?;
        if name == ".osrel" {
            ensure!(
                os_release.is_none() && size <= 65536,
                "Invalid UKI OS metadata"
            );
            os_release = Some(std::str::from_utf8(section)?.trim_end_matches('\0'));
        } else if name == ".linux" {
            linux = !section.is_empty();
        }
    }
    ensure!(linux, "UKI has no Linux kernel section");
    let release = os_release.context("UKI has no OS metadata")?;
    let title = release
        .lines()
        .find_map(|line| {
            line.strip_prefix("PRETTY_NAME=")
                .or_else(|| line.strip_prefix("NAME="))
        })
        .map(shell_words::split)
        .transpose()?
        .and_then(|words| (words.len() == 1).then(|| words[0].clone()))
        .unwrap_or_else(|| id.into());
    let mut target = loader_entry(id, "efi /image.efi", location)?;
    target.label = title;
    target.fingerprint = hex_id(bytes);
    Ok(target)
}

/// A duplicated entry name is insufficient evidence for the current OS.
pub fn mark_current_loader(
    targets: &mut [Target],
    selected: Option<&str>,
    current_loader: Option<u16>,
    loaders: &[(String, u16)],
) {
    let matches = targets
        .iter()
        .enumerate()
        .filter(|(_, target)| {
            matches!(&target.selection, Selection::SystemdBoot(id) if Some(id.as_str()) == selected)
                && loaders
                    .iter()
                    .any(|(id, number)| id == &target.id && Some(*number) == current_loader)
        })
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    if let [index] = matches.as_slice() {
        targets[*index].current = true;
    }
}

/// Tokenize menu headers only. GRUB configuration is never evaluated as shell code.
pub fn grub_entries(config: &str, path: &str) -> Inventory<Target> {
    let mut result = Inventory {
        items: Vec::new(),
        warnings: Vec::new(),
    };
    let mut depth = 0_i32;
    let mut menus: Vec<(i32, String)> = Vec::new();
    let mut unresolved_submenu = None;
    for line in config.lines() {
        let line = line.trim();
        if line.starts_with('#') {
            continue;
        }
        if let Some(start) = unresolved_submenu {
            depth += brace_delta(line);
            if depth <= start {
                unresolved_submenu = None;
            }
            continue;
        }
        let first = line.split_whitespace().next();
        let header = matches!(first, Some("menuentry" | "submenu"));
        if header {
            let words = match shell_words::split(line.trim_end_matches('{').trim()) {
                Ok(words) => words,
                Err(error) => {
                    result
                        .warnings
                        .push(format!("{path}: invalid menu header: {error}"));
                    if first == Some("submenu") {
                        unresolved_submenu = Some(depth);
                    }
                    depth += brace_delta(line);
                    continue;
                }
            };
            let title = words.get(1).cloned().unwrap_or_default();
            let id = words.iter().enumerate().find_map(|(index, word)| {
                if word == "--id" || word == "$menuentry_id_option" {
                    words.get(index + 1).cloned()
                } else {
                    word.strip_prefix("--id=").map(str::to_owned)
                }
            });
            let Some(id) = id.filter(|id| !id.is_empty() && !id.contains(['$', '`', '>', '\0']))
            else {
                result
                    .warnings
                    .push(format!("{path}: menu {title:?} has no stable literal ID"));
                if words.first().is_some_and(|word| word == "submenu") {
                    unresolved_submenu = Some(depth);
                }
                depth += brace_delta(line);
                continue;
            };
            if words.first().is_some_and(|word| word == "submenu") {
                menus.push((depth, id));
            } else {
                let entry = menus
                    .iter()
                    .map(|(_, id)| id.as_str())
                    .chain(std::iter::once(id.as_str()))
                    .collect::<Vec<_>>()
                    .join(">");
                let source = hex_id(path.as_bytes());
                result.items.push(Target {
                    id: format!("grub:{source}:{entry}"),
                    label: title,
                    current: false,
                    location: path.into(),
                    selection: Selection::Grub {
                        config: path.into(),
                        entry,
                    },
                    unavailable: None,
                    fingerprint: hex_id(config.as_bytes()),
                });
            }
        }
        depth += brace_delta(line);
        while menus.last().is_some_and(|(start, _)| depth <= *start) {
            menus.pop();
        }
    }
    result
}

fn brace_delta(line: &str) -> i32 {
    // Braces inside quoted strings and ${variables} do not change menu nesting.
    let mut quote = None;
    let mut escaped = false;
    let mut variable = false;
    let mut previous = '\0';
    let mut delta = 0;
    for character in line.chars() {
        if escaped {
            escaped = false;
            continue;
        }
        if character == '\\' {
            escaped = true;
            continue;
        }
        if Some(character) == quote {
            quote = None;
        } else if quote.is_none() && matches!(character, '\'' | '"') {
            quote = Some(character);
        } else if quote.is_none() {
            match character {
                '#' => break,
                '{' if previous == '$' => variable = true,
                '}' if variable => variable = false,
                '{' => delta += 1,
                '}' => delta -= 1,
                _ => {}
            }
        }
        previous = character;
    }
    delta
}

pub fn hex_id(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes)[..8]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[derive(Clone, Debug)]
pub struct FirmwarePath {
    pub partition: Option<uuid::Uuid>,
    pub files: Vec<String>,
}

pub fn firmware_path(bytes: &[u8]) -> Result<FirmwarePath> {
    use anyhow::Context;
    ensure!(bytes.len() >= 8, "Truncated EFI load option");
    let size = u16::from_le_bytes([bytes[4], bytes[5]]) as usize;
    let mut offset = 6;
    loop {
        ensure!(offset + 2 <= bytes.len(), "Truncated EFI description");
        let end = bytes[offset..offset + 2] == [0, 0];
        offset += 2;
        if end {
            break;
        }
    }
    let end = offset.checked_add(size).context("EFI path overflow")?;
    ensure!(end <= bytes.len(), "Truncated EFI path");
    let mut result = FirmwarePath {
        partition: None,
        files: vec![],
    };
    while offset < end {
        ensure!(offset + 4 <= end, "Truncated EFI node header");
        let length = u16::from_le_bytes([bytes[offset + 2], bytes[offset + 3]]) as usize;
        ensure!(
            length >= 4 && offset + length <= end,
            "Invalid EFI path node"
        );
        let node = &bytes[offset..offset + length];
        if node[0..2] == [4, 1] && length == 42 && node[41] == 2 {
            let guid = uuid::Uuid::from_bytes_le(node[24..40].try_into()?);
            ensure!(
                result.partition.replace(guid).is_none(),
                "Ambiguous EFI partition path"
            );
        }
        if node[0..2] == [4, 4] {
            ensure!((length - 4).is_multiple_of(2), "Invalid EFI filename");
            let words = node[4..]
                .as_chunks::<2>()
                .0
                .iter()
                .map(|pair| u16::from_le_bytes(*pair))
                .take_while(|word| *word != 0)
                .collect::<Vec<_>>();
            result.files.push(
                String::from_utf16(&words)?
                    .to_lowercase()
                    .replace('/', "\\"),
            );
        }
        offset += length;
    }
    Ok(result)
}

/// Recognize a literal GRUB redirect without evaluating its configuration.
pub fn grub_redirect_matches(config: &str, filesystem_uuid: &str, relative_config: &str) -> bool {
    let mut searched = false;
    let mut prefix = None;
    let mut redirected = false;
    for line in config
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
    {
        let Ok(words) = shell_words::split(line) else {
            return false;
        };
        let Some(command) = words.first() else {
            continue;
        };
        match command.as_str() {
            "search.fs_uuid"
                if !searched
                    && words
                        .get(1)
                        .is_some_and(|v| v.eq_ignore_ascii_case(filesystem_uuid))
                    && words.get(2).is_some_and(|v| v == "root") =>
            {
                searched = true
            }
            "search"
                if !searched
                    && words.iter().any(|v| v == "--fs-uuid")
                    && words.iter().any(|v| v == "--set=root")
                    && words
                        .last()
                        .is_some_and(|v| v.eq_ignore_ascii_case(filesystem_uuid)) =>
            {
                searched = true
            }
            "set" if searched && prefix.is_none() && words.len() == 2 => {
                let Some(path) = words[1]
                    .strip_prefix("prefix=($root)")
                    .or_else(|| words[1].strip_prefix("prefix=(${root})"))
                else {
                    return false;
                };
                prefix = Some(format!("{}/grub.cfg", path.trim_end_matches('/')));
            }
            "configfile"
                if searched
                    && !redirected
                    && words.len() == 2
                    && ["$prefix/grub.cfg", "${prefix}/grub.cfg"].contains(&words[1].as_str()) =>
            {
                redirected = true
            }
            _ => return false,
        }
    }
    searched && redirected && prefix.as_deref() == Some(relative_config)
}

pub fn grub_consumes_one_shot(config: &str) -> bool {
    let lines = config
        .lines()
        .map(str::trim)
        .filter(|line| !line.starts_with('#'))
        .collect::<Vec<_>>();
    lines.contains(&"load_env")
        && lines.contains(&"save_env next_entry")
        && lines.iter().any(|line| {
            [
                "set default=\"${next_entry}\"",
                "set default=\"$next_entry\"",
                "set default=${next_entry}",
                "set default=$next_entry",
            ]
            .contains(line)
        })
        && lines.iter().any(|line| {
            [
                "set next_entry=",
                "set next_entry=\"\"",
                "set next_entry=''",
            ]
            .contains(line)
        })
}
