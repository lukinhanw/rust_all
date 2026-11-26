//! Opens a Settings page (`ms-settings:...`). Settings is a Store app: it opens only while Explorer is the shell (activation
//! fails with 0x80040900 otherwise, also with a shell window of our own registered). So with rust-explorer in place of Explorer:
//! start Explorer for as long as Settings is open, then end it again, which keeps the RAM saving. Done on a background thread:
//! it waits for processes, a hook or UI thread must not. Shared by Win+I (reshell) and the topbar's menus.
use crate::proc;
use std::{os::windows::process::CommandExt, process::{Command, Stdio}, thread::sleep, time::Duration};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM},
    UI::{
        Input::KeyboardAndMouse::keybd_event,
        WindowsAndMessaging::{EnumWindows, FindWindowExW, GetShellWindow, GetWindow, GetWindowThreadProcessId, IsHungAppWindow, IsIconic, IsWindowVisible, SetForegroundWindow, ShowWindow},
    },
};

const SETTINGS: &str = "SystemSettings.exe";
const ENROLL: &str = "CredentialEnrollmentManager.exe"; // the PIN / Windows Hello setup dialog

fn w(s: &str) -> Vec<u16> {
    s.encode_utf16().chain([0]).collect()
}

fn pid_of(h: HWND) -> u32 {
    let mut p = 0;
    unsafe { GetWindowThreadProcessId(h, &mut p) };
    p
}

fn shell_pid() -> Option<u32> {
    let h = unsafe { GetShellWindow() };
    (!h.is_null()).then(|| pid_of(h))
}

fn explorer_shell() -> bool {
    shell_pid().is_some_and(|p| proc::list().iter().any(|(i, n)| *i == p && n.eq_ignore_ascii_case("explorer.exe")))
}

/// The shell's own taskbar comes last (the topbar has a Shell_TrayWnd of its own, so only one that belongs to the shell process counts).
fn explorer_tray() -> bool {
    let Some(sp) = shell_pid() else { return false };
    let cls = w("Shell_TrayWnd");
    let mut t = std::ptr::null_mut();
    loop {
        t = unsafe { FindWindowExW(std::ptr::null_mut(), t, cls.as_ptr(), std::ptr::null()) };
        if t.is_null() {
            return false;
        }
        if pid_of(t) == sp {
            return true;
        }
    }
}

/// A fullscreen game keeps the screen: Settings opens behind it unless it is brought to the front.
fn raise() {
    let Some(sp) = proc::pids(SETTINGS).first().copied() else { return };
    let cls = w("Windows.UI.Core.CoreWindow");
    let mut c = std::ptr::null_mut();
    loop {
        c = unsafe { FindWindowExW(std::ptr::null_mut(), c, cls.as_ptr(), std::ptr::null()) };
        if c.is_null() {
            return;
        }
        if pid_of(c) == sp {
            unsafe {
                let mut o = GetWindow(c, 4); // GW_OWNER
                if o.is_null() {
                    o = c;
                }
                keybd_event(0x12, 0, 0, 0); // an Alt tap lets SetForegroundWindow through
                keybd_event(0x12, 0, 2, 0);
                ShowWindow(o, 9); // SW_RESTORE
                SetForegroundWindow(o);
            }
            return;
        }
    }
}

unsafe extern "system" fn hung_cb(h: HWND, l: LPARAM) -> i32 {
    let d = unsafe { &mut *(l as *mut (u32, bool)) };
    if pid_of(h) == d.0 && unsafe { IsWindowVisible(h) != 0 && IsHungAppWindow(h) != 0 } {
        d.1 = true;
    }
    1
}

fn hung(pid: u32) -> bool {
    let mut d = (pid, false);
    unsafe { EnumWindows(Some(hung_cb), &mut d as *mut _ as LPARAM) };
    d.1
}

/// A window the user can see: visible and not cloaked by the shell (a minimized frame counts too).
fn seen(h: HWND) -> bool {
    use windows_sys::Win32::Graphics::Dwm::{DWMWA_CLOAKED, DwmGetWindowAttribute};
    let mut cloaked = 0u32;
    unsafe { IsWindowVisible(h) != 0 && (IsIconic(h) != 0 || DwmGetWindowAttribute(h, DWMWA_CLOAKED as u32, &mut cloaked as *mut u32 as _, 4) != 0 || cloaked == 0) }
}

/// Whether a Settings process shows a window: its CoreWindow, or the frame that owns it.
fn showing(pid: u32) -> bool {
    let cls = w("Windows.UI.Core.CoreWindow");
    let mut c = std::ptr::null_mut();
    loop {
        c = unsafe { FindWindowExW(std::ptr::null_mut(), c, cls.as_ptr(), std::ptr::null()) };
        if c.is_null() {
            return false;
        }
        if pid_of(c) == pid {
            let o = unsafe { GetWindow(c, 4) }; // GW_OWNER: the ApplicationFrameWindow
            if seen(c) || (!o.is_null() && seen(o)) {
                return true;
            }
        }
    }
}

/// A Settings process that is no use: hung (opened before the shell was ready), or closed but kept alive suspended with
/// its window cloaked. Either swallows every later launch, so the page never shows, and a closed one never exits, so the
/// Explorer started for it was never ended.
fn stale(pid: u32) -> bool {
    hung(pid) || !showing(pid)
}

fn any_hung() -> bool {
    proc::pids(SETTINGS).into_iter().any(hung)
}

/// Before a launch: every Settings process that does not show a window. After it: only hung ones (a fresh one has no window yet).
fn kill_stale() {
    proc::pids(SETTINGS).into_iter().filter(|&p| stale(p)).for_each(proc::kill);
}

fn kill_hung() {
    proc::pids(SETTINGS).into_iter().filter(|&p| hung(p)).for_each(proc::kill);
}

fn wait(n: u32, mut done: impl FnMut() -> bool) {
    for _ in 0..n {
        if done() {
            return;
        }
        sleep(Duration::from_millis(100));
    }
}

/// Marks this session's autostart as done, the way Explorer did itself on older builds: an Explorer started as the shell
/// runs the Run keys and the Startup folder again otherwise (a second Chrome window each time Settings opened). The keys
/// are volatile: they go with the session.
fn mark_autostart_done() {
    use windows_sys::Win32::System::{Registry::*, RemoteDesktop::ProcessIdToSessionId};
    let mut sid = 0u32;
    unsafe { ProcessIdToSessionId(std::process::id(), &mut sid) };
    for name in ["RunStuffHasBeenRun", "StartupHasBeenRun"] {
        let key = w(&format!(r"Software\Microsoft\Windows\CurrentVersion\Explorer\SessionInfo\{sid}\{name}"));
        let mut h: HKEY = std::ptr::null_mut();
        if unsafe { RegCreateKeyExW(HKEY_CURRENT_USER, key.as_ptr(), 0, std::ptr::null(), REG_OPTION_VOLATILE, KEY_WRITE, std::ptr::null(), &mut h, std::ptr::null_mut()) } == 0 {
            unsafe { RegCloseKey(h) };
        }
    }
}

fn has_desktop(p: u32) -> bool {
    proc::cmdline(p).is_some_and(|c| c.contains("--desktop"))
}

const LENT: &str = "Local\\WinRust-ExplorerLent";

/// True while a Settings page holds Explorer as the shell (rs-shell must not raise our desktop then: a desktop raised under it
/// leaves Settings cloaked, and Explorer stays forever waiting for a window that never shows). A named object, set before our
/// desktop is ended, so there is no moment where the desktop is gone and Explorer not yet visible as the shell.
pub fn explorer_lent() -> bool {
    use windows_sys::Win32::{Foundation::{CloseHandle, ERROR_ACCESS_DENIED, GetLastError}, System::Threading::OpenMutexW};
    let h = unsafe { OpenMutexW(0x0010_0000 /* SYNCHRONIZE */, 0, w(LENT).as_ptr()) };
    if h.is_null() {
        return unsafe { GetLastError() } == ERROR_ACCESS_DENIED; // made by the elevated bar: exists, just not ours to open
    }
    unsafe { CloseHandle(h) };
    true
}

fn run(uri: &str) {
    kill_stale();
    let mut started = false;
    let mut lent = std::ptr::null_mut();
    if !explorer_shell() {
        lent = unsafe { windows_sys::Win32::System::Threading::CreateMutexW(std::ptr::null(), 0, w(LENT).as_ptr()) };
        // our desktop is a window of class Progman too: with it up Explorer thinks a shell exists and starts as a plain folder host
        proc::pids("rust-explorer.exe").into_iter().filter(|&p| has_desktop(p)).for_each(proc::kill);
        // a frame host left from an earlier Explorer stays tied to that dead shell: the Settings frame it hosts is cloaked for
        // good. Without a shell its windows are not on screen anyway; COM starts a fresh one for the new Explorer.
        proc::pids("ApplicationFrameHost.exe").into_iter().for_each(proc::kill);
        mark_autostart_done();
        let _ = proc::start_shell_explorer();
        wait(200, explorer_shell);
        // the shell window appears before the immersive shell services behind it are up
        wait(150, explorer_tray);
        sleep(Duration::from_secs(1));
        started = true;
    }
    // through explorer.exe: the page opens with the user's normal token even when this process is elevated
    for _ in 0..2 {
        let _ = Command::new(r"C:\Windows\explorer.exe").arg(uri).status();
        wait(80, || !proc::pids(SETTINGS).is_empty());
        sleep(Duration::from_secs(1));
        raise();
        sleep(Duration::from_secs(3));
        raise();
        if !any_hung() {
            break;
        }
        kill_hung();
    }
    if started {
        // ends when Settings is closed, or when it hangs for good
        // the PIN / Windows Hello dialog is a process of its own hosted by Explorer's immersive shell, and Settings looks hung
        // while it waits on it: Explorer stays until both are gone (ending it under the dialog left it blank, #110)
        let (mut bad, mut shown, mut waited) = (0, false, 0);
        loop {
            let enrolling = !proc::pids(ENROLL).is_empty();
            // never on screen within a minute: it will not show any more (Explorer must not stay as the shell for good)
            if !enrolling && (bad >= 5 || proc::pids(SETTINGS).is_empty() || (!shown && waited >= 30)) {
                break;
            }
            sleep(Duration::from_secs(2));
            waited += 1;
            // closing Settings leaves its process suspended with the window cloaked: that counts as closed, but only once it
            // was on screen (while the blue splash shows, its window is still cloaked)
            let on_screen = proc::pids(SETTINGS).into_iter().any(showing);
            shown |= on_screen;
            bad = if !enrolling && (any_hung() || (shown && !on_screen)) { bad + 1 } else { 0 };
        }
        kill_stale();
        proc::kill_shell_explorers();
        let rx = std::path::PathBuf::from(std::env::var_os("ProgramFiles").unwrap_or_default()).join(r"rust-explorer\rust-explorer.exe");
        if rx.is_file() && !proc::pids("rust-explorer.exe").into_iter().any(has_desktop) {
            let _ = Command::new(rx).arg("--desktop").stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn();
        }
    }
    if !lent.is_null() {
        unsafe { windows_sys::Win32::Foundation::CloseHandle(lent) };
    }
}

/// The rust-settings page that stands for an exact `ms-settings:` address (a bare `ms-settings:` is Win+I: the first page).
/// Only these: any other address keeps going to Windows' own Settings.
fn own_page(uri: &str) -> Option<&'static str> {
    match uri.trim().to_ascii_lowercase().as_str() {
        "ms-settings:" | "ms-settings:display" | "ms-settings:nightlight" => Some("display"),
        "ms-settings:sound" => Some("sound"),
        "ms-settings:personalization" | "ms-settings:colors" | "ms-settings:personalization-colors" | "ms-settings:themes" => Some("personalization"),
        "ms-settings:powersleep" | "ms-settings:storagesense" | "ms-settings:batterysaver" => Some("power"),
        "ms-settings:network-status" | "ms-settings:network-wifi" | "ms-settings:network-wifisettings" | "ms-settings:network-vpn" | "ms-settings:network-proxy" => Some("network"),
        "ms-settings:bluetooth" | "ms-settings:network-airplanemode" => Some("bluetooth"),
        "ms-settings:appsfeatures" => Some("apps"),
        "ms-settings:windowsupdate" => Some("update"),
        "ms-settings:about" => Some("about"),
        _ => None,
    }
}

/// rust-settings.exe where the installer puts it: Program Files is writable by administrators only (a user-writable
/// folder would let any program swap the exe that Win+I starts).
fn own_exe() -> Option<std::path::PathBuf> {
    let exe = std::path::PathBuf::from(std::env::var_os("ProgramFiles")?).join("rust-settings").join("rust-settings.exe");
    exe.is_file().then_some(exe)
}

/// A second call while the first still waits is harmless.
pub fn open(uri: &str) {
    if let (Some(page), Some(exe)) = (own_page(uri), own_exe()) {
        if Command::new(exe).arg(page).stdin(Stdio::null()).creation_flags(0x0800_0000).spawn().is_ok() {
            return;
        }
    }
    let uri = uri.to_string();
    std::thread::spawn(move || run(&uri)); // shortcut: dies with the host process, which is long-lived (reshell, topbar)
}

#[cfg(test)]
mod tests {
    use super::own_page;

    #[test]
    fn lent_marker_lives_as_long_as_its_handle() {
        use windows_sys::Win32::{Foundation::CloseHandle, System::Threading::CreateMutexW};
        let h = unsafe { CreateMutexW(std::ptr::null(), 0, super::w(super::LENT).as_ptr()) };
        assert!(!h.is_null());
        assert!(super::explorer_lent());
        unsafe { CloseHandle(h) };
        assert!(!super::explorer_lent(), "gone with the last handle (no Settings page is open during the test)");
    }

    #[test]
    fn only_exact_addresses_go_to_rust_settings() {
        assert_eq!(own_page("ms-settings:"), Some("display"));
        assert_eq!(own_page("MS-Settings:Sound"), Some("sound"));
        assert_eq!(own_page("ms-settings:themes"), Some("personalization"));
        assert_eq!(own_page("ms-settings:powersleep"), Some("power"));
        assert_eq!(own_page("ms-settings:network-status"), Some("network"));
        assert_eq!(own_page("ms-settings:personalization-background"), None);
        assert_eq!(own_page("ms-settings:network-wifi"), Some("network"));
        assert_eq!(own_page("ms-settings:bluetooth"), Some("bluetooth"));
        assert_eq!(own_page("ms-settings:windowsupdate"), Some("update"));
        assert_eq!(own_page("ms-settings:easeofaccess"), None);
        assert_eq!(own_page("ms-settings:sound; calc"), None);
    }
}
