use crate::{
    boot::{self, Inventory, Selection, Target},
    cli::{self, System},
    disk::{Disk, Hibernation},
    reboot::{self, Change, Closure, RebootSystem, Request},
    update::{self, Installer, Platform, Response, Transport, UpdateStatus},
};
use anyhow::{Result, bail};
use std::{
    collections::{BTreeMap, VecDeque},
    ffi::OsString,
};

fn target() -> Target {
    Target {
        id: "uefi:0002".into(),
        label: "Linux".into(),
        current: false,
        location: "Boot0002".into(),
        selection: Selection::Firmware(2),
        unavailable: None,
        fingerprint: "fixture-2".into(),
    }
}

struct Simulated {
    targets: Vec<Target>,
    values: BTreeMap<String, Vec<u8>>,
    events: Vec<String>,
    responses: VecDeque<Response>,
    approve: bool,
    cancelled: bool,
    fail_shutdown: bool,
    fail_write: Option<String>,
    concurrent: bool,
    fail_preflight: bool,
    warnings: Vec<String>,
    installed: Option<Vec<u8>>,
    cancel_after_write: bool,
    uncertain_shutdown: bool,
    replace_target_after_wait: bool,
    application_grace: bool,
    prompts: Vec<String>,
    fail_recovery: bool,
}
impl Default for Simulated {
    fn default() -> Self {
        Self {
            targets: vec![target()],
            values: BTreeMap::new(),
            events: vec![],
            responses: VecDeque::new(),
            approve: true,
            cancelled: false,
            fail_shutdown: false,
            fail_write: None,
            concurrent: false,
            fail_preflight: false,
            warnings: vec![],
            installed: None,
            cancel_after_write: false,
            uncertain_shutdown: false,
            replace_target_after_wait: false,
            application_grace: false,
            prompts: vec![],
            fail_recovery: false,
        }
    }
}
impl RebootSystem for Simulated {
    fn application_grace_period(&self) -> bool {
        self.application_grace
    }
    fn cancelled(&self) -> bool {
        self.cancelled
    }
    fn discover(&mut self) -> Result<Inventory<Target>> {
        self.events.push("discover".into());
        Ok(Inventory {
            items: self.targets.clone(),
            warnings: self.warnings.clone(),
        })
    }
    fn preflight(&mut self, _: &Target, _: &Request) -> Result<()> {
        self.events.push("preflight".into());
        if self.fail_preflight {
            bail!("permission denied")
        }
        Ok(())
    }
    fn confirm(&mut self, message: &str) -> Result<bool> {
        self.prompts.push(message.into());
        self.events.push("confirm".into());
        Ok(self.approve)
    }
    fn wait(&mut self, seconds: u32) -> Result<bool> {
        self.events.push(format!("wait:{seconds}"));
        if self.replace_target_after_wait {
            self.targets[0].fingerprint = "changed-device".into();
        }
        Ok(!self.cancelled)
    }
    fn plan(&mut self, _: &Target) -> Result<Vec<Change>> {
        Ok(vec![
            Change {
                key: "BootNext".into(),
                before: self.values.get("BootNext").cloned(),
                after: vec![2, 0],
            },
            Change {
                key: "nested".into(),
                before: None,
                after: b"linux".to_vec(),
            },
        ])
    }
    fn read(&mut self, key: &str) -> Result<Option<Vec<u8>>> {
        Ok(self.values.get(key).cloned())
    }
    fn write(&mut self, key: &str, value: Option<&[u8]>) -> Result<()> {
        if self.cancel_after_write {
            self.cancelled = true;
        }
        self.events.push(format!("write:{key}"));
        if self.fail_recovery && value.is_none() {
            bail!("simulated recovery failure");
        }
        if let Some(bytes) = value {
            self.values.insert(key.into(), bytes.to_vec());
        } else {
            self.values.remove(key);
        }
        if self.fail_write.as_deref() == Some(key) {
            self.fail_write = None;
            bail!("write failed after changing data")
        }
        Ok(())
    }
    fn shutdown(&mut self, closure: &Closure) -> Result<()> {
        self.events.push(format!("shutdown:{closure:?}"));
        if self.uncertain_shutdown {
            return Err(reboot::ShutdownUncertain("transport lost".into()).into());
        }
        if self.concurrent {
            self.values.insert("BootNext".into(), vec![99]);
        }
        if self.fail_shutdown {
            bail!("restart rejected")
        }
        Ok(())
    }
}
impl Transport for Simulated {
    fn get(&mut self, url: &str, _: usize) -> Result<Response> {
        self.events.push(format!("get:{url}"));
        self.responses
            .pop_front()
            .ok_or_else(|| anyhow::anyhow!("offline"))
    }
}
impl Installer for Simulated {
    fn replace(&mut self, bytes: &[u8]) -> Result<String> {
        self.installed = Some(bytes.to_vec());
        Ok("Replaced simulated file".into())
    }
}
impl System for Simulated {
    fn platform(&self) -> Platform {
        Platform::Linux
    }
    fn disks(&mut self, check: bool) -> Result<Inventory<Disk>> {
        self.events.push(format!("disks:{check}"));
        Ok(Inventory {
            items: vec![],
            warnings: vec![],
        })
    }
    fn diagnose(&mut self, _: u8) -> Vec<String> {
        vec!["Local checks completed".into()]
    }
    fn help_warnings(&mut self) -> Vec<String> {
        self.warnings.clone()
    }
}
fn request() -> Request {
    Request {
        id: target().id,
        delay: 10,
        closure: Closure::Graceful,
        yes: false,
    }
}
fn run(args: &[&str], system: &mut Simulated) -> cli::Output {
    cli::run(args.iter().map(OsString::from), system)
}

#[test]
fn help_is_available_offline_without_operations() {
    let mut system = Simulated {
        warnings: vec!["Missing dependency".into()],
        ..Default::default()
    };
    for args in [
        vec!["boothop"],
        vec!["boothop", "--help"],
        vec!["boothop", "reboot", "--help"],
    ] {
        let result = run(&args, &mut system);
        assert_eq!(result.code, 0);
        assert!(result.stdout.contains("Usage:"));
        assert!(result.stderr.contains("Missing dependency"));
    }
    assert!(system.events.is_empty());
}
#[test]
fn invalid_flags_never_reach_system() {
    for args in [
        vec![
            "boothop",
            "reboot",
            "uefi:0002",
            "--force",
            "--force-after",
            "3",
        ],
        vec!["boothop", "reboot", "uefi:0002", "--force-after", "0"],
        vec!["boothop", "reboot", "uefi:0002", "--timeout", "-1"],
        vec!["boothop", "reboot", "uefi:0002", "--timeout", "4294967296"],
        vec![
            "boothop",
            "reboot",
            "uefi:0002",
            "--force-after",
            "4294967296",
        ],
        vec!["boothop", "--unknown"],
    ] {
        let mut system = Simulated::default();
        assert_eq!(run(&args, &mut system).code, 2);
        assert!(system.events.is_empty());
    }
}
#[test]
fn cancellation_preserves_boot_state() {
    for cancelled in [false, true] {
        let mut system = Simulated {
            approve: cancelled,
            cancelled,
            ..Default::default()
        };
        assert_eq!(
            reboot::execute(&mut system, &request()).unwrap(),
            "Restart cancelled."
        );
        assert!(system.values.is_empty());
        assert!(
            !system
                .events
                .iter()
                .any(|event| event.starts_with("shutdown"))
        );
    }
}
#[test]
fn preflight_failure_occurs_before_confirmation_or_mutation() {
    let mut system = Simulated {
        fail_preflight: true,
        ..Default::default()
    };
    assert!(reboot::execute(&mut system, &request()).is_err());
    assert_eq!(system.events, ["discover", "preflight"]);
}
#[test]
fn shutdown_failure_restores_prior_values() {
    let mut system = Simulated {
        fail_shutdown: true,
        ..Default::default()
    };
    system.values.insert("BootNext".into(), vec![1, 0]);
    let before = system.values.clone();
    assert!(
        reboot::execute(&mut system, &request())
            .unwrap_err()
            .to_string()
            .contains("restored")
    );
    assert_eq!(system.values, before);
}
#[test]
fn partial_write_failure_is_rolled_back() {
    for key in ["BootNext", "nested"] {
        let mut system = Simulated {
            fail_write: Some(key.into()),
            ..Default::default()
        };
        assert!(reboot::execute(&mut system, &request()).is_err());
        assert!(system.values.is_empty());
        assert!(
            !system
                .events
                .iter()
                .any(|event| event.starts_with("shutdown"))
        );
    }
}
#[test]
fn recovery_preserves_concurrent_changes() {
    let mut system = Simulated {
        fail_shutdown: true,
        concurrent: true,
        ..Default::default()
    };
    assert!(
        reboot::execute(&mut system, &request())
            .unwrap_err()
            .to_string()
            .contains("Recovery incomplete")
    );
    assert_eq!(system.values.get("BootNext"), Some(&vec![99]));
}
#[test]
fn yes_skips_confirmation_but_revalidates_and_delays() {
    let mut system = Simulated::default();
    let result = run(
        &[
            "boothop",
            "reboot",
            "uefi:0002",
            "--yes",
            "--timeout",
            "10",
            "--force-after",
            "60",
        ],
        &mut system,
    );
    assert_eq!(result.code, 0, "{}", result.stderr);
    assert!(!system.events.contains(&"confirm".into()));
    assert_eq!(
        system
            .events
            .iter()
            .filter(|event| *event == "preflight")
            .count(),
        2
    );
    assert!(system.events.contains(&"wait:10".into()));
    assert_eq!(system.events.last().unwrap(), "shutdown:ForceAfter(60)");
}
#[test]
fn stale_or_duplicate_ids_fail() {
    for targets in [vec![], vec![target(), target()]] {
        let mut system = Simulated {
            targets,
            ..Default::default()
        };
        assert!(reboot::execute(&mut system, &request()).is_err());
        assert!(system.values.is_empty());
    }
}
#[test]
fn grub_nested_ids_are_independent_of_labels() {
    let config = "submenu 'Advanced' --id advanced {\n menuentry 'Linux' --id first {\n linux /vmlinuz\n }\n menuentry 'Linux' --id second {\n }\n}\nmenuentry 'Other' --id other {\n}\n";
    let result = boot::grub_entries(config, "/boot/grub/grub.cfg");
    assert!(result.warnings.is_empty());
    assert_eq!(result.items.len(), 3);
    assert!(result.items[0].id.ends_with("advanced>first"));
    assert!(result.items[2].id.ends_with(":other"));
}
#[test]
fn malformed_firmware_data_is_rejected() {
    for data in [vec![], vec![0; 7], vec![0, 0, 0, 0, 255, 255, 0, 0]] {
        assert!(boot::firmware_target(1, &data, None).is_err());
    }
}
#[test]
fn hibernation_states_require_positive_evidence() {
    let mut data = vec![0; 4096];
    assert_eq!(
        crate::disk::hibernation_header(&data),
        Hibernation::NotDetected
    );
    data[..4].copy_from_slice(b"hibr");
    assert_eq!(
        crate::disk::hibernation_header(&data),
        Hibernation::Detected
    );
    data[..4].copy_from_slice(b"wake");
    assert!(matches!(
        crate::disk::hibernation_header(&data),
        Hibernation::Undetermined(_)
    ));
    assert!(matches!(
        crate::disk::hibernation_header(&[]),
        Hibernation::Undetermined(_)
    ));
}
#[test]
fn doctor_preserves_local_results_offline() {
    let result = run(&["boothop", "doctor"], &mut Simulated::default());
    assert!(result.stdout.contains("Local checks completed"));
    assert!(result.stderr.contains("offline"));
}
fn response(status: u16, body: &str) -> Response {
    Response {
        status,
        body: body.as_bytes().to_vec(),
    }
}
fn release(version: &str) -> Response {
    let asset = format!("boothop-v{version}-linux-x86_64.tar.gz");
    response(200, &serde_json::json!({"tag_name": format!("v{version}"), "draft": false, "prerelease": false, "assets": [
        {"name": asset, "browser_download_url": format!("https://github.com/MrMaxie/boothop/releases/download/v{version}/{asset}")},
        {"name": "SHA256SUMS", "browser_download_url": format!("https://github.com/MrMaxie/boothop/releases/download/v{version}/SHA256SUMS")}
    ]}).to_string())
}
#[test]
fn update_uses_semantic_version_order() {
    let mut http = Simulated::default();
    http.responses.push_back(release("0.10.0"));
    assert!(matches!(
        update::check(&mut http, Platform::Linux, "0.9.0").unwrap(),
        UpdateStatus::Available { .. }
    ));
    http.responses.push_back(release("0.9.0"));
    assert_eq!(
        update::check(&mut http, Platform::Linux, "0.10.0").unwrap(),
        UpdateStatus::Current
    );
}
#[test]
fn missing_repository_differs_from_no_release() {
    for (code, expected) in [(404, "inaccessible"), (200, "No published")] {
        let mut http = Simulated::default();
        http.responses
            .extend([response(404, "{}"), response(code, "{}")]);
        assert!(
            update::check(&mut http, Platform::Linux, "0.1.0")
                .unwrap()
                .to_string()
                .contains(expected)
        );
    }
}
#[test]
fn checksum_failure_never_calls_installer() {
    let mut http = Simulated::default();
    http.responses.push_back(release("0.2.0"));
    let status = update::check(&mut http, Platform::Linux, "0.1.0").unwrap();
    http.responses.extend([
        response(
            200,
            &format!("{}  boothop-v0.2.0-linux-x86_64.tar.gz\n", "0".repeat(64)),
        ),
        response(200, "bad bytes"),
    ]);
    let mut installer = Simulated::default();
    assert!(update::install(&mut http, &mut installer, Platform::Linux, &status).is_err());
    assert!(installer.installed.is_none());
}

#[test]
fn cancellation_after_a_write_restores_state_without_shutdown() {
    let mut system = Simulated {
        cancel_after_write: true,
        ..Default::default()
    };
    assert!(
        reboot::execute(&mut system, &request())
            .unwrap_err()
            .to_string()
            .contains("restored")
    );
    assert!(system.values.is_empty());
    assert!(!system.events.iter().any(|v| v.starts_with("shutdown")));
}

#[test]
fn unknown_shutdown_acceptance_does_not_rollback_a_possible_pending_boot() {
    let mut system = Simulated {
        uncertain_shutdown: true,
        ..Default::default()
    };
    assert!(
        reboot::execute(&mut system, &request())
            .unwrap_err()
            .to_string()
            .contains("retained")
    );
    assert_eq!(system.values.get("BootNext"), Some(&vec![2, 0]));
}

#[test]
fn changed_device_with_same_id_and_label_is_rejected_after_countdown() {
    let mut system = Simulated {
        replace_target_after_wait: true,
        ..Default::default()
    };
    assert!(reboot::execute(&mut system, &request()).is_err());
    assert!(system.values.is_empty());
}

#[test]
fn default_restart_confirmation_only_asks_about_the_target() {
    let mut system = Simulated {
        approve: false,
        ..Default::default()
    };
    let result = run(&["boothop", "reboot", "uefi:0002"], &mut system);
    assert_eq!(result.code, 0);
    assert_eq!(system.prompts, ["Restart into Linux?"]);
    assert_eq!(system.events, ["discover", "preflight", "confirm"]);
    assert!(system.values.is_empty());
}

#[test]
fn delayed_confirmation_makes_clear_the_timer_starts_after_approval() {
    let mut system = Simulated {
        approve: false,
        ..Default::default()
    };
    let result = run(
        &[
            "boothop",
            "reboot",
            "uefi:0002",
            "--timeout",
            "10",
            "--force",
        ],
        &mut system,
    );
    assert_eq!(result.code, 0);
    assert_eq!(
        system.prompts,
        [
            "Restart into Linux?\nRestart 10 seconds after confirmation.\nForce applications to close. Unsaved work can be lost."
        ]
    );
    assert_eq!(system.events, ["discover", "preflight", "confirm"]);
}

#[test]
fn windows_confirmation_describes_application_grace_period() {
    let mut system = Simulated {
        application_grace: true,
        approve: false,
        ..Default::default()
    };
    let request = Request {
        closure: Closure::ForceAfter(60),
        ..request()
    };
    reboot::execute(&mut system, &request).unwrap();
    assert!(system.prompts[0].contains("System services close at restart"));
    assert!(system.prompts[0].contains("60 seconds"));
}

#[test]
fn unresolved_grub_submenu_does_not_create_wrong_top_level_ids() {
    let result = boot::grub_entries(
        "submenu 'Unknown' {\nmenuentry 'Nested' --id nested {\n}\n}\nmenuentry 'Valid' --id valid {\n}\n",
        "/boot/grub/grub.cfg",
    );
    assert_eq!(result.items.len(), 1);
    assert!(result.items[0].id.ends_with(":valid"));
    assert!(!result.warnings.is_empty());
}

#[test]
fn loader_ids_distinguish_equal_names_on_different_partitions() {
    let first = boot::loader_entry(
        "linux.conf",
        "title Linux\nlinux /vmlinuz",
        "/efi/loader/entries/linux.conf",
    )
    .unwrap();
    let second = boot::loader_entry(
        "linux.conf",
        "title Linux\nlinux /vmlinuz",
        "/other/loader/entries/linux.conf",
    )
    .unwrap();
    assert_ne!(first.id, second.id);
}

#[test]
fn grub_selection_preserves_persistent_defaults_and_block_size() {
    let mut bytes =
        b"# GRUB Environment Block\nsaved_entry=normal\nnext_entry=old\nrecordfail=1\n".to_vec();
    bytes.resize(1024, b'#');
    let output = crate::bootstore::select_grub(&bytes, "advanced>linux").unwrap();
    let text = String::from_utf8(output.clone()).unwrap();
    assert_eq!(output.len(), bytes.len());
    assert!(text.contains("saved_entry=normal\n"));
    assert!(text.contains("next_entry=advanced>linux\n"));
    assert!(!text.contains("next_entry=old"));
    assert!(crate::bootstore::select_grub(&bytes, "bad\ninjected=value").is_err());
}

fn bcd_fixture() -> (Vec<u8>, String) {
    use regf_rs::{Hive, RegValue};
    let mut hive = Hive::new_empty("BCD00000000");
    let id = "{12345678-1234-5678-9abc-123456789abc}".to_owned();
    let desc = format!("Objects\\{id}\\Description");
    hive.create_key(&desc).unwrap();
    hive.set_value(&desc, "Type", RegValue::Dword(0x10200003))
        .unwrap();
    let label = format!("Objects\\{id}\\Elements\\12000004");
    hive.create_key(&label).unwrap();
    hive.set_value(
        &label,
        "Element",
        RegValue::Binary(
            "Windows\0"
                .encode_utf16()
                .flat_map(u16::to_le_bytes)
                .collect(),
        ),
    )
    .unwrap();
    let default = "Objects\\{9dea862c-5cdd-4e70-acc1-f32b344d4795}\\Elements\\23000003";
    hive.create_key(default).unwrap();
    hive.set_value(default, "Element", RegValue::Binary(vec![42; 16]))
        .unwrap();
    (hive.to_bytes(), id)
}

#[test]
fn offline_bcd_selects_one_shot_without_changing_default() {
    use regf_rs::{Hive, RegValue};
    let (bytes, id) = bcd_fixture();
    let found = crate::bootstore::bcd_targets(&bytes, "/efi/EFI/Microsoft/Boot/BCD").unwrap();
    assert_eq!(found.items[0].label, "Windows");
    let output = crate::bootstore::select_bcd(&bytes, &id).unwrap();
    let hive = Hive::from_bytes(output).unwrap();
    assert_eq!(
        hive.get_value(
            "Objects\\{9dea862c-5cdd-4e70-acc1-f32b344d4795}\\Elements\\23000003",
            "Element"
        )
        .unwrap(),
        RegValue::Binary(vec![42; 16])
    );
    assert_eq!(
        hive.get_value(
            "Objects\\{9dea862c-5cdd-4e70-acc1-f32b344d4795}\\Elements\\24000002",
            "Element"
        )
        .unwrap(),
        RegValue::Binary(uuid::Uuid::parse_str(&id).unwrap().to_bytes_le().to_vec())
    );
    assert!(
        crate::bootstore::select_bcd(&bytes, "{ffffffff-ffff-ffff-ffff-ffffffffffff}").is_err()
    );
}

#[test]
fn malformed_bcd_is_rejected() {
    assert!(crate::bootstore::bcd_targets(b"not a hive", "BCD").is_err());
}

#[test]
fn ntfs_fixture_is_read_without_mounting_or_write_access() {
    use std::io::{Cursor, Read};
    let mut bytes = vec![];
    flate2::read::GzDecoder::new(&include_bytes!("../tests/fixtures/ntfs-clean.img.gz")[..])
        .read_to_end(&mut bytes)
        .unwrap();
    assert_eq!(
        crate::disk::inspect_ntfs(Cursor::new(&bytes)).unwrap(),
        Hibernation::NotDetected
    );
    bytes[3..11].fill(0);
    assert!(crate::disk::inspect_ntfs(Cursor::new(&bytes)).is_err());
}

#[test]
fn mount_inventory_handles_spaces_and_excludes_virtual_filesystems() {
    let info = "1 0 8:1 / /media/other\\040disk rw - ext4 /dev/sda1 rw\n2 0 0:1 / /proc rw - proc proc rw\n";
    assert_eq!(crate::disk::mount_points(info), ["/media/other disk"]);
}

fn elf() -> Vec<u8> {
    let mut bytes = vec![0; 64];
    bytes[..6].copy_from_slice(b"\x7fELF\x02\x01");
    bytes[18..20].copy_from_slice(&[62, 0]);
    bytes
}
fn tar_archive(name: &str, bytes: &[u8]) -> Vec<u8> {
    let gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    let mut archive = tar::Builder::new(gzip);
    let mut header = tar::Header::new_gnu();
    header.set_size(bytes.len() as u64);
    header.set_mode(0o755);
    header.set_cksum();
    archive.append_data(&mut header, name, bytes).unwrap();
    archive.into_inner().unwrap().finish().unwrap()
}

#[test]
fn verified_update_replaces_only_through_injected_installer() {
    use sha2::{Digest, Sha256};
    let mut http = Simulated::default();
    http.responses.push_back(release("0.2.0"));
    let status = update::check(&mut http, Platform::Linux, "0.1.0").unwrap();
    let executable = elf();
    let archive = tar_archive("boothop", &executable);
    let hash: String = Sha256::digest(&archive)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    http.responses.push_back(response(
        200,
        &format!("{hash}  boothop-v0.2.0-linux-x86_64.tar.gz\n"),
    ));
    http.responses.push_back(Response {
        status: 200,
        body: archive,
    });
    let mut installer = Simulated::default();
    update::install(&mut http, &mut installer, Platform::Linux, &status).unwrap();
    assert_eq!(installer.installed, Some(executable));
}

#[test]
fn update_rejects_unexpected_executable_path_and_architecture() {
    assert!(update::extract(Platform::Linux, &tar_archive("unexpected/boothop", &elf())).is_err());
    let mut executable = elf();
    executable[18] = 3;
    assert!(update::extract(Platform::Linux, &tar_archive("boothop", &executable)).is_err());
}

#[test]
fn windows_archive_is_validated_without_executing_pe() {
    use std::io::{Cursor, Write};
    let mut executable = vec![0; 128];
    executable[..2].copy_from_slice(b"MZ");
    executable[60..64].copy_from_slice(&64u32.to_le_bytes());
    executable[64..70].copy_from_slice(b"PE\0\0\x64\x86");
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    zip.start_file("boothop.exe", zip::write::SimpleFileOptions::default())
        .unwrap();
    zip.write_all(&executable).unwrap();
    let archive = zip.finish().unwrap().into_inner();
    assert_eq!(
        update::extract(Platform::Windows, &archive).unwrap(),
        executable
    );
}

#[test]
fn installation_restores_after_partial_activation_failure() {
    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("boothop");
    std::fs::write(&destination, b"old executable").unwrap();
    let result = crate::installation::replace_using(&destination, b"new executable", |_| {
        std::fs::remove_file(&destination)?;
        bail!("simulated activation failure")
    });
    assert!(result.unwrap_err().to_string().contains("restored"));
    assert_eq!(std::fs::read(&destination).unwrap(), b"old executable");
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 2);
}

#[test]
fn installation_verifies_new_bytes_and_keeps_permissions() {
    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("boothop");
    std::fs::write(&destination, b"old").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&destination, std::fs::Permissions::from_mode(0o751)).unwrap();
    }
    crate::installation::replace_using(&destination, b"new", |path| {
        std::fs::copy(path, &destination)?;
        Ok(())
    })
    .unwrap();
    assert_eq!(std::fs::read(&destination).unwrap(), b"new");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&destination)
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o751
        );
    }
}

#[test]
fn concurrent_installation_fails_before_activation() {
    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("boothop");
    std::fs::write(&destination, b"old").unwrap();
    let lock = std::fs::File::create(directory.path().join(".boothop-update.lock")).unwrap();
    lock.lock().unwrap();
    let result =
        crate::installation::replace_using(&destination, b"new", |_| panic!("must not activate"));
    assert!(result.unwrap_err().to_string().contains("Another update"));
    assert_eq!(std::fs::read(&destination).unwrap(), b"old");
}

#[test]
fn operating_system_shutdown_arguments_never_request_a_raw_reset() {
    assert_eq!(
        crate::shutdown::systemd_arguments(&Closure::Force).unwrap(),
        ["reboot", "--force", "--no-block"]
    );
    assert_eq!(
        crate::shutdown::sysv_arguments(&Closure::ForceAfter(60)).unwrap(),
        ["-t", "60", "6"]
    );
    assert!(crate::shutdown::sysv_arguments(&Closure::ForceAfter(u32::MAX)).is_err());
}

#[test]
fn windows_service_owns_grace_interval_after_foreground_exit() {
    struct Service {
        events: Vec<String>,
        foreground_alive: bool,
    }
    impl crate::shutdown::ApplicationGraceService for Service {
        fn request_applications(&mut self) -> Result<()> {
            self.events.push("request applications".into());
            Ok(())
        }
        fn accepted(&mut self) -> Result<()> {
            self.foreground_alive = false;
            self.events.push("accepted; CLI exited".into());
            Ok(())
        }
        fn wait(&mut self, seconds: u32) {
            assert!(!self.foreground_alive);
            self.events.push(format!("service waits {seconds}"));
        }
        fn force_restart(&mut self) -> Result<()> {
            self.events.push("OS forced closure".into());
            Ok(())
        }
    }
    let mut service = Service {
        events: vec![],
        foreground_alive: true,
    };
    crate::shutdown::application_grace_restart(&mut service, 60).unwrap();
    assert_eq!(
        service.events,
        [
            "request applications",
            "accepted; CLI exited",
            "service waits 60",
            "OS forced closure"
        ]
    );
}

#[test]
fn firmware_loader_identity_uses_partition_guid_not_display_name() {
    let partition = uuid::Uuid::parse_str("12345678-1234-5678-9abc-123456789abc").unwrap();
    let mut harddrive = vec![0_u8; 42];
    harddrive[..4].copy_from_slice(&[4, 1, 42, 0]);
    harddrive[24..40].copy_from_slice(&partition.to_bytes_le());
    harddrive[40..42].copy_from_slice(&[2, 2]);
    let text = "\\EFI\\Microsoft\\Boot\\bootmgfw.efi\0"
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect::<Vec<_>>();
    let mut path = harddrive;
    path.extend([4, 4]);
    path.extend(((text.len() + 4) as u16).to_le_bytes());
    path.extend(text);
    path.extend([127, 255, 4, 0]);
    let mut load_option = vec![1, 0, 0, 0];
    load_option.extend((path.len() as u16).to_le_bytes());
    load_option.extend([0, 0]);
    load_option.extend(path);
    let parsed = boot::firmware_path(&load_option).unwrap();
    assert_eq!(parsed.partition, Some(partition));
    assert_eq!(parsed.files, ["\\efi\\microsoft\\boot\\bootmgfw.efi"]);
    load_option.pop();
    assert!(boot::firmware_path(&load_option).is_err());
}

#[test]
fn grub_chain_requires_matching_filesystem_and_configuration() {
    let redirect = "search.fs_uuid abc-123 root\nset prefix=($root)'/boot/grub'\nconfigfile $prefix/grub.cfg\n";
    assert!(boot::grub_redirect_matches(
        redirect,
        "abc-123",
        "/boot/grub/grub.cfg"
    ));
    assert!(!boot::grub_redirect_matches(
        redirect,
        "another-volume",
        "/boot/grub/grub.cfg"
    ));
    assert!(!boot::grub_redirect_matches(
        redirect,
        "abc-123",
        "/other/grub.cfg"
    ));
    assert!(!boot::grub_redirect_matches(
        &(redirect.to_owned() + "set root=other\n"),
        "abc-123",
        "/boot/grub/grub.cfg"
    ));
}

#[test]
fn update_reports_missing_artifacts_and_api_failures_without_installation() {
    let mut http = Simulated::default();
    http.responses.push_back(response(
        200,
        r#"{"tag_name":"v0.2.0","draft":false,"prerelease":false,"assets":[]}"#,
    ));
    assert!(matches!(
        update::check(&mut http, Platform::Linux, "0.1.0").unwrap(),
        UpdateStatus::Unavailable(_)
    ));
    http.responses.push_back(response(429, "rate limited"));
    assert!(
        update::check(&mut http, Platform::Linux, "0.1.0")
            .unwrap_err()
            .to_string()
            .contains("429")
    );
    assert!(http.installed.is_none());
}

#[test]
fn replacement_failure_preserves_locked_existing_file() {
    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("boothop");
    std::fs::write(&destination, b"old").unwrap();
    let result = crate::installation::replace_using(&destination, b"new", |_| {
        Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "simulated running executable lock",
        )
        .into())
    });
    assert!(result.is_err());
    assert_eq!(std::fs::read(&destination).unwrap(), b"old");
}

#[cfg(unix)]
#[test]
fn systemd_service_script_uses_only_injected_process_simulations() {
    use std::os::unix::fs::PermissionsExt;
    let directory = tempfile::tempdir().unwrap();
    let systemctl = directory.path().join("mock-systemctl");
    let sleep = directory.path().join("mock-sleep");
    let log = directory.path().join("events");
    let receipt = directory.path().join("receipt");
    let errors = directory.path().join("errors");
    std::fs::write(&systemctl, "#!/bin/sh\nprintf 'systemctl %s\\n' \"$*\" >>\"$BOOTHOP_SIMULATION_LOG\"\nexit \"${BOOTHOP_SIMULATION_REJECT:-0}\"\n").unwrap();
    std::fs::write(
        &sleep,
        "#!/bin/sh\nprintf 'sleep %s\\n' \"$*\" >>\"$BOOTHOP_SIMULATION_LOG\"\n",
    )
    .unwrap();
    for path in [&systemctl, &sleep] {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    for reject in [false, true] {
        std::fs::write(&log, "").unwrap();
        let output = std::process::Command::new("/bin/sh")
            .args(["-c", crate::shutdown::ESCALATION_SCRIPT, "simulation"])
            .arg(&systemctl)
            .arg(&sleep)
            .arg("60")
            .arg(&receipt)
            .arg(&errors)
            .env("BOOTHOP_SIMULATION_LOG", &log)
            .env("BOOTHOP_SIMULATION_REJECT", if reject { "1" } else { "0" })
            .output()
            .unwrap();
        let events = std::fs::read_to_string(&log).unwrap();
        if reject {
            assert!(!output.status.success());
            assert_eq!(events, "systemctl reboot --no-block\n");
            assert_eq!(std::fs::read_to_string(&receipt).unwrap(), "rejected\n");
        } else {
            assert!(output.status.success());
            assert_eq!(
                events,
                "systemctl reboot --no-block\nsleep 60\nsystemctl reboot --force --no-block\n"
            );
            assert_eq!(std::fs::read_to_string(&receipt).unwrap(), "accepted\n");
        }
    }
    assert!(crate::shutdown::ESCALATION_PROPERTIES.contains(&"DefaultDependencies=no"));
    assert!(crate::shutdown::ESCALATION_PROPERTIES.contains(&"IgnoreOnIsolate=yes"));
}

#[test]
fn dirty_ntfs_does_not_turn_missing_evidence_into_a_clean_result() {
    assert!(matches!(
        crate::disk::ntfs_evidence(true, None),
        Hibernation::Undetermined(_)
    ));
    assert!(matches!(
        crate::disk::ntfs_evidence(true, Some(&[0; 4096])),
        Hibernation::Undetermined(_)
    ));
    let mut header = [0; 4096];
    header[..4].copy_from_slice(b"HIBR");
    assert_eq!(
        crate::disk::ntfs_evidence(true, Some(&header)),
        Hibernation::Detected
    );
}

#[test]
fn application_grace_failure_does_not_acknowledge_or_force_restart() {
    struct Service;
    impl crate::shutdown::ApplicationGraceService for Service {
        fn request_applications(&mut self) -> Result<()> {
            bail!("session unavailable")
        }
        fn accepted(&mut self) -> Result<()> {
            panic!("must not acknowledge")
        }
        fn wait(&mut self, _: u32) {
            panic!("must not start timer")
        }
        fn force_restart(&mut self) -> Result<()> {
            panic!("must not force restart")
        }
    }
    assert!(crate::shutdown::application_grace_restart(&mut Service, 60).is_err());
}

#[test]
fn old_and_new_lsblk_formats_produce_the_same_disk_identity() {
    let json = r#"{"blockdevices":[{"name":"/dev/sda","size":8192,"children":[{"name":"/dev/sda1","pkname":"/dev/sda","size":"4096","fstype":"ntfs","label":"Data disk","mountpoint":null}]}]}"#;
    let pairs = "NAME=\"/dev/sda\" SIZE=\"8192\"\nNAME=\"/dev/sda1\" PKNAME=\"/dev/sda\" SIZE=\"4096\" FSTYPE=\"ntfs\" LABEL=\"Data\\x20disk\" MOUNTPOINT=\"\"\n";
    let first = crate::disk::lsblk_json(json).unwrap();
    let second = crate::disk::lsblk_pairs(pairs).unwrap();
    assert_eq!(first.len(), second.len());
    assert_eq!(first[1].id, second[1].id);
    assert_eq!(second[1].label, "Data disk");
    assert_eq!(second[1].parent.as_deref(), Some("/dev/sda"));
    assert!(crate::disk::lsblk_json(r#"{"blockdevices":[{"name":"/dev/sda"}]}"#).is_err());
}

#[test]
fn grub_one_shot_requires_consumption_not_comments() {
    let config = "load_env\nif [ \"${next_entry}\" ]; then\nset default=\"${next_entry}\"\nset next_entry=\nsave_env next_entry\nfi\n";
    assert!(boot::grub_consumes_one_shot(config));
    assert!(!boot::grub_consumes_one_shot(
        &config.replace("save_env next_entry", "# save_env next_entry")
    ));
    assert!(!boot::grub_consumes_one_shot(
        &config.replace("set next_entry=", "# set next_entry=")
    ));
}

fn user_process(pid: i32) -> crate::shutdown::UserProcess {
    crate::shutdown::UserProcess {
        pid,
        started: 123,
        uid: 1000,
        session: 10,
        terminal: 0,
        login_uid: Some(1000),
    }
}

#[derive(Default)]
struct ProcessSimulation {
    events: Vec<String>,
    saved: Option<crate::shutdown::ProcessGraceState>,
    now: u64,
    fail: Option<&'static str>,
}
impl ProcessSimulation {
    fn event(&mut self, name: &str) -> Result<()> {
        self.events.push(name.into());
        if self.fail == Some(name) {
            bail!("simulated {name} failure");
        }
        Ok(())
    }
}
impl crate::shutdown::ProcessGraceSystem for ProcessSimulation {
    fn processes(&mut self) -> Result<Vec<crate::shutdown::UserProcess>> {
        self.event("snapshot")?;
        Ok(vec![user_process(42)])
    }
    fn now_ms(&self) -> u64 {
        self.now
    }
    fn persist(&mut self, state: &crate::shutdown::ProcessGraceState) -> Result<()> {
        self.event(if state.submitted {
            "persist-submitted"
        } else {
            "persist"
        })?;
        self.saved = Some(state.clone());
        Ok(())
    }
    fn signal(&mut self, _: &crate::shutdown::UserProcess, force: bool) -> Result<()> {
        self.event(if force { "kill" } else { "term" })?;
        self.now += 5;
        Ok(())
    }
    fn accepted(&mut self) -> Result<()> {
        self.event("accepted")
    }
    fn wait_until(&mut self, deadline: u64) {
        self.events.push(format!("wait:{deadline}"));
        self.now = self.now.max(deadline);
    }
    fn restart(&mut self) -> Result<()> {
        self.event("restart")
    }
}

#[test]
fn openrc_grace_waits_after_requests_before_normal_restart() {
    let mut system = ProcessSimulation::default();
    crate::shutdown::process_grace_restart(&mut system, 60, None).unwrap();
    assert_eq!(
        system.events,
        [
            "snapshot",
            "persist",
            "term",
            "persist",
            "accepted",
            "wait:60005",
            "kill",
            "persist-submitted",
            "restart"
        ]
    );
}

#[test]
fn openrc_worker_resumes_original_deadline_without_repeating_grace_requests() {
    let previous = crate::shutdown::ProcessGraceState {
        processes: vec![user_process(42)],
        deadline_ms: Some(60_000),
        submitted: false,
    };
    let mut system = ProcessSimulation {
        now: 30_000,
        ..Default::default()
    };
    crate::shutdown::process_grace_restart(&mut system, 60, Some(previous)).unwrap();
    assert_eq!(
        system.events,
        [
            "persist",
            "accepted",
            "wait:60000",
            "kill",
            "persist-submitted",
            "restart"
        ]
    );
}

#[test]
fn openrc_worker_never_repeats_an_uncertain_restart() {
    let previous = crate::shutdown::ProcessGraceState {
        processes: vec![user_process(42)],
        deadline_ms: Some(60_000),
        submitted: true,
    };
    let mut system = ProcessSimulation::default();
    let error =
        crate::shutdown::process_grace_restart(&mut system, 60, Some(previous)).unwrap_err();
    assert!(error.downcast_ref::<reboot::ShutdownUncertain>().is_some());
    assert!(system.events.is_empty());
}

#[test]
fn openrc_force_skips_term_and_grace_but_keeps_normal_restart() {
    let mut system = ProcessSimulation::default();
    crate::shutdown::process_grace_restart(&mut system, 0, None).unwrap();
    assert_eq!(
        system.events,
        [
            "snapshot",
            "persist",
            "persist",
            "accepted",
            "wait:0",
            "kill",
            "persist-submitted",
            "restart"
        ]
    );
}

#[test]
fn openrc_failure_at_each_boundary_does_not_advance_to_restart() {
    for step in [
        "snapshot",
        "persist",
        "term",
        "accepted",
        "kill",
        "persist-submitted",
    ] {
        let mut system = ProcessSimulation {
            fail: Some(step),
            ..Default::default()
        };
        assert!(crate::shutdown::process_grace_restart(&mut system, 60, None).is_err());
        assert!(
            !system.events.iter().any(|event| event == "restart"),
            "{step}"
        );
        assert_eq!(system.events.last().map(String::as_str), Some(step));
    }
}

#[test]
fn process_identity_rejects_recycled_pids_and_changed_owners() {
    let expected = user_process(42);
    let mut current = expected.clone();
    assert!(crate::shutdown::same_process(&expected, &current));
    current.started += 1;
    assert!(!crate::shutdown::same_process(&expected, &current));
    current = expected.clone();
    current.uid = 0;
    assert!(!crate::shutdown::same_process(&expected, &current));
}

#[test]
fn user_process_selection_excludes_services_and_covers_tty_and_audit_sessions() {
    let graphical = user_process(42);
    let mut terminal = user_process(43);
    terminal.login_uid = None;
    terminal.terminal = 123;
    let mut child = terminal.clone();
    child.pid = 44;
    child.terminal = 0;
    let mut service = user_process(45);
    service.uid = 80;
    service.login_uid = None;
    let mut detached_service = user_process(46);
    detached_service.login_uid = None;
    detached_service.session = 99;
    let mut supervisor = user_process(47);
    supervisor.uid = 0;
    let selected = crate::shutdown::user_processes(
        vec![
            graphical,
            terminal,
            child,
            service,
            detached_service,
            supervisor,
        ],
        1000,
    );
    assert_eq!(
        selected.iter().map(|p| p.pid).collect::<Vec<_>>(),
        [42, 43, 44]
    );
}

#[test]
fn proc_stat_handles_spaces_and_parentheses_without_shifting_identity_fields() {
    let mut fields = vec!["0"; 20];
    fields[0] = "S";
    fields[3] = "10";
    fields[4] = "123";
    fields[19] = "999";
    let stat = format!("42 (process ) name) {}", fields.join(" "));
    let process = crate::shutdown::process_record(
        42,
        &stat,
        "Name: app\nUid:\t1000\t1000\t1000\t1000\n",
        "4294967295\n",
    )
    .unwrap();
    assert_eq!(
        (
            process.started,
            process.session,
            process.terminal,
            process.login_uid
        ),
        (999, 10, 123, None)
    );
    assert!(crate::shutdown::process_record(42, "42 (truncated) S 1", "", "").is_err());
}

#[test]
fn service_cleanup_cannot_turn_possible_restart_into_rejection() {
    use crate::shutdown::{ServiceAcceptance, service_acceptance};
    assert_eq!(service_acceptance("", false), ServiceAcceptance::Pending);
    assert_eq!(
        service_acceptance("accepted", true),
        ServiceAcceptance::Accepted
    );
    assert_eq!(service_acceptance("", true), ServiceAcceptance::Uncertain);
    assert_eq!(
        service_acceptance("rejected", true),
        ServiceAcceptance::Rejected
    );
}

#[test]
fn uki_discovery_reads_metadata_without_loading_the_executable() {
    let mut image = vec![0u8; 2048];
    image[..2].copy_from_slice(b"MZ");
    image[0x3c..0x40].copy_from_slice(&128u32.to_le_bytes());
    image[128..132].copy_from_slice(b"PE\0\0");
    image[132..134].copy_from_slice(&0x8664u16.to_le_bytes());
    image[134..136].copy_from_slice(&2u16.to_le_bytes());
    image[148..150].copy_from_slice(&240u16.to_le_bytes());
    image[152..154].copy_from_slice(&0x20bu16.to_le_bytes());
    image[220..222].copy_from_slice(&10u16.to_le_bytes());
    for (index, name, offset, size) in [(0, b".osrel", 1024u32, 64u32), (1, b".linux", 1536, 128)] {
        let start = 392 + index * 40;
        image[start..start + 6].copy_from_slice(name);
        image[start + 16..start + 20].copy_from_slice(&size.to_le_bytes());
        image[start + 20..start + 24].copy_from_slice(&offset.to_le_bytes());
    }
    let text = b"PRETTY_NAME=\"Example Linux\"\n";
    image[1024..1024 + text.len()].copy_from_slice(text);
    let target = boot::unified_entry("example.efi", &image, "/efi/EFI/Linux/example.efi").unwrap();
    assert_eq!(target.label, "Example Linux");
    assert_eq!(
        target.selection,
        Selection::SystemdBoot("example.efi".into())
    );
    image[412..416].copy_from_slice(&4096u32.to_le_bytes());
    assert!(boot::unified_entry("example.efi", &image, "/efi/EFI/Linux/example.efi").is_err());
}

#[test]
fn current_loader_marker_requires_a_unique_entry_and_firmware_path() {
    let first = boot::loader_entry(
        "linux.conf",
        "linux /kernel",
        "/efi/loader/entries/linux.conf",
    )
    .unwrap();
    let second = boot::loader_entry(
        "linux.conf",
        "linux /kernel",
        "/other/loader/entries/linux.conf",
    )
    .unwrap();
    let loaders = vec![(first.id.clone(), 2), (second.id.clone(), 3)];
    let mut targets = vec![first.clone(), second.clone()];
    boot::mark_current_loader(&mut targets, Some("linux.conf"), Some(2), &loaders);
    assert!(targets[0].current && !targets[1].current);
    let mut ambiguous = vec![first, second];
    let ambiguous_loaders = vec![(ambiguous[0].id.clone(), 2), (ambiguous[1].id.clone(), 2)];
    boot::mark_current_loader(
        &mut ambiguous,
        Some("linux.conf"),
        Some(2),
        &ambiguous_loaders,
    );
    assert!(ambiguous.iter().all(|target| !target.current));
}

#[test]
fn disk_probes_preserve_partial_results_and_do_not_read_busy_volumes() {
    for prefix in ["/dev/", r"\\?\Volume"] {
        let disks = (0..4)
            .map(|index| Disk {
                id: format!("{prefix}{index}"),
                parent: None,
                size: 8192,
                filesystem: "NTFS".into(),
                label: "Example".into(),
                mount: if index == 0 {
                    "mounted".into()
                } else {
                    String::new()
                },
                hibernation: None,
            })
            .collect::<Vec<_>>();
        let mut calls = Vec::new();
        let result = crate::disk::inspect_inventory(
            Inventory {
                items: disks,
                warnings: vec!["Another disk is inaccessible".into()],
            },
            true,
            |id| {
                calls.push(id.to_owned());
                if id.ends_with('1') {
                    bail!("permission denied");
                }
                if id.ends_with('2') {
                    let mut header = [0; 4096];
                    header[..4].copy_from_slice(b"hibr");
                    return Ok(crate::disk::hibernation_header(&header));
                }
                Ok(Hibernation::NotDetected)
            },
        );
        assert_eq!(calls.len(), 3);
        assert!(
            result.items[..2]
                .iter()
                .all(|disk| matches!(disk.hibernation, Some(Hibernation::Undetermined(_))))
        );
        assert_eq!(result.items[2].hibernation, Some(Hibernation::Detected));
        assert_eq!(result.items[3].hibernation, Some(Hibernation::NotDetected));
        assert_eq!(result.warnings, ["Another disk is inaccessible"]);
    }
}

#[test]
fn ordinary_disk_listing_never_calls_raw_inspection() {
    let disk = Disk {
        id: "fixture".into(),
        parent: None,
        size: 4096,
        filesystem: "ntfs".into(),
        label: String::new(),
        mount: String::new(),
        hibernation: None,
    };
    let result = crate::disk::inspect_inventory(
        Inventory {
            items: vec![disk],
            warnings: vec![],
        },
        false,
        |_| panic!("raw inspection is forbidden"),
    );
    assert_eq!(result.items[0].hibernation, None);
}

#[test]
fn failed_update_recovery_retains_a_readable_original_backup() {
    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("boothop");
    std::fs::write(&destination, b"original executable fixture").unwrap();
    let result = crate::installation::replace_using(&destination, b"replacement", |_| {
        std::fs::remove_file(&destination)?;
        std::fs::create_dir(&destination)?;
        bail!("simulated activation and recovery failure")
    })
    .unwrap_err();
    assert!(result.to_string().contains("Recovery failed"));
    let backup = std::fs::read_dir(directory.path())
        .unwrap()
        .filter_map(|entry| {
            let path = entry.ok()?.path();
            path.file_name()?
                .to_str()?
                .starts_with(".boothop-update-")
                .then(|| path.join("boothop-backup"))
        })
        .find(|path| path.is_file())
        .unwrap();
    assert_eq!(
        std::fs::read(backup).unwrap(),
        b"original executable fixture"
    );
}

#[test]
fn incompatible_and_timed_out_probes_use_a_supported_alternative() {
    let mut calls = Vec::new();
    let selected = crate::capability::first_usable(
        &["missing", "incompatible", "timeout", "compatible"],
        |name| {
            calls.push(name.to_owned());
            match name {
                "missing" => bail!("not installed"),
                "incompatible" => bail!("operation unsupported"),
                "timeout" => bail!("probe exceeded time limit"),
                _ => Ok(name.to_owned()),
            }
        },
    )
    .unwrap();
    assert_eq!(selected, "compatible");
    assert_eq!(calls, ["missing", "incompatible", "timeout", "compatible"]);
    let error =
        crate::capability::first_usable::<()>(&["only candidate"], |_| bail!("permission denied"))
            .unwrap_err();
    assert!(error.to_string().contains("permission denied"));
}

#[test]
fn native_confirmation_and_terminal_fallback_require_explicit_approval() {
    struct Prompt {
        dialog: Option<bool>,
        interactive: bool,
        answer: &'static str,
        calls: usize,
    }
    impl crate::capability::Confirmation for Prompt {
        fn dialog(&mut self, _: &str) -> Option<bool> {
            self.dialog
        }
        fn interactive(&self) -> bool {
            self.interactive
        }
        fn terminal(&mut self, _: &str) -> Result<bool> {
            self.calls += 1;
            Ok(crate::capability::terminal_answer(self.answer))
        }
    }
    for dialog in [Some(true), Some(false)] {
        let mut prompt = Prompt {
            dialog,
            interactive: true,
            answer: "yes",
            calls: 0,
        };
        assert_eq!(
            crate::capability::confirm(&mut prompt, "Restart?").unwrap(),
            dialog.unwrap()
        );
        assert_eq!(prompt.calls, 0);
    }
    for (answer, expected) in [
        ("", false),
        ("no", false),
        ("anything", false),
        ("YES\n", true),
    ] {
        let mut prompt = Prompt {
            dialog: None,
            interactive: true,
            answer,
            calls: 0,
        };
        assert_eq!(
            crate::capability::confirm(&mut prompt, "Restart?").unwrap(),
            expected
        );
        assert_eq!(prompt.calls, 1);
    }
    let mut prompt = Prompt {
        dialog: None,
        interactive: false,
        answer: "yes",
        calls: 0,
    };
    assert!(
        crate::capability::confirm(&mut prompt, "Restart?")
            .unwrap_err()
            .to_string()
            .contains("--yes")
    );
    assert_eq!(prompt.calls, 0);
}

#[test]
fn version_reports_package_version_without_probing_the_machine() {
    let mut system = Simulated::default();
    let result = run(&["boothop", "--version"], &mut system);
    assert_eq!(result.code, 0);
    assert!(result.stdout.contains(env!("CARGO_PKG_VERSION")));
    assert!(system.events.is_empty());
}

#[test]
fn discovery_preserves_ids_and_partial_results_at_every_detail_level() {
    let mut system = Simulated {
        warnings: vec!["Another boot source is inaccessible".into()],
        ..Default::default()
    };
    system.targets.extend(
        boot::grub_entries("menuentry 'Linux' --id linux {\n}\n", "/boot/grub/grub.cfg").items,
    );
    system.targets.push(
        boot::loader_entry(
            "linux.conf",
            "title Linux\nlinux /kernel",
            "/efi/loader/entries/linux.conf",
        )
        .unwrap(),
    );
    let mut windows = target();
    windows.id = "bcd:fixture".into();
    windows.selection = Selection::Bcd("fixture".into());
    system.targets.push(windows);
    let ids = system
        .targets
        .iter()
        .map(|target| target.id.clone())
        .collect::<Vec<_>>();
    for args in [
        vec!["boothop", "systems"],
        vec!["boothop", "systems", "-v"],
        vec!["boothop", "systems", "-vv"],
    ] {
        system.targets.reverse();
        let result = run(&args, &mut system);
        assert_eq!(result.code, 0);
        for id in &ids {
            assert!(result.stdout.contains(id));
        }
        assert!(result.stderr.contains("inaccessible"));
    }
    assert!(system.events.iter().all(|event| event == "discover"));
    assert!(system.values.is_empty());
}

#[test]
fn rejected_restart_reports_failed_recovery_and_retained_keys() {
    let mut system = Simulated {
        fail_shutdown: true,
        fail_recovery: true,
        ..Default::default()
    };
    let error = reboot::execute(&mut system, &request())
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("Recovery incomplete")
            && error.contains("BootNext")
            && error.contains("nested")
    );
    assert_eq!(system.values.len(), 2);
}
