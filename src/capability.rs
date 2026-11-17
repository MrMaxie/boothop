use anyhow::{Result, bail, ensure};

/// Probe candidates in priority order. Probes inspect capabilities, not operations.
pub fn first_usable<T>(names: &[&str], mut probe: impl FnMut(&str) -> Result<T>) -> Result<T> {
    let mut failures = Vec::new();
    for name in names {
        match probe(name) {
            Ok(value) => return Ok(value),
            Err(error) => failures.push(format!("{name}: {error:#}")),
        }
    }
    bail!("No usable mechanism: {}", failures.join("; "))
}

pub trait Confirmation {
    fn dialog(&mut self, message: &str) -> Option<bool>;
    fn interactive(&self) -> bool;
    fn terminal(&mut self, message: &str) -> Result<bool>;
}

pub fn confirm(system: &mut impl Confirmation, message: &str) -> Result<bool> {
    if let Some(answer) = system.dialog(message) {
        return Ok(answer);
    }
    ensure!(
        system.interactive(),
        "No confirmation dialog or interactive terminal is available. Use --yes to approve explicitly."
    );
    system.terminal(message)
}

pub fn terminal_answer(line: &str) -> bool {
    matches!(line.trim().to_ascii_lowercase().as_str(), "y" | "yes")
}
