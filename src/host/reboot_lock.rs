#[cfg(not(windows))]
use anyhow::Context;
use anyhow::Result;

#[cfg(windows)]
pub struct RebootLock(windows_sys::Win32::Foundation::HANDLE);

#[cfg(windows)]
impl RebootLock {
    pub fn acquire() -> Result<Self> {
        use windows_sys::Win32::{Foundation::*, System::Threading::*};
        let name = "Global\\BoothopReboot\0".encode_utf16().collect::<Vec<_>>();
        unsafe {
            let mutex = CreateMutexW(std::ptr::null(), 0, name.as_ptr());
            anyhow::ensure!(
                !mutex.is_null(),
                "Cannot acquire reboot coordination lock: {}",
                std::io::Error::last_os_error()
            );
            let result = WaitForSingleObject(mutex, 0);
            if result != WAIT_OBJECT_0 && result != WAIT_ABANDONED {
                CloseHandle(mutex);
                anyhow::bail!("Another Boothop reboot operation is running");
            }
            Ok(Self(mutex))
        }
    }
}
#[cfg(windows)]
impl Drop for RebootLock {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::System::Threading::ReleaseMutex(self.0);
            windows_sys::Win32::Foundation::CloseHandle(self.0);
        }
    }
}

#[cfg(not(windows))]
pub struct RebootLock {
    _file: std::fs::File,
}

#[cfg(not(windows))]
impl RebootLock {
    pub fn acquire() -> Result<Self> {
        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open("/run/boothop-reboot.lock")?;
        file.try_lock()
            .context("Another Boothop reboot operation is running")?;
        Ok(Self { _file: file })
    }
}
