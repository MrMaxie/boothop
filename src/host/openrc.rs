use super::{
    linux_shutdown,
    process::{find, run_env},
};
use crate::{
    reboot::{Closure, ShutdownUncertain},
    shutdown::{self, ProcessGraceState, ProcessGraceSystem, UserProcess},
};
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use std::{
    ffi::OsString,
    fs,
    io::Write,
    os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

const NAME: &str = "boothop-shutdown";
const DIRECTORY: &str = "/run/boothop-shutdown";
const ENVIRONMENT: &[(&str, &str)] = &[("RC_SVCNAME", NAME)];

#[derive(Serialize, Deserialize)]
struct Plan {
    seconds: u32,
    program: PathBuf,
    arguments: Vec<String>,
}
#[derive(Serialize, Deserialize)]
struct Failure {
    uncertain: bool,
    reason: String,
}

pub fn preflight() -> Result<()> {
    ensure!(
        Path::new("/run/openrc/softlevel").exists(),
        "Application grace requires OpenRC"
    );
    ensure!(
        !Path::new(DIRECTORY).exists(),
        "A previous OpenRC shutdown helper exists. Inspect /run/boothop-shutdown and its supervisor before retrying"
    );
    linux_shutdown::graceful_command()?;
    let supervisor = find("supervise-daemon").context("OpenRC forced closure requires supervise-daemon. Install the distribution's OpenRC supervisor package")?;
    let help = run_env(
        &supervisor,
        &[NAME, "--help"],
        Duration::from_secs(5),
        ENVIRONMENT,
    )?;
    let help = format!("{}{}", help.stdout, help.stderr);
    ensure!(
        help.contains("--respawn-max") && help.contains("--respawn-period"),
        "Installed OpenRC supervisor lacks the required lifecycle controls"
    );
    // Opening a pidfd is read-only. Requiring it avoids signals to recycled PIDs.
    let descriptor = unsafe { libc::syscall(libc::SYS_pidfd_open, std::process::id(), 0) };
    ensure!(
        descriptor >= 0,
        "OpenRC application grace requires Linux pidfd support (kernel 5.3 or newer)"
    );
    unsafe {
        libc::close(descriptor as i32);
    }
    Ok(())
}

pub fn start(closure: &Closure) -> Result<()> {
    let seconds = match closure {
        Closure::Force => 0,
        Closure::ForceAfter(seconds) => *seconds,
        Closure::Graceful => bail!("Graceful restart does not need a supervisor"),
    };
    let (program, arguments) = linux_shutdown::graceful_command()?;
    let supervisor = find("supervise-daemon").context("OpenRC supervisor unavailable")?;
    fs::DirBuilder::new().mode(0o700).create(DIRECTORY)?;
    let preparation = (|| -> Result<()> {
        let worker = Path::new(DIRECTORY).join("worker");
        fs::copy(std::env::current_exe()?, &worker)?;
        fs::set_permissions(worker, fs::Permissions::from_mode(0o700))?;
        persist(
            "request.json",
            &Plan {
                seconds,
                program,
                arguments,
            },
        )
    })();
    if let Err(error) = preparation {
        cleanup();
        return Err(error);
    }
    let result = run_env(
        &supervisor,
        &[
            NAME,
            "--start",
            "--pidfile",
            "/run/boothop-shutdown/supervisor.pid",
            "--respawn-max",
            "1",
            "--respawn-period",
            "0",
            "--respawn-delay",
            "0",
            "--stdin",
            "/dev/null",
            "--stdout",
            "/run/boothop-shutdown/stdout",
            "--stderr",
            "/run/boothop-shutdown/stderr",
            "/run/boothop-shutdown/worker",
            "--",
            "__boothop-openrc",
        ],
        Duration::from_secs(10),
        ENVIRONMENT,
    );
    let result = result.map_err(|error| {
        ShutdownUncertain(format!(
            "OpenRC supervisor submission is uncertain: {error:#}. Inspect {DIRECTORY}"
        ))
    })?;
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Ok(bytes) = fs::read(Path::new(DIRECTORY).join("failure.json")) {
            let failure: Failure = serde_json::from_slice(&bytes)?;
            if failure.uncertain {
                return Err(ShutdownUncertain(failure.reason).into());
            }
            let stopped = run_env(
                &supervisor,
                &[
                    NAME,
                    "--stop",
                    "--pidfile",
                    "/run/boothop-shutdown/supervisor.pid",
                    "/run/boothop-shutdown/worker",
                ],
                Duration::from_secs(5),
                ENVIRONMENT,
            );
            if stopped.is_ok_and(|result| result.code == 0) {
                cleanup();
            }
            bail!(
                "OpenRC shutdown helper rejected the request: {}",
                failure.reason
            );
        }
        if matches!(
            fs::read_to_string(Path::new(DIRECTORY).join("accepted")).as_deref(),
            Ok("accepted")
        ) {
            return Ok(());
        }
        if result.code != 0 || Instant::now() >= deadline {
            return Err(ShutdownUncertain(format!(
                "OpenRC helper acceptance is unknown: {}. Inspect {DIRECTORY}",
                result.stderr
            ))
            .into());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

pub fn dispatch(args: &[OsString]) -> Option<Result<()>> {
    if args.get(1).and_then(|value| value.to_str()) != Some("__boothop-openrc") {
        return None;
    }
    Some(worker())
}

fn worker() -> Result<()> {
    ensure!(
        unsafe { libc::geteuid() } == 0,
        "The OpenRC worker requires root"
    );
    let directory = fs::symlink_metadata(DIRECTORY)?;
    ensure!(
        directory.is_dir() && directory.uid() == 0 && directory.permissions().mode() & 0o077 == 0,
        "Untrusted shutdown state directory"
    );
    ensure!(
        std::env::current_exe()?.canonicalize()? == Path::new(DIRECTORY).join("worker"),
        "Worker was not started from the protected operation directory"
    );
    ensure!(
        !Path::new(DIRECTORY).join("failure.json").exists(),
        "Previous operation failed; refusing to repeat application closure"
    );
    let result = (|| -> Result<()> {
        let plan: Plan =
            serde_json::from_slice(&fs::read(Path::new(DIRECTORY).join("request.json"))?)?;
        ensure!(
            (plan.program.clone(), plan.arguments.clone()) == linux_shutdown::graceful_command()?,
            "Shutdown mechanism changed after preflight"
        );
        let previous = match fs::read(Path::new(DIRECTORY).join("state.json")) {
            Ok(bytes) => Some(serde_json::from_slice::<ProcessGraceState>(&bytes)?),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(error.into()),
        };
        let seconds = plan.seconds;
        let mut system = NativeGrace {
            plan,
            boot_ms: boot_time_ms()?,
            started: Instant::now(),
        };
        shutdown::process_grace_restart(&mut system, seconds, previous)
    })();
    if let Err(error) = &result {
        persist(
            "failure.json",
            &Failure {
                uncertain: error.downcast_ref::<ShutdownUncertain>().is_some(),
                reason: format!("{error:#}"),
            },
        )?;
    }
    // Successful operation state lives under /run and disappears at restart.
    // A restart of this worker sees submitted=true and cannot repeat the request.
    result
}

fn persist(name: &str, value: &impl Serialize) -> Result<()> {
    let mut temporary = tempfile::NamedTempFile::new_in(DIRECTORY)?;
    temporary.write_all(&serde_json::to_vec(value)?)?;
    temporary.as_file().sync_all()?;
    temporary.persist(Path::new(DIRECTORY).join(name))?;
    fs::File::open(DIRECTORY)?.sync_all()?;
    Ok(())
}

fn cleanup() {
    // Remove only this operation's fixed files; never recursively remove /run.
    for name in [
        "request.json",
        "state.json",
        "failure.json",
        "accepted",
        "worker",
        "supervisor.pid",
        "stdout",
        "stderr",
    ] {
        let _ = fs::remove_file(Path::new(DIRECTORY).join(name));
    }
    let _ = fs::remove_dir(DIRECTORY);
}

fn boot_time_ms() -> Result<u64> {
    let mut time = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    ensure!(
        unsafe { libc::clock_gettime(libc::CLOCK_BOOTTIME, &mut time) } == 0,
        "Cannot read the boot clock"
    );
    Ok(time.tv_sec as u64 * 1000 + time.tv_nsec as u64 / 1_000_000)
}

fn process(pid: i32) -> Result<UserProcess> {
    let directory = PathBuf::from(format!("/proc/{pid}"));
    shutdown::process_record(
        pid,
        &fs::read_to_string(directory.join("stat"))?,
        &fs::read_to_string(directory.join("status"))?,
        &fs::read_to_string(directory.join("loginuid")).unwrap_or_default(),
    )
}

struct NativeGrace {
    plan: Plan,
    boot_ms: u64,
    started: Instant,
}
impl ProcessGraceSystem for NativeGrace {
    fn processes(&mut self) -> Result<Vec<UserProcess>> {
        let mut processes = Vec::new();
        for entry in fs::read_dir("/proc")? {
            let entry = entry?;
            let Ok(pid) = entry.file_name().to_string_lossy().parse::<i32>() else {
                continue;
            };
            match process(pid) {
                Ok(process) => processes.push(process),
                Err(error)
                    if error
                        .downcast_ref::<std::io::Error>()
                        .is_some_and(|error| error.kind() == std::io::ErrorKind::NotFound) => {}
                Err(error) => return Err(error).context("Cannot inspect user processes safely"),
            }
        }
        let definitions = fs::read_to_string("/etc/login.defs").unwrap_or_default();
        let minimum = definitions
            .lines()
            .find_map(|line| {
                let mut fields = line.split_whitespace();
                (fields.next() == Some("UID_MIN"))
                    .then(|| fields.next()?.parse::<u32>().ok())
                    .flatten()
            })
            .unwrap_or(1000)
            .max(100);
        Ok(shutdown::user_processes(processes, minimum))
    }
    fn now_ms(&self) -> u64 {
        self.boot_ms
            .saturating_add(self.started.elapsed().as_millis() as u64)
    }
    fn persist(&mut self, state: &ProcessGraceState) -> Result<()> {
        persist("state.json", state)
    }
    fn signal(&mut self, expected: &UserProcess, force: bool) -> Result<()> {
        let descriptor = unsafe { libc::syscall(libc::SYS_pidfd_open, expected.pid, 0) } as i32;
        if descriptor < 0 {
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() == Some(libc::ESRCH) {
                return Ok(());
            }
            return Err(error.into());
        }
        let result = (|| -> Result<()> {
            let current = match process(expected.pid) {
                Ok(process) => process,
                Err(error)
                    if error
                        .downcast_ref::<std::io::Error>()
                        .is_some_and(|error| error.kind() == std::io::ErrorKind::NotFound) =>
                {
                    return Ok(());
                }
                Err(error) => return Err(error),
            };
            if !shutdown::same_process(expected, &current) {
                return Ok(());
            }
            let signal = if force { libc::SIGKILL } else { libc::SIGTERM };
            if unsafe {
                libc::syscall(
                    libc::SYS_pidfd_send_signal,
                    descriptor,
                    signal,
                    std::ptr::null::<libc::siginfo_t>(),
                    0,
                )
            } < 0
            {
                let error = std::io::Error::last_os_error();
                if error.raw_os_error() != Some(libc::ESRCH) {
                    return Err(error.into());
                }
            }
            Ok(())
        })();
        unsafe {
            libc::close(descriptor);
        }
        result
    }
    fn accepted(&mut self) -> Result<()> {
        fs::write(Path::new(DIRECTORY).join("accepted"), "accepted").map_err(Into::into)
    }
    fn wait_until(&mut self, deadline_ms: u64) {
        while self.now_ms() < deadline_ms {
            std::thread::sleep(Duration::from_millis(
                deadline_ms.saturating_sub(self.now_ms()).min(100),
            ));
        }
    }
    fn restart(&mut self) -> Result<()> {
        let result = super::process::run(
            &self.plan.program,
            &self
                .plan
                .arguments
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            Duration::from_secs(10),
        )
        .map_err(|error| {
            ShutdownUncertain(format!("OpenRC restart submission is uncertain: {error:#}"))
        })?;
        ensure!(
            result.code == 0,
            "OpenRC restart rejected: {}",
            result.stderr
        );
        Ok(())
    }
}
