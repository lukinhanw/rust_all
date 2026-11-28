//! Starting things the way the shell does, without relying on Explorer being the shell.
//!
//! `explorer.exe <target>` is how WinRust hands most targets over: it starts paths, shortcuts, shell addresses and protocol
//! URIs (`ms-settings:`) with the desktop user's normal, unelevated token. A program is the exception — Explorer treats it
//! as a path to show, and when Explorer is not the shell (WinRust replaces it: see `rust-explorer`) the command only opens
//! a folder window and Windows never starts the program. That is why "Terminal" in the Win+X menu opened the file manager,
//! and why the installer's own "start it now" steps started nothing at all.
//!
//! [`program`] starts those the way Explorer would, with the same restricted token (`runas /trustlevel:0x20000`, the trick
//! the rest of WinRust uses for a normal-rights child) and ShellExecuteW as the last resort.

use std::process::Command;

/// CreateProcessW flag: no console window for the helper.
const NO_WINDOW: u32 = 0x0800_0000;

/// The extensions Explorer, not being the shell, opens as a folder instead of starting the program.
const PROGRAM_EXTS: [&str; 7] = ["exe", "com", "scr", "bat", "cmd", "msc", "cpl"];

fn ext_of(target: &str) -> String {
    match target.trim().trim_matches('"').rsplit_once('.') {
        Some((_, ext)) => ext.trim().to_ascii_lowercase(),
        None => String::new(),
    }
}

/// True for the targets [`program`] has to start itself: `wt.exe`, `taskmgr.exe`, `devmgmt.msc`, `powercfg.cpl`, a `.bat`.
pub fn is_program(target: &str) -> bool {
    let t = target.trim().trim_matches('"');
    if t.is_empty() || is_uri(t) {
        return false;
    }
    PROGRAM_EXTS.contains(&ext_of(t).as_str())
}

/// A protocol URI or a shell address: the shell's, even when its host name ends in `.com` like an exe would.
fn is_uri(t: &str) -> bool {
    if t.contains("://") || t.starts_with("::") {
        return true;
    }
    match t.find(':') {
        Some(i) if (1..8).contains(&i) && !(i == 1 && (t[i + 1..].starts_with('\\') || t[i + 1..].starts_with('/'))) => {
            t[..i].chars().all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '-' || c == '.')
        }
        _ => false,
    }
}

/// Starts a program with the desktop user's normal token, without a UAC prompt: true when something was started.
/// `runas` starts executables only (it cannot pass them arguments), so the console and control-panel files go through the
/// shell's own association, which is also where their elevation comes from.
pub fn program(target: &str) -> bool {
    use std::os::windows::process::CommandExt;
    let target = target.trim().trim_matches('"');
    if target.is_empty() {
        return false;
    }
    if matches!(ext_of(target).as_str(), "exe" | "com" | "scr")
        && Command::new("runas").args(["/trustlevel:0x20000", target]).creation_flags(NO_WINDOW).spawn().is_ok()
    {
        return true;
    }
    shell_execute(target)
}

/// This process runs elevated (as administrator).
pub fn elevated() -> bool {
    use windows_sys::Win32::{Foundation::CloseHandle, Security::{GetTokenInformation, TOKEN_QUERY, TokenElevation}, System::Threading::{GetCurrentProcess, OpenProcessToken}};
    unsafe {
        let mut tok = std::ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut tok) == 0 {
            return false;
        }
        let (mut e, mut len) = (0u32, 0u32);
        let ok = GetTokenInformation(tok, TokenElevation, &mut e as *mut u32 as _, 4, &mut len) != 0;
        CloseHandle(tok);
        ok && e != 0
    }
}

/// Starts `exe` with `args` under the desktop's own token (the process behind the "Progman" window: WinRust's desktop or
/// Explorer's, both unelevated), so a program started from an elevated one (the bar, the hotkeys helper) is not elevated
/// too. Windows keeps an elevated window and a normal one apart: no paste or drag and drop between them. False when there
/// is no desktop process, it is elevated itself, or Windows refused (the caller then carries on as it is).
pub fn as_desktop_user(exe: &str, args: &str) -> bool {
    use windows_sys::Win32::{
        Foundation::CloseHandle,
        Security::{DuplicateTokenEx, GetTokenInformation, SecurityImpersonation, TOKEN_ASSIGN_PRIMARY, TOKEN_DUPLICATE, TOKEN_QUERY, TokenElevation, TokenPrimary},
        System::Threading::{CreateProcessWithTokenW, OpenProcess, OpenProcessToken, PROCESS_INFORMATION, PROCESS_QUERY_LIMITED_INFORMATION, STARTUPINFOW},
        UI::WindowsAndMessaging::{FindWindowW, GetWindowThreadProcessId},
    };
    let wide = |s: &str| s.encode_utf16().chain([0]).collect::<Vec<u16>>();
    unsafe {
        let desk = FindWindowW(wide("Progman").as_ptr(), std::ptr::null());
        let mut pid = 0;
        if desk.is_null() || GetWindowThreadProcessId(desk, &mut pid) == 0 || pid == std::process::id() {
            return false;
        }
        let proc_ = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if proc_.is_null() {
            return false;
        }
        let mut tok = std::ptr::null_mut();
        let opened = OpenProcessToken(proc_, TOKEN_DUPLICATE | TOKEN_QUERY, &mut tok) != 0;
        CloseHandle(proc_);
        if !opened {
            return false;
        }
        let (mut e, mut len) = (1u32, 0u32);
        GetTokenInformation(tok, TokenElevation, &mut e as *mut u32 as _, 4, &mut len);
        let mut dup = std::ptr::null_mut();
        let ok = e == 0 && DuplicateTokenEx(tok, TOKEN_ASSIGN_PRIMARY | TOKEN_DUPLICATE | TOKEN_QUERY | 0x0080 /* ADJUST_DEFAULT */ | 0x0100 /* ADJUST_SESSIONID */, std::ptr::null(), SecurityImpersonation, TokenPrimary, &mut dup) != 0;
        CloseHandle(tok);
        if !ok {
            return false;
        }
        let mut cmd = wide(&format!("\"{exe}\" {args}"));
        let mut si: STARTUPINFOW = std::mem::zeroed();
        si.cb = size_of::<STARTUPINFOW>() as u32;
        let mut pi: PROCESS_INFORMATION = std::mem::zeroed();
        let started = CreateProcessWithTokenW(dup, 0, wide(exe).as_ptr(), cmd.as_mut_ptr(), 0, std::ptr::null(), std::ptr::null(), &si, &mut pi) != 0;
        CloseHandle(dup);
        if started {
            CloseHandle(pi.hProcess);
            CloseHandle(pi.hThread);
        }
        started
    }
}

/// The shell's own "open" association: the last resort, and the normal way for `.msc` / `.cpl` / `.bat`.
fn shell_execute(target: &str) -> bool {
    use std::ptr::{null, null_mut};
    use windows_sys::Win32::{UI::Shell::ShellExecuteW, UI::WindowsAndMessaging::SW_SHOWNORMAL};
    let wide = |s: &str| s.encode_utf16().chain([0]).collect::<Vec<u16>>();
    let (verb, file) = (wide("open"), wide(target));
    unsafe { ShellExecuteW(null_mut(), verb.as_ptr(), file.as_ptr(), null(), null(), SW_SHOWNORMAL) as isize > 32 }
}

#[cfg(test)]
mod tests {
    use super::is_program;

    #[test]
    fn programs_are_told_from_shell_targets() {
        for p in ["wt.exe", "taskmgr.exe", r"C:\Windows\System32\taskmgr.exe", "devmgmt.msc", "powercfg.cpl"] {
            assert!(is_program(p), "{p} is a program");
        }
        // these go through explorer.exe like before: it hands them to the shell without needing to be the shell
        for p in ["shell:MyComputerFolder", "ms-settings:", "https://example.com", r"C:\Users", "app.lnk", "regedit"] {
            assert!(!is_program(p), "{p} is not a program");
        }
    }
}
