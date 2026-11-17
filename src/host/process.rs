use anyhow::{Context, Result, bail, ensure};
use std::{
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

pub struct ProcessOutput {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

pub fn find(name: &str) -> Option<PathBuf> {
    #[cfg(windows)]
    let directories = std::env::var_os("SystemRoot")
        .map(PathBuf::from)
        .map(|root| {
            vec![
                root.join("System32"),
                root.join("System32/WindowsPowerShell/v1.0"),
            ]
        })
        .unwrap_or_default();
    #[cfg(not(windows))]
    let directories = ["/usr/sbin", "/usr/bin", "/sbin", "/bin"]
        .iter()
        .map(PathBuf::from)
        .collect::<Vec<_>>();
    directories
        .into_iter()
        .map(|directory| directory.join(name))
        .find(|path| path.is_file())
}

pub fn run(program: &Path, args: &[&str], timeout: Duration) -> Result<ProcessOutput> {
    run_env(program, args, timeout, &[])
}

pub fn run_env(
    program: &Path,
    args: &[&str],
    timeout: Duration,
    environment: &[(&str, &str)],
) -> Result<ProcessOutput> {
    let mut command = Command::new(program);
    command.envs(environment.iter().copied());
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let mut child = command
        .spawn()
        .with_context(|| format!("Cannot start {}", program.display()))?;
    let stdout = child.stdout.take().context("Missing stdout pipe")?;
    let stderr = child.stderr.take().context("Missing stderr pipe")?;
    let (out_tx, out_rx) = std::sync::mpsc::channel();
    let (err_tx, err_rx) = std::sync::mpsc::channel();
    thread::spawn(move || {
        let _ = out_tx.send(capture(stdout));
    });
    thread::spawn(move || {
        let _ = err_tx.send(capture(stderr));
    });
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if started.elapsed() >= timeout {
            let _ = child.kill();
            let _ = child.wait();
            // Drop reader handles instead of joining inherited descendant pipes.
            bail!("{} exceeded its time limit", program.display());
        }
        thread::sleep(Duration::from_millis(25));
    };
    let stdout = out_rx
        .recv_timeout(timeout.saturating_sub(started.elapsed()))
        .context("Output capture timed out")??;
    let stderr = err_rx
        .recv_timeout(timeout.saturating_sub(started.elapsed()))
        .context("Error capture timed out")??;
    Ok(ProcessOutput {
        code: status.code().unwrap_or(-1),
        stdout,
        stderr,
    })
}

fn capture(mut reader: impl Read) -> Result<String> {
    let mut output = Vec::new();
    let mut buffer = [0; 8192];
    let mut overflow = false;
    loop {
        let count = reader.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        if output.len() + count <= 4 * 1024 * 1024 {
            output.extend_from_slice(&buffer[..count]);
        } else {
            overflow = true;
        }
    }
    ensure!(!overflow, "Process output exceeded 4 MiB");
    String::from_utf8(output).context("Process output is not UTF-8")
}
