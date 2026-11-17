mod boot_chain;
mod firmware;
#[cfg(not(windows))]
mod linux_shutdown;
#[cfg(not(windows))]
pub mod openrc;
mod process;
#[cfg(windows)]
mod raw_linux;
mod reboot_lock;
mod storage;
#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub mod windows_escalation;

use crate::{
    boot::{self, Inventory, Selection, Target},
    bootstore,
    cli::System,
    disk::Disk,
    reboot::{Change, Closure, RebootSystem, Request},
    update::{Installer, Platform, Response, Transport},
};
use anyhow::{Context, Result, bail, ensure};
use process::find;
use std::{
    fs,
    io::{IsTerminal, Write},
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
#[cfg(not(windows))]
use {process::run, std::path::Path};

fn validate_grub(path: &std::path::Path) -> Result<()> {
    let config = fs::read_to_string(path)?;
    ensure!(
        boot::grub_consumes_one_shot(&config),
        "GRUB configuration does not support consuming next_entry"
    );
    let environment = path.with_file_name("grubenv");
    boot_chain::validate_grub_environment(&environment)?;
    let bytes = fs::read(&environment)?;
    bootstore::select_grub(&bytes, "probe")?;
    fs::OpenOptions::new()
        .write(true)
        .open(&environment)
        .context("GRUB environment is not writable")?;
    Ok(())
}

pub struct Host {
    cancelled: Arc<AtomicBool>,
    reboot_lock: Option<reboot_lock::RebootLock>,
    application_grace: bool,
}
impl Default for Host {
    fn default() -> Self {
        Self::new()
    }
}
impl Host {
    pub fn new() -> Self {
        Self {
            cancelled: Arc::new(AtomicBool::new(false)),
            reboot_lock: None,
            application_grace: cfg!(windows),
        }
    }

    fn loader_roots(&self) -> Result<Vec<PathBuf>> {
        #[cfg(windows)]
        {
            let json = windows::powershell(
                "$paths=@(Get-Partition | ForEach-Object { $_.AccessPaths } | Where-Object {$_} | Sort-Object -Unique); ConvertTo-Json -InputObject $paths -Compress",
            )?;
            Ok(serde_json::from_str::<Vec<String>>(&json)?
                .into_iter()
                .map(PathBuf::from)
                .collect())
        }
        #[cfg(not(windows))]
        {
            let mut roots = ["/boot", "/boot/efi", "/efi"]
                .iter()
                .map(PathBuf::from)
                .collect::<Vec<_>>();
            roots.extend(
                crate::disk::mount_points(&fs::read_to_string("/proc/self/mountinfo")?)
                    .into_iter()
                    .map(PathBuf::from),
            );
            roots.sort();
            roots.dedup();
            Ok(roots)
        }
    }

    fn terminal_confirmation(&self, message: &str) -> Result<bool> {
        ensure!(
            std::io::stdin().is_terminal() && std::io::stderr().is_terminal(),
            "No confirmation dialog or interactive terminal is available. Use --yes to approve explicitly."
        );
        eprint!("{message}\nConfirm [y/N]: ");
        std::io::stderr().flush()?;
        let mut line = String::new();
        std::io::stdin().read_line(&mut line)?;
        Ok(crate::capability::terminal_answer(&line))
    }
}

impl RebootSystem for Host {
    fn application_grace_period(&self) -> bool {
        self.application_grace
    }
    fn cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }
    fn discover(&mut self) -> Result<Inventory<Target>> {
        let mut inventory = firmware::discover();
        #[cfg(windows)]
        match raw_linux::discover() {
            Ok(mut entries) => {
                inventory.items.append(&mut entries.items);
                inventory.warnings.append(&mut entries.warnings);
            }
            Err(error) => inventory.warnings.push(format!(
                "Linux filesystem discovery is incomplete: {error:#}"
            )),
        }
        let roots = match self.loader_roots() {
            Ok(roots) => roots,
            Err(error) => {
                inventory.warnings.push(format!(
                    "Boot filesystem discovery is incomplete: {error:#}"
                ));
                Vec::new()
            }
        };
        let selected = firmware::read("LoaderEntrySelected", firmware::LOADER_GUID)
            .ok()
            .flatten()
            .and_then(|bytes| firmware::decode_text(&bytes).ok());
        let current_loader = firmware::read("BootCurrent", firmware::EFI_GUID)
            .ok()
            .flatten()
            .filter(|bytes| bytes.len() == 2)
            .map(|bytes| u16::from_le_bytes([bytes[0], bytes[1]]));
        let mut loaders = Vec::new();
        for root in &roots {
            if let Err(error) = fs::read_dir(root) {
                if error.kind() == std::io::ErrorKind::NotFound {
                    continue;
                }
                inventory.warnings.push(format!(
                    "Boot filesystem {} could not be inspected: {error}",
                    root.display()
                ));
                continue;
            }
            for (relative, extension) in [("loader/entries", "conf"), ("EFI/Linux", "efi")] {
                let entries = root.join(relative);
                match fs::read_dir(&entries) {
                    Ok(files) => {
                        for file in files {
                            if file.as_ref().is_ok_and(|file| {
                                file.path()
                                    .extension()
                                    .is_none_or(|ext| !ext.eq_ignore_ascii_case(extension))
                                    || file.file_name().to_string_lossy().starts_with('.')
                            }) {
                                continue;
                            }
                            let result = (|| -> Result<Target> {
                                let path = file?.path();
                                ensure!(
                                    fs::symlink_metadata(&path)?.is_file(),
                                    "Boot entry is not a regular file"
                                );
                                let id = path
                                    .file_name()
                                    .and_then(|value| value.to_str())
                                    .context("Invalid loader entry name")?;
                                let mut target = if extension == "conf" {
                                    boot::loader_entry(
                                        id,
                                        &fs::read_to_string(&path)?,
                                        &path.display().to_string(),
                                    )?
                                } else {
                                    use std::io::Read;
                                    let mut bytes = Vec::new();
                                    fs::File::open(&path)?
                                        .take(128 * 1024 * 1024 + 1)
                                        .read_to_end(&mut bytes)?;
                                    ensure!(
                                        bytes.len() <= 128 * 1024 * 1024,
                                        "UKI exceeds the supported 128 MiB inspection limit"
                                    );
                                    boot::unified_entry(id, &bytes, &path.display().to_string())?
                                };
                                match boot_chain::loader(&target, &roots) {
                                    Ok(number) => loaders.push((target.id.clone(), number)),
                                    Err(error) => target.unavailable = Some(error.to_string()),
                                };
                                Ok(target)
                            })();
                            match result {
                                Ok(target) => inventory.items.push(target),
                                Err(error) => inventory
                                    .warnings
                                    .push(format!("{}: {error:#}", entries.display())),
                            }
                        }
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => inventory
                        .warnings
                        .push(format!("{}: {error}", entries.display())),
                }
            }
        }
        boot::mark_current_loader(
            &mut inventory.items,
            selected.as_deref(),
            current_loader,
            &loaders,
        );
        let mut configs = roots
            .iter()
            .flat_map(|root| {
                [
                    "grub/grub.cfg",
                    "grub2/grub.cfg",
                    "boot/grub/grub.cfg",
                    "boot/grub2/grub.cfg",
                ]
                .map(|path| root.join(path))
            })
            .filter_map(|path| match path.canonicalize() {
                Ok(path) => Some(path),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                Err(error) => {
                    inventory
                        .warnings
                        .push(format!("{}: {error}", path.display()));
                    None
                }
            })
            .collect::<Vec<_>>();
        configs.sort();
        configs.dedup();
        for path in configs.into_iter().filter(|path| path.exists()) {
            match fs::read_to_string(&path) {
                Ok(text) => {
                    let mut parsed = boot::grub_entries(&text, &path.display().to_string());
                    for target in &mut parsed.items {
                        if let Err(error) = validate_grub(&path)
                            .and_then(|()| boot_chain::loader(target, &roots).map(|_| ()))
                        {
                            target.unavailable = Some(format!("{error:#}"));
                        }
                    }
                    inventory.items.extend(parsed.items);
                    inventory.warnings.extend(parsed.warnings);
                }
                Err(error) => inventory
                    .warnings
                    .push(format!("{}: {error}", path.display())),
            }
        }
        #[cfg(windows)]
        match windows::bcd_targets() {
            Ok(mut targets) => {
                for target in &mut targets {
                    if let Err(error) = boot_chain::loader(target, &roots) {
                        target.unavailable = Some(error.to_string());
                    }
                }
                inventory.items.extend(targets);
            }
            Err(error) => inventory
                .warnings
                .push(format!("Windows Boot Manager: {error:#}")),
        }
        for root in &roots {
            let path = root.join("EFI/Microsoft/Boot/BCD");
            let bytes = match fs::read(&path) {
                Ok(bytes) => Ok(bytes),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => Err(error),
            };
            match bytes
                .map_err(anyhow::Error::from)
                .and_then(|bytes| bootstore::bcd_targets(&bytes, &path.display().to_string()))
            {
                Ok(mut entries) => {
                    for target in &mut entries.items {
                        if let Err(error) = boot_chain::loader(target, &roots) {
                            target.unavailable = Some(error.to_string());
                        }
                    }
                    inventory.items.extend(entries.items);
                    inventory.warnings.extend(entries.warnings);
                }
                Err(error) => inventory
                    .warnings
                    .push(format!("{}: {error:#}", path.display())),
            }
        }
        inventory
            .items
            .sort_by(|left, right| left.id.cmp(&right.id));
        Ok(inventory)
    }

    fn preflight(&mut self, target: &Target, request: &Request) -> Result<()> {
        #[cfg(windows)]
        windows::require_privilege("SeShutdownPrivilege")?;
        #[cfg(not(windows))]
        {
            let status = fs::read_to_string("/proc/self/status")?;
            ensure!(
                status
                    .lines()
                    .find(|line| line.starts_with("Uid:"))
                    .and_then(|line| line.split_whitespace().nth(2))
                    == Some("0"),
                "Restart requires root. Run this command with sudo."
            );
        }
        #[cfg(windows)]
        if matches!(request.closure, Closure::ForceAfter(_)) {
            windows_escalation::preflight()?;
        }
        #[cfg(not(windows))]
        {
            self.application_grace = linux_shutdown::preflight(&request.closure)?;
        }
        match &target.selection {
            Selection::Firmware(_) => firmware::preflight(),
            Selection::SystemdBoot(_) => {
                firmware::preflight()?;
                boot_chain::loader(target, &self.loader_roots()?)?;
                Ok(())
            }
            #[cfg(windows)]
            Selection::Bcd(_) => {
                windows::powershell("$null=[wmiclass]'root\\WMI:BcdStore'")?;
                boot_chain::loader(target, &self.loader_roots()?)?;
                Ok(())
            }
            Selection::Grub { config, .. } => {
                validate_grub(std::path::Path::new(config))?;
                boot_chain::loader(target, &self.loader_roots()?)?;
                Ok(())
            }
            Selection::OfflineBcd { store, entry } => {
                bootstore::select_bcd(&fs::read(store)?, entry)?;
                let mut options = fs::OpenOptions::new();
                options.read(true).write(true);
                #[cfg(windows)]
                {
                    use std::os::windows::fs::OpenOptionsExt;
                    options.share_mode(0);
                }
                options
                    .open(store)
                    .context("BCD store is busy or not writable")?;
                boot_chain::loader(target, &self.loader_roots()?)?;
                Ok(())
            }
            #[cfg(not(windows))]
            Selection::Bcd(_) => bail!("Live Windows BCD is unavailable on Linux"),
        }
    }

    fn confirm(&mut self, message: &str) -> Result<bool> {
        crate::capability::confirm(self, message)
    }

    fn wait(&mut self, seconds: u32) -> Result<bool> {
        let cancelled = self.cancelled.clone();
        ctrlc::set_handler(move || cancelled.store(true, Ordering::SeqCst))
            .context("Cannot install cancellation handler")?;
        for remaining in (1..=seconds).rev() {
            eprint!("\rRestart starts in {remaining}s. Press Ctrl+C to cancel. ");
            std::io::stderr().flush()?;
            for _ in 0..10 {
                if self.cancelled.load(Ordering::SeqCst) {
                    eprintln!();
                    return Ok(false);
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        }
        if seconds > 0 {
            eprintln!();
        }
        Ok(!self.cancelled.load(Ordering::SeqCst))
    }

    fn plan(&mut self, target: &Target) -> Result<Vec<Change>> {
        self.reboot_lock = Some(reboot_lock::RebootLock::acquire()?);
        let (key, after) = match &target.selection {
            Selection::Firmware(number) => (
                format!("efi:BootNext:{}", firmware::EFI_GUID),
                number.to_le_bytes().to_vec(),
            ),
            Selection::SystemdBoot(entry) => (
                format!("efi:LoaderEntryOneShot:{}", firmware::LOADER_GUID),
                firmware::encode_text(entry),
            ),
            Selection::Bcd(id) => ("bcd:bootsequence".into(), id.as_bytes().to_vec()),
            Selection::Grub { config, entry } => {
                let env = std::path::Path::new(config).with_file_name("grubenv");
                (
                    format!("grub-file:{}", env.display()),
                    bootstore::select_grub(&fs::read(&env)?, entry)?,
                )
            }
            Selection::OfflineBcd { store, entry } => (
                format!("bcd-file:{store}"),
                bootstore::select_bcd(&fs::read(store)?, entry)?,
            ),
        };
        let before = self.read(&key)?;
        let mut changes = vec![Change { key, before, after }];
        let loader = match target.selection {
            Selection::Firmware(_) => None,
            Selection::SystemdBoot(_) => Some("systemd"),
            Selection::Grub { .. } => Some("grub"),
            _ => Some("windows"),
        };
        if loader.is_some() {
            let key = format!("efi:BootNext:{}", firmware::EFI_GUID);
            changes.push(Change {
                before: self.read(&key)?,
                key,
                after: boot_chain::loader(target, &self.loader_roots()?)?
                    .to_le_bytes()
                    .to_vec(),
            });
        }
        Ok(changes)
    }

    fn read(&mut self, key: &str) -> Result<Option<Vec<u8>>> {
        if let Some(path) = key
            .strip_prefix("grub-file:")
            .or_else(|| key.strip_prefix("bcd-file:"))
        {
            return Ok(Some(fs::read(path)?));
        }
        if let Some(key) = key.strip_prefix("efi:") {
            let (name, guid) = key.split_once(':').context("Invalid EFI key")?;
            return firmware::read(name, guid);
        }
        #[cfg(windows)]
        if key == "bcd:bootsequence" {
            return windows::bcd_sequence();
        }
        bail!("Unknown boot selection key")
    }

    fn write(&mut self, key: &str, value: Option<&[u8]>) -> Result<()> {
        if let Some(path) = key.strip_prefix("grub-file:") {
            let bytes = value.context("GRUB environment cannot be removed")?;
            let mut file = fs::OpenOptions::new().read(true).write(true).open(path)?;
            file.try_lock()?;
            ensure!(
                file.metadata()?.len() == bytes.len() as u64,
                "GRUB environment size changed"
            );
            file.write_all(bytes)?;
            file.sync_all()?;
            return Ok(());
        }
        if let Some(path) = key.strip_prefix("bcd-file:") {
            let bytes = value.context("BCD store cannot be removed")?;
            let path = std::path::Path::new(path);
            let mut temporary =
                tempfile::NamedTempFile::new_in(path.parent().context("BCD has no directory")?)?;
            temporary.write_all(bytes)?;
            temporary
                .as_file()
                .set_permissions(fs::metadata(path)?.permissions())?;
            temporary.as_file().sync_all()?;
            temporary.persist(path)?;
            #[cfg(not(windows))]
            fs::File::open(path.parent().context("BCD has no directory")?)?.sync_all()?;
            return Ok(());
        }
        if let Some(key) = key.strip_prefix("efi:") {
            let (name, guid) = key.split_once(':').context("Invalid EFI key")?;
            return firmware::write(name, guid, value);
        }
        #[cfg(windows)]
        if key == "bcd:bootsequence" {
            return windows::set_bcd_sequence(value);
        }
        bail!("Unknown boot selection key")
    }

    fn shutdown(&mut self, closure: &Closure) -> Result<()> {
        #[cfg(windows)]
        {
            windows::shutdown(closure)
        }
        #[cfg(not(windows))]
        {
            linux_shutdown::shutdown(closure)
        }
    }
}

impl crate::capability::Confirmation for Host {
    fn dialog(&mut self, message: &str) -> Option<bool> {
        #[cfg(windows)]
        {
            windows::confirmation(message)
        }
        #[cfg(not(windows))]
        {
            if std::env::var_os("DISPLAY").is_some()
                || std::env::var_os("WAYLAND_DISPLAY").is_some()
            {
                for (name, args) in [
                    (
                        "zenity",
                        vec![
                            "--question",
                            "--default-cancel",
                            "--title=Boothop",
                            "--text",
                            message,
                        ],
                    ),
                    (
                        "kdialog",
                        vec!["--defaultno", "--yesno", message, "--title", "Boothop"],
                    ),
                ] {
                    if let Some(program) = find(name) {
                        match run(&program, &args, Duration::from_secs(300)) {
                            Ok(result) if result.code == 0 => return Some(true),
                            Ok(result) if result.code == 1 => return Some(false),
                            _ => continue,
                        }
                    }
                }
            }
            None
        }
    }
    fn interactive(&self) -> bool {
        std::io::stdin().is_terminal() && std::io::stderr().is_terminal()
    }
    fn terminal(&mut self, message: &str) -> Result<bool> {
        self.terminal_confirmation(message)
    }
}

impl System for Host {
    fn platform(&self) -> Platform {
        if cfg!(windows) {
            Platform::Windows
        } else {
            Platform::Linux
        }
    }
    fn disks(&mut self, check: bool) -> Result<Inventory<Disk>> {
        storage::inventory(check)
    }
    fn diagnose(&mut self, detail: u8) -> Vec<String> {
        let mut lines = self.help_warnings();
        match self.discover() {
            Ok(inventory) => {
                lines.push(format!(
                    "Boot targets: {} discovered",
                    inventory.items.len()
                ));
                lines.extend(inventory.warnings);
                for target in inventory
                    .items
                    .iter()
                    .filter(|target| target.unavailable.is_some())
                {
                    if let Some(reason) = &target.unavailable {
                        lines.push(format!("Target {}: unavailable: {reason}", target.id));
                    }
                }
            }
            Err(error) => lines.push(format!("Boot discovery: {error:#}")),
        }
        #[cfg(windows)]
        {
            for privilege in ["SeShutdownPrivilege", "SeSystemEnvironmentPrivilege"] {
                lines.push(match windows::require_privilege(privilege) {
                    Ok(()) => format!("{privilege}: available"),
                    Err(error) => format!("{privilege}: {error:#}"),
                });
            }
            lines.push(match windows_escalation::preflight() {
                Ok(()) => "Shutdown: graceful, forced, and service-owned application grace period available".into(),
                Err(error) => format!("Shutdown: graceful and forced closure available; application grace period: {error:#}"),
            });
        }
        #[cfg(not(windows))]
        for (name, closure) in [
            ("Graceful restart", Closure::Graceful),
            ("Forced restart", Closure::Force),
            ("Timed escalation", Closure::ForceAfter(60)),
        ] {
            lines.push(match linux_shutdown::preflight(&closure) {
                Ok(application_grace) => format!(
                    "{name}: mechanism available (requires root){}",
                    if application_grace {
                        "; application grace period before service shutdown"
                    } else {
                        ""
                    }
                ),
                Err(error) => format!("{name}: unavailable: {error:#}"),
            });
        }
        match storage::inventory(false) {
            Ok(disks) => lines.push(format!("Disk inventory: {} disks and volumes; raw NTFS inspection requires elevated read access", disks.items.len())),
            Err(error) => lines.push(format!("Disk inventory: {error:#}")),
        }
        #[cfg(windows)]
        lines.push(
            if windows::desktop_available() {
                "Confirmation: native Windows dialog available"
            } else {
                "Confirmation: no interactive desktop; terminal or --yes required"
            }
            .into(),
        );
        #[cfg(not(windows))]
        lines.push(
            if (std::env::var_os("DISPLAY").is_some()
                || std::env::var_os("WAYLAND_DISPLAY").is_some())
                && (find("zenity").is_some() || find("kdialog").is_some())
            {
                "Confirmation: desktop dialog candidate found"
            } else {
                "Confirmation: no desktop dialog found; terminal or --yes required"
            }
            .into(),
        );
        lines.push(
            "Bootloader files must be accessible through an existing filesystem mount.".into(),
        );
        if detail > 0 {
            lines.push(format!("Platform: {} x86_64", std::env::consts::OS));
        }
        if detail > 1 {
            lines.push("Selection uses one-shot boot settings; doctor does not write or test those settings.".into());
        }
        lines
    }
    fn help_warnings(&mut self) -> Vec<String> {
        let mut warnings = Vec::new();
        #[cfg(windows)]
        if find("powershell.exe").is_none() {
            warnings.push("Disk and BCD inventory require Windows PowerShell. Restore the Windows PowerShell system component.".into());
        }
        #[cfg(not(windows))]
        {
            if find("lsblk").is_none() {
                warnings.push(
                    "Disk inventory requires lsblk. Install the distribution's util-linux package."
                        .into(),
                );
            }
            if !Path::new("/sys/firmware/efi/efivars").is_dir() {
                warnings.push(
                    "UEFI variables are unavailable. Boot in UEFI mode with efivarfs mounted."
                        .into(),
                );
            }
            if !(find("systemctl").is_some() && Path::new("/run/systemd/system").exists())
                && find("shutdown").is_none()
                && find("openrc-shutdown").is_none()
                && !(Path::new("/run/openrc/softlevel").exists() && find("reboot").is_some())
            {
                warnings.push(
                    "Restart requires a systemd or OpenRC-compatible shutdown utility.".into(),
                );
            }
            if find("findmnt").is_none() {
                warnings.push(
                    "Bootloader path validation requires findmnt. Install util-linux.".into(),
                );
            }
        }
        warnings
    }
}

impl Transport for Host {
    fn get(&mut self, url: &str, limit: usize) -> Result<Response> {
        ensure!(url.starts_with("https://"), "HTTPS is required");
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .https_only(true)
            .timeout_global(Some(Duration::from_secs(60)))
            .http_status_as_error(false)
            .build()
            .into();
        let mut response = agent
            .get(url)
            .header("User-Agent", concat!("boothop/", env!("CARGO_PKG_VERSION")))
            .header("Accept", "application/vnd.github+json")
            .call()?;
        let status = response.status().as_u16();
        let body = response
            .body_mut()
            .with_config()
            .limit(limit as u64)
            .read_to_vec()?;
        Ok(Response { status, body })
    }
}

impl Installer for Host {
    fn replace(&mut self, executable: &[u8]) -> Result<String> {
        let destination = std::env::current_exe()?.canonicalize()?;
        crate::installation::replace_using(&destination, executable, |replacement| {
            self_replace::self_replace(replacement).map_err(Into::into)
        })
    }
}
