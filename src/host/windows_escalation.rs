use super::windows;
use crate::reboot::{Closure, ShutdownUncertain};
use anyhow::{Context, Result, bail, ensure};
use std::{
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    ptr,
    time::{Duration, Instant},
};
use windows_service::{
    define_windows_service,
    service::*,
    service_control_handler::{self, ServiceControlHandlerResult},
    service_dispatcher,
    service_manager::{ServiceManager, ServiceManagerAccess},
};
use windows_sys::Win32::{
    Foundation::*,
    Security::{Authorization::*, *},
    Storage::FileSystem::*,
    System::{Com::CoTaskMemFree, RemoteDesktop::*, Threading::*},
    UI::{
        Shell::{FOLDERID_ProgramFiles, SHGetKnownFolderPath},
        WindowsAndMessaging::*,
    },
};

const NAME: &str = "BoothopShutdown";
const DIRECTORY: &str = "Boothop Shutdown";

fn wide(value: &std::ffi::OsStr) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    value.encode_wide().chain(Some(0)).collect()
}

fn program_files() -> Result<PathBuf> {
    use std::os::windows::ffi::OsStringExt;
    unsafe {
        let mut raw = ptr::null_mut();
        ensure!(
            SHGetKnownFolderPath(&FOLDERID_ProgramFiles, 0, ptr::null_mut(), &mut raw) >= 0,
            "Cannot locate Program Files"
        );
        let mut length = 0;
        while *raw.add(length) != 0 {
            length += 1;
        }
        let path = OsString::from_wide(std::slice::from_raw_parts(raw, length));
        CoTaskMemFree(raw.cast());
        Ok(PathBuf::from(path))
    }
}

pub fn preflight() -> Result<()> {
    ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT | ServiceManagerAccess::CREATE_SERVICE)
        .context("Timed escalation requires an elevated administrator terminal and access to the Service Control Manager")?;
    ensure!(
        !program_files()?.join(DIRECTORY).exists(),
        "A previous Boothop shutdown helper still exists. Inspect the BoothopShutdown service before retrying."
    );
    Ok(())
}

fn create_directory(path: &Path) -> Result<()> {
    // A protected administrator-owned directory prevents a user session from
    // replacing the executable that SCM will launch as LocalSystem.
    unsafe {
        let mut descriptor = ptr::null_mut();
        let sddl = wide(std::ffi::OsStr::new(
            "O:BAG:BAD:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;OICI;GRGX;;;BU)",
        ));
        ensure!(
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                1,
                &mut descriptor,
                ptr::null_mut()
            ) != 0,
            "Cannot prepare helper permissions"
        );
        let security = SECURITY_ATTRIBUTES {
            nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: descriptor,
            bInheritHandle: 0,
        };
        let ok = CreateDirectoryW(wide(path.as_os_str()).as_ptr(), &security);
        let error = std::io::Error::last_os_error();
        LocalFree(descriptor);
        ensure!(ok != 0, "Cannot create protected helper directory: {error}");
    }
    Ok(())
}

pub fn start(seconds: u32) -> Result<()> {
    let directory = program_files()?.join(DIRECTORY);
    create_directory(&directory)?;
    let executable = directory.join("boothop-helper.exe");
    let preparation = (|| -> Result<_> {
        fs::copy(std::env::current_exe()?, &executable)?;
        secure_executable(&executable)?;
        let manager = ServiceManager::local_computer(
            None::<&str>,
            ServiceManagerAccess::CONNECT | ServiceManagerAccess::CREATE_SERVICE,
        )?;
        manager
            .create_service(
                &ServiceInfo {
                    name: NAME.into(),
                    display_name: "Boothop temporary shutdown helper".into(),
                    service_type: ServiceType::OWN_PROCESS,
                    start_type: ServiceStartType::OnDemand,
                    error_control: ServiceErrorControl::Normal,
                    executable_path: executable.clone(),
                    launch_arguments: vec!["__boothop-service".into(), seconds.to_string().into()],
                    dependencies: vec![],
                    account_name: None,
                    account_password: None,
                },
                ServiceAccess::START | ServiceAccess::QUERY_STATUS | ServiceAccess::DELETE,
            )
            .map_err(Into::into)
    })();
    let service = match preparation {
        Ok(service) => service,
        Err(error) => {
            let _ = fs::remove_file(&executable);
            let _ = fs::remove_dir(&directory);
            return Err(error);
        }
    };
    if let Err(error) = service.start::<&str>(&[]) {
        // ERROR_SERVICE_REQUEST_TIMEOUT does not prove the service failed to start.
        let _ = service.delete();
        return Err(ShutdownUncertain(format!(
            "Shutdown helper startup is uncertain: {error}. Inspect {NAME} and {}",
            directory.display()
        ))
        .into());
    }
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        use crate::shutdown::ServiceAcceptance;
        let receipt = fs::read_to_string(directory.join("receipt")).unwrap_or_default();
        let status = service
            .query_status()
            .map_err(|error| ShutdownUncertain(format!("Cannot query shutdown helper: {error}")))?;
        let acceptance = crate::shutdown::service_acceptance(
            &receipt,
            status.current_state == ServiceState::Stopped,
        );
        if acceptance == ServiceAcceptance::Accepted {
            // SCM retains the running service until it exits, but it cannot start again.
            service.delete().map_err(|error| {
                ShutdownUncertain(format!(
                    "Shutdown helper accepted the request but cleanup registration failed: {error}"
                ))
            })?;
            return Ok(());
        }
        if acceptance == ServiceAcceptance::Rejected
            && status.current_state == ServiceState::Stopped
        {
            let _ = service.delete();
            // SCM's stopped status can precede process exit and file-handle release.
            let cleanup_deadline = Instant::now() + Duration::from_secs(2);
            while executable.exists()
                && fs::remove_file(&executable).is_err()
                && Instant::now() < cleanup_deadline
            {
                std::thread::sleep(Duration::from_millis(25));
            }
            let _ = fs::remove_file(directory.join("receipt"));
            let cleanup = fs::remove_dir(&directory);
            bail!(
                "Shutdown helper rejected the request ({:?}). No forced restart was scheduled.{}",
                status.exit_code,
                if cleanup.is_err() && directory.exists() {
                    format!(
                        " Helper cleanup is incomplete; inspect {} before retrying.",
                        directory.display()
                    )
                } else {
                    String::new()
                }
            );
        }
        if acceptance == ServiceAcceptance::Uncertain {
            let _ = service.delete();
            return Err(ShutdownUncertain("Shutdown helper stopped without a retained outcome; restart may already have been accepted".into()).into());
        }
        if Instant::now() >= deadline {
            let _ = service.delete();
            return Err(ShutdownUncertain(
                "Shutdown helper did not report acceptance within 30 seconds".into(),
            )
            .into());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn secure_executable(path: &Path) -> Result<()> {
    unsafe {
        let mut descriptor = ptr::null_mut();
        let sddl = wide(std::ffi::OsStr::new(
            "O:BAG:BAD:P(A;;FA;;;SY)(A;;FA;;;BA)(A;;GRGX;;;BU)",
        ));
        ensure!(
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                1,
                &mut descriptor,
                ptr::null_mut()
            ) != 0,
            "Cannot prepare helper executable permissions"
        );
        let ok = SetFileSecurityW(
            wide(path.as_os_str()).as_ptr(),
            OWNER_SECURITY_INFORMATION
                | DACL_SECURITY_INFORMATION
                | PROTECTED_DACL_SECURITY_INFORMATION,
            descriptor,
        );
        let error = std::io::Error::last_os_error();
        LocalFree(descriptor);
        ensure!(
            ok != 0,
            "Cannot protect the system helper executable: {error}"
        );
    }
    Ok(())
}

/// Internal entry points are excluded from every simulation test binary.
pub fn dispatch(args: &[OsString]) -> Option<Result<()>> {
    match args.get(1)?.to_str()? {
        "__boothop-service" => {
            Some(service_dispatcher::start(NAME, service_main).map_err(Into::into))
        }
        "__boothop-close-apps" => Some(close_applications(args)),
        _ => None,
    }
}

define_windows_service!(service_main, run_service);

fn run_service(_: Vec<OsString>) {
    let result = service_control_handler::register(NAME, |control| match control {
        ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
        _ => ServiceControlHandlerResult::NotImplemented,
    });
    let Ok(handle) = result else {
        return;
    };
    let status = |state, code| ServiceStatus {
        service_type: ServiceType::OWN_PROCESS,
        current_state: state,
        controls_accepted: ServiceControlAccept::empty(),
        exit_code: ServiceExitCode::Win32(code),
        checkpoint: if state == ServiceState::StartPending {
            1
        } else {
            0
        },
        wait_hint: Duration::from_secs(30),
        process_id: None,
    };
    let mut accepted = false;
    let outcome = (|| -> Result<()> {
        handle.set_service_status(status(ServiceState::StartPending, 0))?;
        let seconds = std::env::args()
            .nth(2)
            .context("Missing grace period")?
            .parse::<u32>()?;
        ensure!(seconds > 0, "Grace period must be positive");
        windows::require_privilege("SeShutdownPrivilege")?;
        windows::require_privilege("SeTcbPrivilege")?;
        handle.set_service_status(status(ServiceState::Running, 0))?;
        crate::shutdown::application_grace_restart(
            &mut GraceService {
                receipt: std::env::current_exe()?.with_file_name("receipt"),
                accepted: &mut accepted,
            },
            seconds,
        )
    })();
    // A missing receipt is ambiguous after service cleanup. Retain an explicit
    // rejection only when the timer was never acknowledged and restart never ran.
    if outcome.is_err()
        && !accepted
        && let Ok(executable) = std::env::current_exe()
    {
        let _ = fs::write(executable.with_file_name("receipt"), "rejected");
    }
    // Remove our service even if the originating CLI disappeared before acceptance.
    if let Ok(manager) = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)
        && let Ok(service) = manager.open_service(NAME, ServiceAccess::DELETE)
    {
        let _ = service.delete();
    }
    let _ = handle.set_service_status(status(
        ServiceState::Stopped,
        if outcome.is_ok() { 0 } else { 1 },
    ));
    if let Ok(executable) = std::env::current_exe()
        && let Some(directory) = executable.parent()
        && self_replace::self_delete_outside_path(directory).is_ok()
    {
        if accepted {
            let _ = fs::remove_file(directory.join("receipt"));
        }
        let _ = fs::remove_dir(directory);
    }
}

struct GraceService<'a> {
    receipt: PathBuf,
    accepted: &'a mut bool,
}
impl crate::shutdown::ApplicationGraceService for GraceService<'_> {
    fn request_applications(&mut self) -> Result<()> {
        request_session_closure()
    }
    fn accepted(&mut self) -> Result<()> {
        fs::write(&self.receipt, "accepted")?;
        *self.accepted = true;
        Ok(())
    }
    fn wait(&mut self, seconds: u32) {
        std::thread::sleep(Duration::from_secs(seconds.into()));
    }
    fn force_restart(&mut self) -> Result<()> {
        windows::shutdown(&Closure::Force)
    }
}

fn request_session_closure() -> Result<()> {
    unsafe {
        let mut raw = ptr::null_mut();
        let mut count = 0;
        ensure!(
            WTSEnumerateSessionsW(ptr::null_mut(), 0, 1, &mut raw, &mut count) != 0,
            "Cannot enumerate logged-on sessions"
        );
        let sessions = if count == 0 {
            Vec::new()
        } else {
            std::slice::from_raw_parts(raw, count as usize)
                .iter()
                .filter(|s| {
                    s.SessionId != 0
                        && [WTSActive, WTSConnected, WTSDisconnected].contains(&s.State)
                })
                .map(|s| s.SessionId)
                .collect::<Vec<_>>()
        };
        WTSFreeMemory(raw.cast());
        let mut children = Vec::new();
        for session in sessions {
            let mut token = ptr::null_mut();
            ensure!(
                WTSQueryUserToken(session, &mut token) != 0,
                "Cannot obtain the user token for session {session}"
            );
            let result = start_session_helper(token);
            CloseHandle(token);
            match result {
                Ok(child) => children.push(child),
                Err(error) => {
                    for child in children {
                        CloseHandle(child);
                    }
                    return Err(error);
                }
            }
        }
        let mut failures = false;
        let deadline = Instant::now() + Duration::from_secs(15);
        for child in children {
            let timeout = deadline
                .saturating_duration_since(Instant::now())
                .as_millis() as u32;
            let result = WaitForSingleObject(child, timeout);
            let mut code = 1;
            if result != WAIT_OBJECT_0 {
                TerminateProcess(child, 1);
            }
            if GetExitCodeProcess(child, &mut code) == 0 || code != 0 {
                failures = true;
            }
            CloseHandle(child);
        }
        ensure!(
            !failures,
            "One or more sessions could not receive application closure requests"
        );
    }
    Ok(())
}

unsafe fn start_session_helper(token: HANDLE) -> Result<HANDLE> {
    let executable = std::env::current_exe()?;
    let command = format!(
        "\"{}\" __boothop-close-apps {}",
        executable.display(),
        std::process::id()
    );
    let mut command = wide(std::ffi::OsStr::new(&command));
    let mut desktop = wide(std::ffi::OsStr::new("winsta0\\default"));
    let startup = STARTUPINFOW {
        cb: size_of::<STARTUPINFOW>() as u32,
        lpDesktop: desktop.as_mut_ptr(),
        ..Default::default()
    };
    let mut process = PROCESS_INFORMATION::default();
    unsafe {
        ensure!(
            CreateProcessAsUserW(
                token,
                wide(executable.as_os_str()).as_ptr(),
                command.as_mut_ptr(),
                ptr::null(),
                ptr::null(),
                0,
                CREATE_NO_WINDOW,
                ptr::null(),
                ptr::null(),
                &startup,
                &mut process
            ) != 0,
            "Cannot start a session closure helper: {}",
            std::io::Error::last_os_error()
        );
        CloseHandle(process.hThread);
    }
    Ok(process.hProcess)
}

fn close_applications(args: &[OsString]) -> Result<()> {
    let expected = args
        .get(2)
        .and_then(|v| v.to_str())
        .context("Missing service identity")?
        .parse::<u32>()?;
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)?;
    let service = manager.open_service(NAME, ServiceAccess::QUERY_STATUS)?;
    let status = service.query_status()?;
    ensure!(
        status.process_id == Some(expected) && status.current_state == ServiceState::Running,
        "No approved shutdown service is requesting application closure"
    );
    unsafe {
        ensure!(
            EnumWindows(Some(close_window), 0) != 0,
            "Cannot enumerate application windows"
        );
    }
    Ok(())
}

unsafe extern "system" fn close_window(window: HWND, _: LPARAM) -> i32 {
    unsafe {
        if IsWindowVisible(window) == 0
            || window == GetShellWindow()
            || window == GetDesktopWindow()
        {
            return 1;
        }
        let mut class = [0_u16; 128];
        let length = GetClassNameW(window, class.as_mut_ptr(), class.len() as i32);
        let class = String::from_utf16_lossy(&class[..length.max(0) as usize]);
        if [
            "Progman",
            "WorkerW",
            "Shell_TrayWnd",
            "Shell_SecondaryTrayWnd",
        ]
        .contains(&class.as_str())
        {
            return 1;
        }
        // Post instead of waiting for a window that may display a save prompt.
        PostMessageW(window, WM_CLOSE, 0, 0);
    }
    1
}
