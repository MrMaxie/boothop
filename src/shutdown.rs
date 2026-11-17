use crate::reboot::Closure;
use anyhow::{Result, bail};

/// A single --force delegates process termination and filesystem teardown to PID 1.
/// A second --force would bypass that teardown and must never be generated.
pub fn systemd_arguments(closure: &Closure) -> Result<Vec<&'static str>> {
    match closure {
        Closure::Graceful => Ok(vec!["reboot", "--no-block"]),
        Closure::Force => Ok(vec!["reboot", "--force", "--no-block"]),
        Closure::ForceAfter(_) => bail!("Timed escalation requires the system service"),
    }
}

// Positional arguments are passed directly to sh, never interpolated into shell code.
// systemd owns this service, so losing the CLI or its terminal does not stop it.
// Before=final.target releases it once orderly shutdown reaches its final phase.
pub const ESCALATION_SCRIPT: &str = r#""$1" reboot --no-block 2>"$5"
result=$?
if [ "$result" -ne 0 ]; then
    printf 'rejected\n' >"$4"
    exit "$result"
fi
printf 'accepted\n' >"$4"
"$2" "$3" || exit 1
exec "$1" reboot --force --no-block
"#;

pub const ESCALATION_PROPERTIES: [&str; 3] = [
    "DefaultDependencies=no",
    "IgnoreOnIsolate=yes",
    "Before=final.target",
];

pub fn sysv_arguments(closure: &Closure) -> Result<Vec<String>> {
    let seconds = match closure {
        Closure::Graceful => return Ok(vec!["6".into()]),
        Closure::Force => 0,
        Closure::ForceAfter(seconds) => *seconds,
    };
    anyhow::ensure!(
        seconds <= i32::MAX as u32,
        "Grace period exceeds the init backend's supported range"
    );
    Ok(vec!["-t".into(), seconds.to_string(), "6".into()])
}

pub trait ApplicationGraceService {
    fn request_applications(&mut self) -> Result<()>;
    fn accepted(&mut self) -> Result<()>;
    fn wait(&mut self, seconds: u32);
    fn force_restart(&mut self) -> Result<()>;
}

#[derive(Debug, PartialEq, Eq)]
pub enum ServiceAcceptance {
    Pending,
    Accepted,
    Rejected,
    Uncertain,
}

pub fn service_acceptance(receipt: &str, stopped: bool) -> ServiceAcceptance {
    match receipt {
        "accepted" => ServiceAcceptance::Accepted,
        "rejected" => ServiceAcceptance::Rejected,
        _ if stopped => ServiceAcceptance::Uncertain,
        _ => ServiceAcceptance::Pending,
    }
}

pub fn application_grace_restart(
    service: &mut impl ApplicationGraceService,
    seconds: u32,
) -> Result<()> {
    anyhow::ensure!(seconds > 0, "Application grace period must be positive");
    service.request_applications()?;
    service.accepted()?;
    service.wait(seconds);
    service.force_restart()
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct UserProcess {
    pub pid: i32,
    pub started: u64,
    pub uid: u32,
    pub session: i32,
    pub terminal: i64,
    pub login_uid: Option<u32>,
}

pub fn same_process(expected: &UserProcess, current: &UserProcess) -> bool {
    expected.pid == current.pid
        && expected.started == current.started
        && expected.uid == current.uid
}

pub fn process_record(pid: i32, stat: &str, status: &str, login_uid: &str) -> Result<UserProcess> {
    use anyhow::Context;
    let (_, fields) = stat.rsplit_once(") ").context("Invalid process stat")?;
    let fields = fields.split_whitespace().collect::<Vec<_>>();
    anyhow::ensure!(fields.len() > 19, "Truncated process stat");
    let uid = status
        .lines()
        .find(|line| line.starts_with("Uid:"))
        .and_then(|line| line.split_whitespace().nth(1))
        .context("Missing process UID")?
        .parse()?;
    Ok(UserProcess {
        pid,
        started: fields[19].parse()?,
        uid,
        session: fields[3].parse()?,
        terminal: fields[4].parse()?,
        login_uid: login_uid
            .trim()
            .parse::<u32>()
            .ok()
            .filter(|uid| *uid != u32::MAX),
    })
}

pub fn user_processes(processes: Vec<UserProcess>, uid_min: u32) -> Vec<UserProcess> {
    let sessions = processes
        .iter()
        .filter(|p| p.terminal != 0 && p.uid != 65534)
        .map(|p| (p.uid, p.session))
        .collect::<std::collections::BTreeSet<_>>();
    processes
        .into_iter()
        .filter(|p| {
            p.pid > 1
                && p.uid != 65534
                && ((p.uid >= uid_min
                    && (p.login_uid == Some(p.uid) || sessions.contains(&(p.uid, p.session))))
                    || (p.uid == 0 && p.terminal != 0))
        })
        .collect()
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct ProcessGraceState {
    pub processes: Vec<UserProcess>,
    pub deadline_ms: Option<u64>,
    pub submitted: bool,
}

pub trait ProcessGraceSystem {
    fn processes(&mut self) -> Result<Vec<UserProcess>>;
    fn now_ms(&self) -> u64;
    fn persist(&mut self, state: &ProcessGraceState) -> Result<()>;
    fn signal(&mut self, process: &UserProcess, force: bool) -> Result<()>;
    fn accepted(&mut self) -> Result<()>;
    fn wait_until(&mut self, deadline_ms: u64);
    fn restart(&mut self) -> Result<()>;
}

pub fn process_grace_restart(
    system: &mut impl ProcessGraceSystem,
    seconds: u32,
    previous: Option<ProcessGraceState>,
) -> Result<()> {
    use anyhow::Context;
    let mut state = match previous {
        Some(state) => state,
        None => ProcessGraceState {
            processes: system.processes()?,
            deadline_ms: None,
            submitted: false,
        },
    };
    if state.submitted {
        return Err(crate::reboot::ShutdownUncertain(
            "A previous helper may have submitted restart; refusing a duplicate request".into(),
        )
        .into());
    }
    system.persist(&state)?;
    if state.deadline_ms.is_none() {
        if seconds > 0 {
            for process in &state.processes {
                system.signal(process, false)?;
            }
        }
        state.deadline_ms = Some(
            system
                .now_ms()
                .checked_add(u64::from(seconds) * 1000)
                .context("Grace deadline overflow")?,
        );
        system.persist(&state)?;
    }
    system.accepted()?;
    system.wait_until(state.deadline_ms.context("Missing grace deadline")?);
    for process in &state.processes {
        system.signal(process, true)?;
    }
    state.submitted = true;
    system.persist(&state)?;
    system.restart()
}
