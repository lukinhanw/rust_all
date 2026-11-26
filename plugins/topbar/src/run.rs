//! topbar: run (split out of main.rs; shares its items through `super::*`).
use super::*;

/// The Windows "Run" box. `rundll32 shell32.dll,#61` cannot do it: ordinal 61 is RunFileDlg(hwnd, icon, dir, title,
/// prompt, flags) and rundll32 passes other arguments. Its own thread: the box runs a modal loop until closed.
pub(crate) fn run_dialog() {
    std::thread::spawn(|| unsafe {
        let sh = LoadLibraryW(wide("shell32.dll").as_ptr());
        if let Some(f) = GetProcAddress(sh, 61 as *const u8) {
            type RunFileDlg = unsafe extern "system" fn(HWND, HICON, *const u16, *const u16, *const u16, u32);
            std::mem::transmute::<_, RunFileDlg>(f)(null_mut(), null_mut(), null(), null(), null(), 0);
        }
    });
}

/// Win+R and the Start menu's "Run": rs-launcher's own Run mode when it runs (history, folder completion, the WinRust
/// look), else the Windows box.
pub(crate) unsafe fn run_box() {
    let mut h: HWND = null_mut();
    EnumWindows(Some(find_launcher), &mut h as *mut _ as LPARAM);
    if h.is_null() {
        return run_dialog();
    }
    let mut pid = 0;
    GetWindowThreadProcessId(h, &mut pid);
    AllowSetForegroundWindow(pid); // the card must take the keyboard, or the typing goes to the window below
    PostMessageW(h, WM_HOTKEY, 2, 0); // id 2: Run mode (1 is its Alt+Space)
}

static WIN_D_HELD: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Win key on its own opens our Start. Runs on its own thread so a busy bar can never stall the system keyboard.
pub(crate) unsafe extern "system" fn kb_hook(n: i32, w: WPARAM, l: LPARAM) -> LRESULT {
    if n >= 0 {
        let k = &*(l as *const KBDLLHOOKSTRUCT);
        let down0 = w == WM_KEYDOWN as usize || w == WM_SYSKEYDOWN as usize;
        // without explorer nobody handles the volume keys, and Surface's power + volume-up arrives as an injected Win+PrtSc
        // (the injected events are skipped below, so these two are taken before that check)
        if !EXPLORER_UP.load(Relaxed) {
            let vol = [(VK_VOLUME_UP, 0), (VK_VOLUME_DOWN, 1), (VK_VOLUME_MUTE, 2)].into_iter().find(|&(vk, _)| k.vkCode == vk as u32);
            if let Some((_, v)) = vol {
                if down0 {
                    PostMessageW(BAR.load(Relaxed) as HWND, VOLKEY, v, 0);
                }
                return 1;
            }
            if k.vkCode == VK_SNAPSHOT as u32 && k.flags & LLKHF_INJECTED != 0 {
                if down0 {
                    PostMessageW(BAR.load(Relaxed) as HWND, SNIP, 1, 0);
                }
                return 1;
            }
        }
        if k.flags & LLKHF_INJECTED == 0 {
            // Win+Shift+S is our own region clipper (also under Explorer); without explorer nobody opens the screen
            // capture either: PrtSc (when Windows is set to snip with it; otherwise it copies the screen by itself) does it here
            let held = |vk: u16| GetAsyncKeyState(vk as i32) < 0;
            let snip_key = !EXPLORER_UP.load(Relaxed) && k.vkCode == VK_SNAPSHOT as u32;
            let win_shift_s = k.vkCode == b'S' as u32 && down0 && (held(VK_LWIN) || held(VK_RWIN)) && held(VK_SHIFT);
            if snip_key || win_shift_s {
                if down0 {
                    WIN_ALONE.store(false, Relaxed);
                    if win_shift_s {
                        // a key between Win down and up, so Windows does not open its Start menu on the release
                        keybd_event(0xE8, 0, 0, 0);
                        keybd_event(0xE8, 0, KEYEVENTF_KEYUP, 0);
                    }
                    PostMessageW(BAR.load(Relaxed) as HWND, SNIP, snip_key as usize, 0); // wp 1: PrtSc, 0: Win+Shift+S
                }
                return 1;
            }
            // Win+1..9: the n-th app button of the bar (Windows 11 taskbar behaviour)
            if (0x31..=0x39).contains(&k.vkCode) && (held(VK_LWIN) || held(VK_RWIN)) && !held(VK_CONTROL) && !held(VK_MENU) && !held(VK_SHIFT) {
                if down0 {
                    WIN_ALONE.store(false, Relaxed);
                    keybd_event(0xE8, 0, 0, 0); // a key between Win down and up: no Start menu on release
                    keybd_event(0xE8, 0, KEYEVENTF_KEYUP, 0);
                    PostMessageW(BAR.load(Relaxed) as HWND, WIN_NUM, k.vkCode as usize - 0x31, 0);
                }
                return 1;
            }
            // Win+Space / Win+Shift+Space: without explorer nobody switches the keyboard layout (Alt+Shift still works, the
            // system handles that one itself). Our own Start in front: the system switcher's request never reaches the elevated
            // bar (UIPI drops it), so it is done here too (#105).
            if (!EXPLORER_UP.load(Relaxed) || own_foreground()) && k.vkCode == VK_SPACE as u32 && (held(VK_LWIN) || held(VK_RWIN)) && !held(VK_CONTROL) && !held(VK_MENU) {
                if down0 {
                    WIN_ALONE.store(false, Relaxed);
                    keybd_event(0xE8, 0, 0, 0); // a key between Win down and up: no Start menu on release
                    keybd_event(0xE8, 0, KEYEVENTF_KEYUP, 0);
                    PostMessageW(BAR.load(Relaxed) as HWND, NEXT_LANG, held(VK_SHIFT) as usize, 0);
                }
                return 1;
            }
            // Win+V: without explorer nobody opens the clipboard history; ours does
            if !EXPLORER_UP.load(Relaxed) && k.vkCode == b'V' as u32 && (held(VK_LWIN) || held(VK_RWIN)) && !held(VK_CONTROL) && !held(VK_MENU) && !held(VK_SHIFT) {
                if down0 {
                    WIN_ALONE.store(false, Relaxed);
                    keybd_event(0xE8, 0, 0, 0); // a key between Win down and up: no Start menu on release
                    keybd_event(0xE8, 0, KEYEVENTF_KEYUP, 0);
                    PostMessageW(BAR.load(Relaxed) as HWND, CLIPHIST, 0, 0);
                }
                return 1;
            }
            // Win+D: show the desktop / bring the windows back. Here, not only in reshell: reshell runs without admin, and
            // Windows keeps keys from it while an elevated window has the focus, nor lets it minimize that window.
            if k.vkCode == b'D' as u32 && (held(VK_LWIN) || held(VK_RWIN)) && !held(VK_CONTROL) && !held(VK_MENU) && !held(VK_SHIFT) {
                if down0 {
                    // auto-repeat while held must not minimize and restore again and again
                    if !WIN_D_HELD.swap(true, Relaxed) {
                        WIN_ALONE.store(false, Relaxed);
                        keybd_event(0xE8, 0, 0, 0); // a key between Win down and up: no Start menu on release
                        keybd_event(0xE8, 0, KEYEVENTF_KEYUP, 0);
                        std::thread::spawn(winrust_common::desktop::toggle); // a hung window must not stall the keyboard
                    }
                } else {
                    WIN_D_HELD.store(false, Relaxed);
                }
                return 1;
            }
            if k.vkCode == b'D' as u32 && !down0 {
                WIN_D_HELD.store(false, Relaxed); // Win let go first
            }
            // Win+R: always our Run box, also while Explorer runs (lent to Settings, or not replaced): its own box is the plain one
            if k.vkCode == b'R' as u32 && (held(VK_LWIN) || held(VK_RWIN)) && !held(VK_CONTROL) && !held(VK_MENU) && !held(VK_SHIFT) {
                if down0 {
                    WIN_ALONE.store(false, Relaxed);
                    keybd_event(0xE8, 0, 0, 0); // a key between Win down and up: no Start menu on release
                    keybd_event(0xE8, 0, KEYEVENTF_KEYUP, 0);
                    run_box();
                }
                return 1;
            }
            // Win+X: our power-user menu (the one Start's right click opens); Explorer's needs its own taskbar, which is hidden
            if k.vkCode == b'X' as u32 && (held(VK_LWIN) || held(VK_RWIN)) && !held(VK_CONTROL) && !held(VK_MENU) && !held(VK_SHIFT) {
                if down0 {
                    WIN_ALONE.store(false, Relaxed);
                    keybd_event(0xE8, 0, 0, 0); // a key between Win down and up: no Start menu on release
                    keybd_event(0xE8, 0, KEYEVENTF_KEYUP, 0);
                    PostMessageW(BAR.load(Relaxed) as HWND, WIN_X, 0, 0);
                }
                return 1;
            }
            let down = w == WM_KEYDOWN as usize || w == WM_SYSKEYDOWN as usize;
            let bar = BAR.load(Relaxed) as HWND;
            // Alt+Tab: our own switcher. Without explorer Windows only flips to the next window blindly, and a fullscreen
            // game often takes the focus straight back.
            if k.vkCode == VK_TAB as u32 && k.flags & LLKHF_ALTDOWN != 0 && GetAsyncKeyState(VK_CONTROL as i32) >= 0 {
                if down {
                    SWITCHING.store(true, Relaxed);
                    PostMessageW(bar, ALT_TAB, (GetAsyncKeyState(VK_SHIFT as i32) < 0) as usize, 0);
                }
                return 1;
            }
            if SWITCHING.load(Relaxed) {
                if k.vkCode == VK_ESCAPE as u32 && down {
                    SWITCHING.store(false, Relaxed);
                    PostMessageW(bar, ALT_UP, 1, 0);
                    return 1;
                }
                // the switcher owns these keys while open (they must not reach the app behind it, #92)
                if matches!(k.vkCode as u16, VK_LEFT | VK_RIGHT | VK_UP | VK_DOWN | VK_DELETE | VK_RETURN) {
                    if down {
                        match k.vkCode as u16 {
                            VK_LEFT | VK_UP => { PostMessageW(bar, ALT_TAB, 1, 0); }
                            VK_RIGHT | VK_DOWN => { PostMessageW(bar, ALT_TAB, 0, 0); }
                            VK_DELETE => { PostMessageW(bar, ALT_UP, 2, 0); }
                            _ => {
                                // Enter: switch now; the Alt release afterwards must not open the menu bar
                                SWITCHING.store(false, Relaxed);
                                PostMessageW(bar, ALT_UP, 0, 0);
                                keys(&[0xE8]);
                            }
                        }
                    }
                    return 1;
                }
                if matches!(k.vkCode as u16, VK_LMENU | VK_RMENU | VK_MENU) && !down {
                    SWITCHING.store(false, Relaxed);
                    keys(&[0xE8]); // the target app must not take the lone Alt release as "open the menu bar"
                    PostMessageW(bar, ALT_UP, 0, 0);
                }
            }
            if k.vkCode == VK_LWIN as u32 || k.vkCode == VK_RWIN as u32 {
                if down {
                    WIN_ALONE.store(true, Relaxed);
                } else if WIN_ALONE.swap(false, Relaxed) {
                    // A lone Win *release* is what opens Windows' own Start. An E8 sent from here used to be queued behind
                    // the release already being dispatched, so the shell sometimes saw no other key in time and opened its
                    // Start on top of ours. Swallow the real release and hand the shell an E8 and then a synthetic Win-up:
                    // it sees a key between down and up (so no Start), and the synthetic up still clears the Win key for
                    // everything else (our own hook skips it: injected). A Win+key chord never reaches here: the key down
                    // clears WIN_ALONE before the release.
                    keys(&[0xE8]);
                    key_up(k.vkCode as u16);
                    PostMessageW(BAR.load(Relaxed) as HWND, WIN_START, 0, 0);
                    return 1;
                }
            } else if down {
                WIN_ALONE.store(false, Relaxed); // Win+something else: a shortcut, not a Start press
            }
        }
    }
    CallNextHookEx(null_mut(), n, w, l)
}

// ---------- drawing ----------
