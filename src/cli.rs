use crate::{
    boot::{Inventory, Target},
    disk::Disk,
    reboot::{self, Closure, RebootSystem, Request},
    update::{self, Installer, Platform, Transport},
};
use anyhow::Result;
use clap::{Arg, ArgAction, Command, error::ErrorKind};
use std::ffi::OsString;

pub trait System: RebootSystem + Transport + Installer {
    fn platform(&self) -> Platform;
    fn disks(&mut self, check_ntfs: bool) -> Result<Inventory<Disk>>;
    fn diagnose(&mut self, detail: u8) -> Vec<String>;
    fn help_warnings(&mut self) -> Vec<String>;
}

#[derive(Debug, Default)]
pub struct Output {
    pub stdout: String,
    pub stderr: String,
    pub code: u8,
}

fn verbose() -> Arg {
    Arg::new("verbose")
        .long("verbose")
        .short('v')
        .action(ArgAction::Count)
        .help("Show details (-v) or diagnostics (-vv)")
}

pub fn command() -> Command {
    Command::new("boothop")
        .version(env!("CARGO_PKG_VERSION"))
        .about("Discover boot systems and restart into a selected system")
        .subcommand(
            Command::new("systems")
                .about("List boot targets")
                .arg(verbose()),
        )
        .subcommand(
            Command::new("disks")
                .about("List disks and volumes")
                .arg(verbose())
                .arg(
                    Arg::new("check-ntfs")
                        .long("check-ntfs")
                        .action(ArgAction::SetTrue)
                        .help("Check NTFS hibernation without changing volumes"),
                ),
        )
        .subcommand(
            Command::new("doctor")
                .about("Check system readiness and updates")
                .arg(verbose()),
        )
        .subcommand(
            Command::new("update")
                .about("Check GitHub for a newer stable version")
                .arg(
                    Arg::new("install")
                        .long("install")
                        .action(ArgAction::SetTrue)
                        .help("Download and replace the installed executable"),
                ),
        )
        .subcommand(
            Command::new("reboot")
                .about("Restart into a target from 'systems' for the next boot only")
                .arg(
                    Arg::new("id")
                        .required(true)
                        .help("Target ID from 'boothop systems'"),
                )
                .arg(
                    Arg::new("timeout")
                        .long("timeout")
                        .value_name("SECONDS")
                        .default_value("0")
                        .value_parser(clap::value_parser!(u32))
                        .help("Delay before starting shutdown"),
                )
                .arg(
                    Arg::new("force")
                        .long("force")
                        .action(ArgAction::SetTrue)
                        .conflicts_with("force-after")
                        .help("Force applications to close through the OS"),
                )
                .arg(
                    Arg::new("force-after")
                        .long("force-after")
                        .value_name("SECONDS")
                        .value_parser(clap::value_parser!(u32).range(1..))
                        .help("Allow graceful application closure for SECONDS before forcing"),
                )
                .arg(
                    Arg::new("yes")
                        .long("yes")
                        .short('y')
                        .action(ArgAction::SetTrue)
                        .help("Skip restart confirmation"),
                ),
        )
}

pub fn run(args: impl IntoIterator<Item = OsString>, system: &mut impl System) -> Output {
    let mut cmd = command();
    let matches = match cmd.try_get_matches_from_mut(args) {
        Ok(matches) => matches,
        Err(error) => {
            let help = error.kind() == ErrorKind::DisplayHelp;
            let successful = help || error.kind() == ErrorKind::DisplayVersion;
            return Output {
                stdout: if successful {
                    error.to_string()
                } else {
                    String::new()
                },
                stderr: if help {
                    warnings(system.help_warnings())
                } else if successful {
                    String::new()
                } else {
                    error.to_string()
                },
                code: if successful { 0 } else { 2 },
            };
        }
    };
    let Some((name, options)) = matches.subcommand() else {
        return Output {
            stdout: format!("{}\n", cmd.render_long_help()),
            stderr: warnings(system.help_warnings()),
            code: 0,
        };
    };
    match dispatch(name, options, system) {
        Ok(output) => output,
        Err(error) => Output {
            stdout: String::new(),
            stderr: format!("Error: {error:#}\n"),
            code: 1,
        },
    }
}

fn dispatch(name: &str, options: &clap::ArgMatches, system: &mut impl System) -> Result<Output> {
    let mut output = Output::default();
    match name {
        "systems" => {
            let inventory = system.discover()?;
            output.stdout = format_targets(&inventory.items, options.get_count("verbose"));
            output.stderr = warnings(inventory.warnings);
        }
        "disks" => {
            let inventory = system.disks(options.get_flag("check-ntfs"))?;
            for disk in &inventory.items {
                output.stdout.push_str(&format!(
                    "{}  {}  {} bytes  {}",
                    clean(&disk.id),
                    clean(disk.parent.as_deref().unwrap_or("disk")),
                    disk.size,
                    clean(&disk.filesystem)
                ));
                if let Some(state) = &disk.hibernation {
                    output
                        .stdout
                        .push_str(&format!("  {}", clean(&state.to_string())));
                }
                if options.get_count("verbose") > 0 {
                    output.stdout.push_str(&format!(
                        "\n  label: {}  mount: {}",
                        clean(&disk.label),
                        clean(&disk.mount)
                    ));
                }
                if options.get_count("verbose") > 1 && disk.hibernation.is_some() {
                    output
                        .stdout
                        .push_str("\n  NTFS probe: read-only metadata inspection");
                }
                output.stdout.push('\n');
            }
            if inventory.items.is_empty() {
                output.stdout.push_str("No disks found.\n");
            }
            output.stderr = warnings(inventory.warnings);
        }
        "reboot" => {
            let closure = if options.get_flag("force") {
                Closure::Force
            } else if let Some(seconds) = options.get_one::<u32>("force-after") {
                Closure::ForceAfter(*seconds)
            } else {
                Closure::Graceful
            };
            let request = Request {
                id: options.get_one::<String>("id").cloned().unwrap_or_default(),
                delay: options
                    .get_one::<u32>("timeout")
                    .copied()
                    .unwrap_or_default(),
                closure,
                yes: options.get_flag("yes"),
            };
            output.stdout = format!("{}\n", reboot::execute(system, &request)?);
        }
        "doctor" => {
            output.stdout = format!(
                "{}\n",
                system.diagnose(options.get_count("verbose")).join("\n")
            );
            let platform = system.platform();
            match update::check(system, platform, env!("CARGO_PKG_VERSION")) {
                Ok(status) => output.stdout.push_str(&format!("Updates: {status}\n")),
                Err(error) => output
                    .stderr
                    .push_str(&format!("Updates undetermined: {error:#}\n")),
            }
        }
        "update" => {
            let platform = system.platform();
            let status = update::check(system, platform, env!("CARGO_PKG_VERSION"))?;
            update::require_available(&status)?;
            if options.get_flag("install") {
                // Separate sequential borrows by adapting the prepared executable.
                let mut capture = PreparedUpdate(None);
                let result = update::install(system, &mut capture, platform, &status)?;
                output.stdout = format!(
                    "{}\n",
                    match capture.0 {
                        Some(bytes) => system.replace(&bytes)?,
                        None => result,
                    }
                );
            } else {
                output.stdout = format!("{status}\n");
            }
        }
        _ => unreachable!("clap only accepts declared commands"),
    }
    Ok(output)
}

struct PreparedUpdate(Option<Vec<u8>>);
impl Installer for PreparedUpdate {
    fn replace(&mut self, bytes: &[u8]) -> Result<String> {
        self.0 = Some(bytes.to_vec());
        Ok(String::new())
    }
}

fn warnings(items: Vec<String>) -> String {
    items
        .into_iter()
        .map(|item| format!("Warning: {}\n", clean(&item)))
        .collect()
}
fn clean(value: &str) -> String {
    value
        .chars()
        .filter(|character| !character.is_control())
        .collect()
}
fn format_targets(targets: &[Target], detail: u8) -> String {
    if targets.is_empty() {
        return "No boot targets found.\n".into();
    }
    let mut result = String::new();
    for target in targets {
        result.push_str(&format!(
            "{}  {}{}\n",
            clean(&target.id),
            clean(&target.label),
            if target.current { "  (current)" } else { "" }
        ));
        if let Some(reason) = &target.unavailable {
            result.push_str(&format!("  Unavailable: {}\n", clean(reason)));
        }
        if detail > 0 {
            let source = match target.selection {
                crate::boot::Selection::Firmware(_) => "UEFI",
                crate::boot::Selection::SystemdBoot(_) => "systemd-boot",
                crate::boot::Selection::Grub { .. } => "GRUB",
                crate::boot::Selection::Bcd(_) | crate::boot::Selection::OfflineBcd { .. } => {
                    "Windows Boot Manager"
                }
            };
            result.push_str(&format!("  Bootloader: {source}\n"));
            result.push_str(&format!("  Location: {}\n", clean(&target.location)));
        }
        if detail > 1 {
            result.push_str(&format!("  Selection: {:?}\n", target.selection));
        }
    }
    result
}
