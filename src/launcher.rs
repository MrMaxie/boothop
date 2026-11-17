//! Builds a shortcut invocation without starting a process.
use anyhow::{Result, ensure};
use std::{
    ffi::OsString,
    path::Path,
    process::{Command, Stdio},
};

pub fn command(launcher: &Path, args: Vec<OsString>) -> Result<Command> {
    ensure!(
        args.first().is_some_and(|arg| arg == "reboot"),
        "Use this launcher with: reboot <id> [options]"
    );
    let mut command = Command::new(launcher.with_file_name("boothop.exe"));
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(windows_sys::Win32::System::Threading::CREATE_NO_WINDOW);
    }
    Ok(command)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forwards_exact_arguments_to_the_sibling_executable_without_approving() {
        let args = ["reboot", "uefi:0002", "--timeout", "10"]
            .map(OsString::from)
            .to_vec();
        let command = command(Path::new("some folder/boothop-launcher.exe"), args.clone()).unwrap();
        assert_eq!(command.get_program(), Path::new("some folder/boothop.exe"));
        assert_eq!(command.get_args().collect::<Vec<_>>(), args);
    }

    #[test]
    fn rejects_missing_or_unrelated_commands_without_starting_anything() {
        for args in [vec![], vec![OsString::from("update")]] {
            assert!(command(Path::new("boothop-launcher.exe"), args).is_err());
        }
    }
}
