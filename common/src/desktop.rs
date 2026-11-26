//! Win+D without Explorer: minimize every app window, press again to bring the same windows back.
//! Used by reshell (keyboard hook) and rs-shell (when it is the shell).
use std::sync::Mutex;
use windows_sys::{
    core::BOOL,
    Win32::{Foundation::{HWND, LPARAM}, Graphics::Dwm::DwmGetWindowAttribute, UI::WindowsAndMessaging::*},
};

static HIDDEN: Mutex<Vec<isize>> = Mutex::new(Vec::new());

/// Anything open on screen means "minimize": after a window was brought back by hand the old list must not turn the next
/// press into a silent "restore".
pub fn toggle() {
    let mut hid = HIDDEN.lock().unwrap();
    unsafe {
        let mut v: Vec<isize> = vec![];
        EnumWindows(Some(collect), &mut v as *mut _ as LPARAM);
        if !v.is_empty() {
            v.iter().for_each(|&h| { ShowWindow(h as HWND, SW_MINIMIZE); });
            *hid = v;
        } else {
            hid.iter().for_each(|&h| { if IsWindow(h as HWND) != 0 { ShowWindow(h as HWND, SW_RESTORE); } });
            hid.clear();
        }
    }
}

unsafe extern "system" fn collect(h: HWND, lp: LPARAM) -> BOOL {
    unsafe {
        let mut cloaked = 0u32; // DWMWA_CLOAKED: UWP ghosts that are "visible" but not on screen
        DwmGetWindowAttribute(h, 14, &mut cloaked as *mut _ as _, 4);
        let ex = GetWindowLongW(h, GWL_EXSTYLE) as u32;
        let tool = ex & WS_EX_TOOLWINDOW != 0; // topbar, desktop, tray hosts
        // an untitled window that asks for a taskbar button (WS_EX_APPWINDOW, e.g. DeusOT) is an app window too: left
        // up, a maximized one kept covering the desktop
        let titled = GetWindowTextLengthW(h) > 0 || ex & WS_EX_APPWINDOW != 0;
        if IsWindowVisible(h) != 0 && IsIconic(h) == 0 && !tool && cloaked == 0 && GetWindow(h, GW_OWNER).is_null() && titled && !is_desktop(h) {
            (*(lp as *mut Vec<isize>)).push(h as isize);
        }
        1
    }
}

/// The desktop itself (Explorer's, or the rust-explorer one, which also registers as "Progman") must never be minimized.
fn is_desktop(h: HWND) -> bool {
    let mut cls = [0u16; 32];
    let n = unsafe { GetClassNameW(h, cls.as_mut_ptr(), cls.len() as i32) } as usize;
    is_desktop_class(&String::from_utf16_lossy(&cls[..n]))
}

fn is_desktop_class(c: &str) -> bool {
    matches!(c, "Progman" | "WorkerW")
}

#[cfg(test)]
mod tests {
    #[test]
    fn desktop_classes() {
        assert!(super::is_desktop_class("Progman"));
        assert!(!super::is_desktop_class("Notepad"));
    }
}
