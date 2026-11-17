pub mod boot;
pub mod bootstore;
pub mod capability;
pub mod cli;
pub mod disk;
pub mod installation;
pub mod launcher;
pub mod rawfs;
pub mod reboot;
pub mod shutdown;
pub mod update;

// Unit tests cannot instantiate the production adapters.
#[cfg(not(test))]
pub mod host;

#[cfg(all(windows, not(test)))]
pub mod windows_dialog;

#[cfg(test)]
mod tests;
