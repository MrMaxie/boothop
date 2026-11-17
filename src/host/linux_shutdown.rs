use super::process::{find, run};
use crate::{
    reboot::{Closure, ShutdownUncertain},
    shutdown,
};
use anyhow::{Context, Result, bail, ensure};
use std::{
    fs,
    path::Path,
    time::{Duration, Instant},
};

pub fn preflight(closure: &Closure) -> Result<bool> {
    if Path::new("/run/systemd/system").exists() {
        let systemctl = find("systemctl").context("systemctl is unavailable")?;
        let result = run(
            &systemctl,
            &["show", "--property=Version", "--value"],
            Duration::from_secs(5),
        )?;
        ensure!(
            result.code == 0,
            "Cannot contact the system service manager: {}",
            result.stderr
        );
        if matches!(closure, Closure::ForceAfter(_)) {
            let version = result
                .stdout
                .trim()
                .split(|c: char| !c.is_ascii_digit())
                .next()
                .unwrap_or_default()
                .parse::<u32>()
                .unwrap_or_default();
            ensure!(
                version >= 240,
                "Timed escalation requires systemd 240 or newer"
            );
            let pending = run(
                &systemctl,
                &[
                    "show",
                    "boothop-shutdown.service",
                    "--property=ActiveState",
                    "--value",
                ],
                Duration::from_secs(5),
            )?;
            ensure!(
                pending.code == 0 && ["inactive", "failed"].contains(&pending.stdout.trim()),
                "A previous shutdown service is active or its state could not be established"
            );
            for program in ["systemd-run", "sh", "sleep"] {
                find(program).with_context(|| format!("Timed escalation requires {program}"))?;
            }
        }
    } else if *closure == Closure::Graceful {
        graceful_command()?;
    } else if sysv_backend().is_ok() {
        shutdown::sysv_arguments(closure)?;
    } else {
        super::openrc::preflight()?;
        return Ok(true);
    }
    Ok(false)
}

pub fn shutdown(closure: &Closure) -> Result<()> {
    if let Closure::ForceAfter(seconds) = closure
        && Path::new("/run/systemd/system").exists()
    {
        return escalate(*seconds);
    }
    let (program, args) = if Path::new("/run/systemd/system").exists() {
        (
            find("systemctl").context("systemctl unavailable")?,
            shutdown::systemd_arguments(closure)?
                .into_iter()
                .map(str::to_owned)
                .collect::<Vec<_>>(),
        )
    } else if *closure != Closure::Graceful {
        if let Ok(backend) = sysv_backend() {
            (backend, shutdown::sysv_arguments(closure)?)
        } else {
            return super::openrc::start(closure);
        }
    } else {
        graceful_command()?
    };
    let result = run(
        &program,
        &args.iter().map(String::as_str).collect::<Vec<_>>(),
        Duration::from_secs(10),
    )
    .map_err(|error| ShutdownUncertain(format!("Shutdown transport failed: {error:#}")))?;
    ensure!(result.code == 0, "Shutdown rejected: {}", result.stderr);
    Ok(())
}

pub fn graceful_command() -> Result<(std::path::PathBuf, Vec<String>)> {
    let init = fs::canonicalize("/proc/1/exe")?;
    if init.file_name().is_some_and(|name| name == "busybox") {
        ensure!(
            Path::new("/run/openrc/softlevel").exists(),
            "BusyBox restart requires an OpenRC shutdown integration"
        );
        let inittab = fs::read_to_string("/etc/inittab")?;
        ensure!(
            inittab
                .lines()
                .filter(|line| !line.trim_start().starts_with('#'))
                .any(|line| {
                    let fields = line.splitn(4, ':').collect::<Vec<_>>();
                    fields.len() == 4
                        && fields[2] == "shutdown"
                        && shell_words::split(fields[3]).is_ok_and(|args| {
                            args.len() == 2 && args[0].ends_with("/openrc") && args[1] == "shutdown"
                        })
                }),
            "Cannot verify BusyBox's normal OpenRC shutdown hook"
        );
        let reboot = find("reboot").context("BusyBox reboot applet is unavailable")?;
        ensure!(
            fs::canonicalize(&reboot)? == init,
            "Reboot applet does not match the running BusyBox init"
        );
        return Ok((reboot, Vec::new()));
    }
    let candidates: &[&str] = if init.file_name().is_some_and(|name| name == "openrc-init") {
        &["openrc-shutdown"]
    } else {
        &["shutdown", "openrc-shutdown"]
    };
    crate::capability::first_usable(candidates, |name| {
        let program =
            find(name).with_context(|| format!("Install {name} for the running init system"))?;
        let help = run(&program, &["--help"], Duration::from_secs(5))?;
        ensure!(
            format!("{}{}", help.stdout, help.stderr).contains("-r"),
            "Shutdown utility does not advertise reboot support"
        );
        Ok((program, vec!["-r".into(), "now".into()]))
    })
}

fn sysv_backend() -> Result<std::path::PathBuf> {
    ensure!(
        Path::new("/run/openrc/softlevel").exists(),
        "No supported OpenRC environment detected"
    );
    let telinit =
        find("telinit").context("Timed application closure on OpenRC requires SysVinit telinit")?;
    ensure!(
        fs::canonicalize(&telinit)? == fs::canonicalize("/proc/1/exe")?,
        "Timed application closure requires SysVinit as PID 1; this OpenRC init variant is not supported"
    );
    let version = run(&telinit, &["--version"], Duration::from_secs(5))?;
    ensure!(
        format!("{}{}", version.stdout, version.stderr)
            .to_lowercase()
            .contains("sysvinit"),
        "Cannot identify a compatible SysVinit telinit"
    );
    let inittab = fs::read_to_string("/etc/inittab")?;
    ensure!(
        inittab
            .lines()
            .filter(|line| !line.trim_start().starts_with('#'))
            .any(|line| {
                let fields = line.splitn(4, ':').collect::<Vec<_>>();
                fields.len() == 4
                    && fields[1].contains('6')
                    && fields[2] == "wait"
                    && shell_words::split(fields[3]).is_ok_and(|args| {
                        args.len() == 2 && args[0].ends_with("/openrc") && args[1] == "reboot"
                    })
            }),
        "Cannot verify that runlevel 6 performs OpenRC's normal reboot sequence"
    );
    Ok(telinit)
}

fn escalate(seconds: u32) -> Result<()> {
    let systemd_run = find("systemd-run").context("systemd-run unavailable")?;
    let systemctl = find("systemctl").context("systemctl unavailable")?;
    let shell = find("sh").context("sh unavailable")?;
    let sleep = find("sleep").context("sleep unavailable")?;
    let state = tempfile::Builder::new()
        .prefix("boothop-shutdown-")
        .tempdir_in("/run")?;
    let receipt = state.path().join("receipt");
    let errors = state.path().join("errors");
    let unit = "boothop-shutdown";
    let mut args = vec![
        "--quiet".to_owned(),
        "--collect".into(),
        format!("--unit={unit}"),
        "--service-type=exec".into(),
    ];
    for property in shutdown::ESCALATION_PROPERTIES {
        args.push(format!("--property={property}"));
    }
    args.extend([
        "--".into(),
        shell.display().to_string(),
        "-c".into(),
        shutdown::ESCALATION_SCRIPT.into(),
        "boothop".into(),
        systemctl.display().to_string(),
        sleep.display().to_string(),
        seconds.to_string(),
        receipt.display().to_string(),
        errors.display().to_string(),
    ]);
    let refs = args.iter().map(String::as_str).collect::<Vec<_>>();
    let started = run(&systemd_run, &refs, Duration::from_secs(10));
    // Once submitted, a transport failure cannot prove that the service did not start.
    if let Err(error) = started {
        let retained = state.keep();
        return Err(ShutdownUncertain(format!(
            "Service submission is uncertain: {error:#}. Status files: {}",
            retained.display()
        ))
        .into());
    }
    let started = started?;
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match fs::read_to_string(&receipt).as_deref() {
            Ok("accepted\n") => return Ok(()),
            Ok("rejected\n") => bail!(
                "Graceful shutdown was rejected: {}",
                fs::read_to_string(&errors).unwrap_or_default()
            ),
            _ => {}
        }
        if started.code != 0 || Instant::now() >= deadline {
            let retained = state.keep();
            return Err(ShutdownUncertain(format!(
                "Service acceptance could not be established: {}. Inspect {unit}.service and {}",
                started.stderr,
                retained.display()
            ))
            .into());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}
