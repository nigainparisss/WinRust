//! Starts a program with the signed-in user's normal token from an elevated process (the installer, the topbar task).
//! `runas /trustlevel:0x20000` is no substitute: its token keeps the High label with Administrators deny-only, so nothing
//! started from it (the whole desktop, when it is Explorer) can elevate again, and Windows does not even show the UAC prompt.
use std::{
    mem::{size_of, zeroed},
    process::Command,
    ptr::{null, null_mut},
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE},
    Security::{DuplicateTokenEx, GetTokenInformation, SecurityImpersonation, TOKEN_ASSIGN_PRIMARY, TOKEN_DUPLICATE, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation, TokenPrimary},
    System::{
        Diagnostics::ToolHelp::{CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS},
        RemoteDesktop::ProcessIdToSessionId,
        Threading::*,
    },
    UI::WindowsAndMessaging::{FindWindowW, GetShellWindow, GetWindowThreadProcessId},
};

pub const EXPLORER: &str = r"C:\Windows\explorer.exe";
const MAXIMUM_ALLOWED: u32 = 0x0200_0000;

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain([0]).collect()
}

pub fn elevated() -> bool {
    unsafe {
        let mut t = null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut t) == 0 {
            return false;
        }
        let mut e = TOKEN_ELEVATION { TokenIsElevated: 0 };
        let mut n = 0;
        let ok = GetTokenInformation(t, TokenElevation, &mut e as *mut _ as *mut _, size_of::<TOKEN_ELEVATION>() as u32, &mut n) != 0;
        CloseHandle(t);
        ok && e.TokenIsElevated != 0
    }
}

/// sihost.exe runs in every interactive session with the token Explorer gets at logon.
fn user_token() -> Option<HANDLE> {
    unsafe {
        let mut me = 0;
        ProcessIdToSessionId(GetCurrentProcessId(), &mut me);
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snap == INVALID_HANDLE_VALUE {
            return None;
        }
        let mut pe: PROCESSENTRY32W = zeroed();
        pe.dwSize = size_of::<PROCESSENTRY32W>() as u32;
        let mut tok = None;
        let mut more = Process32FirstW(snap, &mut pe) != 0;
        while more && tok.is_none() {
            let len = pe.szExeFile.iter().position(|&c| c == 0).unwrap_or(pe.szExeFile.len());
            let mut s = u32::MAX;
            if String::from_utf16_lossy(&pe.szExeFile[..len]).eq_ignore_ascii_case("sihost.exe") && ProcessIdToSessionId(pe.th32ProcessID, &mut s) != 0 && s == me {
                let p = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pe.th32ProcessID);
                if !p.is_null() {
                    let mut t = null_mut();
                    if OpenProcessToken(p, TOKEN_DUPLICATE | TOKEN_QUERY | TOKEN_ASSIGN_PRIMARY, &mut t) != 0 {
                        let mut d = null_mut();
                        if DuplicateTokenEx(t, MAXIMUM_ALLOWED, null(), SecurityImpersonation, TokenPrimary, &mut d) != 0 {
                            tok = Some(d);
                        }
                        CloseHandle(t);
                    }
                    CloseHandle(p);
                }
            }
            more = Process32NextW(snap, &mut pe) != 0;
        }
        CloseHandle(snap);
        tok
    }
}

/// `exe` with no arguments; unelevated callers just start it.
pub fn spawn_unelevated(exe: &str) -> Result<(), String> {
    if !elevated() {
        return Command::new(exe).spawn().map(|_| ()).map_err(|e| e.to_string());
    }
    let tok = user_token().ok_or("no sihost.exe in this session")?;
    unsafe {
        let mut si: STARTUPINFOW = zeroed();
        si.cb = size_of::<STARTUPINFOW>() as u32;
        let mut pi: PROCESS_INFORMATION = zeroed();
        let mut cmd = wide(&format!("\"{exe}\""));
        let ok = CreateProcessWithTokenW(tok, 0, null(), cmd.as_mut_ptr(), 0, null(), null(), &si, &mut pi) != 0;
        let err = std::io::Error::last_os_error();
        CloseHandle(tok);
        if !ok {
            return Err(err.to_string());
        }
        CloseHandle(pi.hProcess);
        CloseHandle(pi.hThread);
    }
    Ok(())
}

fn exe_of(pid: u32) -> String {
    unsafe {
        let p = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if p.is_null() {
            return String::new();
        }
        let mut buf = [0u16; 512];
        let mut n = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(p, PROCESS_NAME_WIN32, buf.as_mut_ptr(), &mut n) != 0;
        CloseHandle(p);
        if ok { String::from_utf16_lossy(&buf[..n as usize]) } else { String::new() }
    }
}

/// The shell window belongs to Explorer (not to rust-explorer's desktop, which is a Progman window too).
pub fn explorer_is_shell() -> bool {
    unsafe {
        let h = GetShellWindow();
        let mut pid = 0;
        !h.is_null() && GetWindowThreadProcessId(h, &mut pid) != 0 && exe_of(pid).to_ascii_lowercase().ends_with(r"\explorer.exe")
    }
}

/// Ends rust-explorer's desktop window: with it up Explorer thinks a shell exists and starts as a plain folder host.
pub fn end_rs_desktop() {
    unsafe {
        let h = FindWindowW(wide("Progman").as_ptr(), wide("rs-desktop").as_ptr());
        let mut pid = 0;
        if h.is_null() || GetWindowThreadProcessId(h, &mut pid) == 0 {
            return;
        }
        let p = OpenProcess(PROCESS_TERMINATE, 0, pid);
        if !p.is_null() {
            TerminateProcess(p, 0);
            CloseHandle(p);
        }
    }
}
