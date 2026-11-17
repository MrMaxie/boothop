#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(windows)]
fn main() -> std::process::ExitCode {
    let result = (|| -> anyhow::Result<()> {
        use anyhow::{Context, bail};
        let launcher = std::env::current_exe()?;
        let args = std::env::args_os().skip(1).collect();
        let output = boothop::launcher::command(&launcher, args)?
            .output()
            .context("Cannot start boothop.exe. Keep it in the same folder as this launcher.")?;
        if !output.status.success() {
            let message = String::from_utf8_lossy(&output.stderr);
            if message.trim().is_empty() {
                bail!(
                    "Boothop exited without completing the restart request ({}).",
                    output.status
                );
            }
            bail!("{}", message.trim());
        }
        Ok(())
    })();
    match result {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            boothop::windows_dialog::error(&format!("{error:#}"));
            std::process::ExitCode::FAILURE
        }
    }
}

#[cfg(not(windows))]
fn main() -> std::process::ExitCode {
    eprintln!("This shortcut launcher is only available on Windows. Use boothop reboot <id>.");
    std::process::ExitCode::FAILURE
}
