use crate::{
    boot::{Selection, Target},
    reboot::Closure,
};
use anyhow::{Context, Result, ensure};
use std::{ptr, time::Duration};
use windows_sys::Win32::{
    Foundation::*,
    Security::*,
    System::{Shutdown::*, Threading::*, WindowsProgramming::*},
};

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}

pub fn require_privilege(name: &str) -> Result<()> {
    // All pointers refer to initialized values that remain alive for the call.
    unsafe {
        let mut token = ptr::null_mut();
        ensure!(
            OpenProcessToken(
                GetCurrentProcess(),
                TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY,
                &mut token
            ) != 0,
            "Cannot open process token: {}",
            std::io::Error::last_os_error()
        );
        let result = (|| {
            let mut luid = LUID::default();
            ensure!(
                LookupPrivilegeValueW(ptr::null(), wide(name).as_ptr(), &mut luid) != 0,
                "Cannot resolve {name}"
            );
            let state = TOKEN_PRIVILEGES {
                PrivilegeCount: 1,
                Privileges: [LUID_AND_ATTRIBUTES {
                    Luid: luid,
                    Attributes: SE_PRIVILEGE_ENABLED,
                }],
            };
            SetLastError(0);
            ensure!(
                AdjustTokenPrivileges(token, 0, &state, 0, ptr::null_mut(), ptr::null_mut()) != 0
                    && GetLastError() == 0,
                "{name} is unavailable. Run from an elevated terminal."
            );
            Ok(())
        })();
        CloseHandle(token);
        result
    }
}

pub fn firmware_read(name: &str, guid: &str) -> Result<Option<Vec<u8>>> {
    require_privilege("SeSystemEnvironmentPrivilege")?;
    let mut buffer = vec![0_u8; 1024 * 1024];
    let mut flags = 0;
    let length = unsafe {
        GetFirmwareEnvironmentVariableExW(
            wide(name).as_ptr(),
            wide(&format!("{{{guid}}}")).as_ptr(),
            buffer.as_mut_ptr().cast(),
            buffer.len() as u32,
            &mut flags,
        )
    };
    if length == 0 {
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() == Some(ERROR_ENVVAR_NOT_FOUND as i32) {
            return Ok(None);
        }
        return Err(error).context("Cannot read firmware variable");
    }
    buffer.truncate(length as usize);
    Ok(Some(buffer))
}

pub fn firmware_write(name: &str, guid: &str, value: Option<&[u8]>) -> Result<()> {
    require_privilege("SeSystemEnvironmentPrivilege")?;
    let bytes = value.unwrap_or_default();
    let ok = unsafe {
        SetFirmwareEnvironmentVariableExW(
            wide(name).as_ptr(),
            wide(&format!("{{{guid}}}")).as_ptr(),
            if bytes.is_empty() {
                ptr::null()
            } else {
                bytes.as_ptr().cast()
            },
            bytes.len() as u32,
            7,
        )
    };
    ensure!(
        ok != 0,
        "Cannot write firmware variable: {}",
        std::io::Error::last_os_error()
    );
    Ok(())
}

pub fn confirmation(message: &str) -> Option<bool> {
    if !desktop_available() {
        return None;
    }
    crate::windows_dialog::confirmation(message)
}

pub fn desktop_available() -> bool {
    use windows_sys::Win32::System::StationsAndDesktops::{
        CloseDesktop, DESKTOP_READOBJECTS, OpenInputDesktop,
    };
    if std::env::var_os("SSH_CONNECTION").is_some() || std::env::var_os("SSH_TTY").is_some() {
        return false;
    }
    unsafe {
        let desktop = OpenInputDesktop(0, 0, DESKTOP_READOBJECTS);
        if desktop.is_null() {
            return false;
        }
        CloseDesktop(desktop);
    }
    true
}

pub fn shutdown(closure: &Closure) -> Result<()> {
    require_privilege("SeShutdownPrivilege")?;
    let flags = match closure {
        Closure::Graceful => SHUTDOWN_RESTART,
        Closure::Force => SHUTDOWN_RESTART | SHUTDOWN_FORCE_SELF | SHUTDOWN_FORCE_OTHERS,
        Closure::ForceAfter(seconds) => return super::windows_escalation::start(*seconds),
    };
    let result = unsafe {
        InitiateShutdownW(
            ptr::null(),
            ptr::null(),
            0,
            flags,
            SHTDN_REASON_MAJOR_APPLICATION | SHTDN_REASON_MINOR_OTHER | SHTDN_REASON_FLAG_PLANNED,
        )
    };
    ensure!(
        result == 0,
        "Windows rejected restart: {}",
        std::io::Error::from_raw_os_error(result as i32)
    );
    Ok(())
}

pub fn powershell(script: &str) -> Result<String> {
    let program =
        super::process::find("powershell.exe").context("Windows PowerShell is unavailable")?;
    let script = format!(
        "$ErrorActionPreference='Stop'; [Console]::OutputEncoding=[Text.UTF8Encoding]::new($false); {script}"
    );
    let result = super::process::run(
        &program,
        &[
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            &script,
        ],
        Duration::from_secs(15),
    )?;
    ensure!(
        result.code == 0,
        "Windows query failed: {}",
        result.stderr.trim()
    );
    Ok(result.stdout.trim_start_matches('\u{feff}').trim().into())
}

const OPEN_STORE: &str = "$r=([wmiclass]'root\\WMI:BcdStore').OpenStore(''); if(-not $r.ReturnValue){throw 'Cannot open system BCD store'}; $store=$r.Store;";

pub fn bcd_targets() -> Result<Vec<Target>> {
    let script = format!(
        "{OPEN_STORE} $r=$store.EnumerateObjects(0x10200003); if(-not $r.ReturnValue){{throw 'Cannot enumerate BCD entries'}}; $items=@($r.Objects | ForEach-Object {{ $d=$_.GetElement(0x12000004); $e=$_.EnumerateElements(); if(-not $e.ReturnValue){{throw 'Cannot read BCD entry'}}; [pscustomobject]@{{id=$_.Id; label=$d.Element.String; evidence=($e.Elements | ConvertTo-Json -Depth 8 -Compress)}} }}); ConvertTo-Json -InputObject $items -Compress"
    );
    let values: Vec<serde_json::Value> = serde_json::from_str(&powershell(&script)?)?;
    values
        .iter()
        .map(|value| {
            let id = value["id"].as_str().context("BCD object missing ID")?;
            validate_guid(id)?;
            Ok(Target {
                id: format!("bcd:{id}"),
                label: value["label"].as_str().unwrap_or("Windows").into(),
                current: false,
                location: "Windows Boot Manager".into(),
                selection: Selection::Bcd(id.into()),
                unavailable: None,
                fingerprint: crate::boot::hex_id(value.to_string().as_bytes()),
            })
        })
        .collect()
}

fn validate_guid(id: &str) -> Result<()> {
    ensure!(
        id.len() == 38
            && id.starts_with('{')
            && id.ends_with('}')
            && id[1..37].bytes().enumerate().all(|(index, byte)| {
                if [8, 13, 18, 23].contains(&index) {
                    byte == b'-'
                } else {
                    byte.is_ascii_hexdigit()
                }
            }),
        "Invalid BCD identifier"
    );
    Ok(())
}

pub fn bcd_sequence() -> Result<Option<Vec<u8>>> {
    let script = format!(
        "{OPEN_STORE} $o=$store.OpenObject('{{9dea862c-5cdd-4e70-acc1-f32b344d4795}}'); if(-not $o.ReturnValue){{throw 'Cannot read boot manager'}}; $all=$o.Object.EnumerateElements(); if(-not $all.ReturnValue){{throw 'Cannot enumerate boot manager elements'}}; $e=@($all.Elements | Where-Object Type -eq 0x24000002); if($e.Count -gt 0){{ $e[0].Ids -join ',' }}"
    );
    let sequence = powershell(&script)?;
    if sequence.is_empty() {
        return Ok(None);
    }
    for id in sequence.split(',') {
        validate_guid(id)?;
    }
    Ok(Some(sequence.into_bytes()))
}

pub fn set_bcd_sequence(value: Option<&[u8]>) -> Result<()> {
    let operation = if let Some(bytes) = value {
        let ids = std::str::from_utf8(bytes)?
            .split(',')
            .map(|id| {
                validate_guid(id)?;
                Ok(format!("'{id}'"))
            })
            .collect::<Result<Vec<_>>>()?;
        format!(
            "$r=$o.Object.SetObjectListElement(0x24000002,[string[]]@({}))",
            ids.join(",")
        )
    } else {
        "$r=$o.Object.DeleteElement(0x24000002)".into()
    };
    powershell(&format!(
        "{OPEN_STORE} $o=$store.OpenObject('{{9dea862c-5cdd-4e70-acc1-f32b344d4795}}'); if(-not $o.ReturnValue){{throw 'Cannot open boot manager'}}; {operation}; if(-not $r.ReturnValue){{throw 'BCD write rejected'}}"
    ))?;
    Ok(())
}
