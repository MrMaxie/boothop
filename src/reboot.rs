use crate::boot::{Inventory, Target, resolve};
use anyhow::{Result, bail, ensure};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Closure {
    Graceful,
    Force,
    ForceAfter(u32),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    pub id: String,
    pub delay: u32,
    pub closure: Closure,
    pub yes: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Change {
    pub key: String,
    pub before: Option<Vec<u8>>,
    pub after: Vec<u8>,
}

#[derive(Debug)]
pub struct ShutdownUncertain(pub String);
impl std::fmt::Display for ShutdownUncertain {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}
impl std::error::Error for ShutdownUncertain {}

pub trait RebootSystem {
    fn application_grace_period(&self) -> bool {
        false
    }
    fn cancelled(&self) -> bool {
        false
    }
    fn discover(&mut self) -> Result<Inventory<Target>>;
    fn preflight(&mut self, target: &Target, request: &Request) -> Result<()>;
    fn confirm(&mut self, message: &str) -> Result<bool>;
    fn wait(&mut self, seconds: u32) -> Result<bool>;
    fn plan(&mut self, target: &Target) -> Result<Vec<Change>>;
    fn read(&mut self, key: &str) -> Result<Option<Vec<u8>>>;
    fn write(&mut self, key: &str, value: Option<&[u8]>) -> Result<()>;
    fn shutdown(&mut self, closure: &Closure) -> Result<()>;
}

pub fn execute(system: &mut impl RebootSystem, request: &Request) -> Result<String> {
    let target = resolve(system.discover()?, &request.id)?;
    system.preflight(&target, request)?;
    let policy = match request.closure {
        Closure::Graceful => "Close applications gracefully.".into(),
        Closure::Force => "Force applications to close. Unsaved work can be lost.".into(),
        Closure::ForceAfter(seconds) if system.application_grace_period() => format!(
            "Ask applications to close, wait {seconds} seconds, then force restart. System services close at restart. Unsaved work can be lost."
        ),
        Closure::ForceAfter(seconds) => format!(
            "Close gracefully, then force after {seconds} seconds. Unsaved work can be lost."
        ),
    };
    let message = format!(
        "Restart into {}?\nStart in {} seconds.\n{policy}",
        target.label, request.delay
    );
    if !request.yes && !system.confirm(&message)? {
        return Ok("Restart cancelled.".into());
    }
    if !system.wait(request.delay)? {
        return Ok("Restart cancelled.".into());
    }
    let refreshed = resolve(system.discover()?, &request.id)?;
    ensure!(
        target == refreshed,
        "Boot target changed during confirmation. Run 'boothop systems' again."
    );
    system.preflight(&refreshed, request)?;
    let changes = system.plan(&refreshed)?;
    ensure!(
        !changes.is_empty(),
        "No one-time boot selection was prepared"
    );
    let mut applied = Vec::new();
    for change in &changes {
        let result = (|| {
            ensure!(!system.cancelled(), "Restart cancelled before shutdown");
            ensure!(
                system.read(&change.key)? == change.before,
                "Boot state changed concurrently: {}",
                change.key
            );
            // Include even a failed write in recovery: an OS may write before returning an error.
            applied.push(change);
            system.write(&change.key, Some(&change.after))?;
            ensure!(
                system.read(&change.key)?.as_deref() == Some(change.after.as_slice()),
                "Boot write verification failed: {}",
                change.key
            );
            Ok(())
        })();
        if let Err(error) = result {
            return recover(system, &applied, error);
        }
    }
    if system.cancelled() {
        return recover(
            system,
            &applied,
            anyhow::anyhow!("Restart cancelled before shutdown"),
        );
    }
    if let Err(error) = system.shutdown(&request.closure) {
        if error.downcast_ref::<ShutdownUncertain>().is_some() {
            bail!(
                "{error:#}. Restart acceptance is unknown. One-time boot selection has been retained because shutdown may already be in progress."
            );
        }
        return recover(system, &applied, error);
    }
    Ok(format!("Restart request accepted for {}.", target.label))
}

fn recover(
    system: &mut impl RebootSystem,
    applied: &[&Change],
    error: anyhow::Error,
) -> Result<String> {
    let mut failures = Vec::new();
    for change in applied.iter().rev() {
        let result = (|| {
            let current = system.read(&change.key)?;
            if current == change.before {
                return Ok(());
            }
            ensure!(
                current.as_deref() == Some(change.after.as_slice()),
                "concurrent change, left untouched"
            );
            system.write(&change.key, change.before.as_deref())?;
            ensure!(
                system.read(&change.key)? == change.before,
                "restoration verification failed"
            );
            Ok(())
        })();
        if let Err(failure) = result {
            failures.push(format!("{}: {failure:#}", change.key));
        }
    }
    if failures.is_empty() {
        bail!("{error:#}. Previous one-time boot selection restored.");
    }
    bail!(
        "{error:#}. Recovery incomplete: {}. Inspect these one-time selections with the corresponding boot manager before restarting.",
        failures.join("; ")
    )
}
