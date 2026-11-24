//! Right-click menus drawn like Windows 11's: owner-drawn rows (taller, Fluent icons, a rounded hover highlight, chevrons
//! for submenus, a tick / dot for checked items) on a menu window with rounded corners. Light or dark as ui::menu_dark().
use super::*;
use windows_sys::Win32::{Graphics::{Dwm::DwmSetWindowAttribute, Gdi::*}, System::Threading::GetCurrentThreadId};

/// No mark, a tick (checkbox on), or a dot (the chosen one of a group).
#[derive(Clone, Copy, PartialEq)]
pub enum Mark {
    None,
    Tick,
    Dot,
}

struct Item {
    text: String,
    glyph: Option<&'static str>,
    sub: bool,
    sep: bool,
    mark: Mark,
}

thread_local! {
    // the rows of the menu being built / shown; their addresses are the items' dwItemData
    static ITEMS: std::cell::RefCell<Vec<Box<Item>>> = const { std::cell::RefCell::new(Vec::new()) };
}

fn ours(data: usize) -> bool {
    data != 0 && ITEMS.with_borrow(|v| v.iter().any(|b| &**b as *const Item as usize == data))
}

/// (background, hover, text, disabled text, separator) as COLORREF.
fn colors() -> (u32, u32, u32, u32, u32) {
    if ui::menu_dark() { (0x2C2C2C, 0x3D3D3D, 0xFFFFFF, 0x8C8C8C, 0x454545) } else { (0xF9F9F9, 0xEBEBEB, 0x1A1A1A, 0x9A9A9A, 0xE0E0E0) }
}

#[allow(unsafe_op_in_unsafe_fn)]
unsafe fn insert(m: HMENU, id: usize, it: Item, sub: Option<HMENU>, disabled: bool) {
    let mark = it.mark;
    let b = Box::new(it);
    let data = &*b as *const Item as usize;
    ITEMS.with_borrow_mut(|v| v.push(b));
    let mut mi: MENUITEMINFOW = std::mem::zeroed();
    mi.cbSize = size_of::<MENUITEMINFOW>() as u32;
    mi.fMask = MIIM_FTYPE | MIIM_ID | MIIM_STATE | MIIM_DATA | if sub.is_some() { MIIM_SUBMENU } else { 0 };
    mi.fType = MFT_OWNERDRAW;
    mi.fState = if disabled { MFS_DISABLED } else { 0 } | if mark != Mark::None { MFS_CHECKED } else { 0 };
    mi.wID = id as u32;
    mi.hSubMenu = sub.unwrap_or(ptr::null_mut());
    mi.dwItemData = data;
    InsertMenuItemW(m, GetMenuItemCount(m).max(0) as u32, 1, &mi);
}

/// A row: a command `id`, or a submenu `sub`; `glyph` an icon-font character.
#[allow(unsafe_op_in_unsafe_fn)]
pub unsafe fn add(m: HMENU, id: usize, text: &str, glyph: Option<&'static str>, sub: Option<HMENU>, mark: Mark) {
    insert(m, id, Item { text: text.to_string(), glyph, sub: sub.is_some(), sep: false, mark }, sub, false);
}

/// A thin line between groups.
#[allow(unsafe_op_in_unsafe_fn)]
pub unsafe fn sep(m: HMENU) {
    insert(m, 0, Item { text: String::new(), glyph: None, sub: false, sep: true, mark: Mark::None }, None, true);
}

/// Paints the menu window's frame (the classic square border Windows still draws inside it) in the menu's background:
/// with DWM's rounded corners on top, only Windows 11's own thin rounded outline is left.
#[allow(unsafe_op_in_unsafe_fn)]
unsafe fn paint_frame(h: HWND, dc: HDC) {
    let (mut wr, mut cr): (RECT, RECT) = (std::mem::zeroed(), std::mem::zeroed());
    GetWindowRect(h, &mut wr);
    GetClientRect(h, &mut cr);
    let mut p = POINT { x: 0, y: 0 };
    ClientToScreen(h, &mut p);
    let (ox, oy) = (p.x - wr.left, p.y - wr.top);
    let saved = SaveDC(dc);
    ExcludeClipRect(dc, ox, oy, ox + cr.right, oy + cr.bottom);
    let b = CreateSolidBrush(colors().0);
    FillRect(dc, &RECT { left: 0, top: 0, right: wr.right - wr.left, bottom: wr.bottom - wr.top }, b);
    DeleteObject(b as HGDIOBJ);
    RestoreDC(dc, saved);
}

/// Rounded corners like Windows 11's menus (DWMWA_WINDOW_CORNER_PREFERENCE = DWMWCP_ROUND). Asked again whenever the
/// menu window is created or shown: DWM ignores it while the window is still being made.
#[allow(unsafe_op_in_unsafe_fn)]
unsafe fn corners(h: HWND) {
    let pref: u32 = 2;
    DwmSetWindowAttribute(h, 33, &pref as *const u32 as _, 4);
}

unsafe extern "system" fn frame_proc(h: HWND, m: u32, wp: WPARAM, lp: LPARAM, _id: usize, _data: usize) -> LRESULT {
    unsafe {
        match m {
            WM_NCPAINT => {
                let dc = GetWindowDC(h);
                paint_frame(h, dc);
                ReleaseDC(h, dc);
                0
            }
            // the open / fade animation draws the menu through WM_PRINT: the frame goes the same way there
            WM_PRINT => {
                let r = DefSubclassProc(h, m, wp, lp);
                if lp as i32 & PRF_NONCLIENT != 0 { paint_frame(h, wp as HDC); }
                r
            }
            WM_CREATE | WM_SHOWWINDOW => {
                let r = DefSubclassProc(h, m, wp, lp);
                corners(h);
                r
            }
            WM_WINDOWPOSCHANGING => {
                let p = &*(lp as *const WINDOWPOS);
                if p.flags & SWP_SHOWWINDOW != 0 { corners(h); }
                DefSubclassProc(h, m, wp, lp)
            }
            WM_NCDESTROY => {
                RemoveWindowSubclass(h, Some(frame_proc), 1);
                DefSubclassProc(h, m, wp, lp)
            }
            _ => DefSubclassProc(h, m, wp, lp),
        }
    }
}

/// Makes a menu window look like Windows 11's: no old square shadow, no square frame inside, rounded corners. `made` =
/// the window already exists on screen (found at WM_ENTERIDLE): it is redrawn. Once per window.
#[allow(unsafe_op_in_unsafe_fn)]
unsafe fn style(h: HWND, made: bool) {
    let mut cls = [0u16; 16];
    let n = GetClassNameW(h, cls.as_mut_ptr(), 16).max(0) as usize;
    if String::from_utf16_lossy(&cls[..n]) != "#32768" {
        return;
    }
    if made { corners(h); }
    if DONE.with_borrow(|d| d.contains(&(h as isize))) {
        return;
    }
    DONE.with_borrow_mut(|d| d.push(h as isize));
    // the menu class's CS_DROPSHADOW is the old square shadow (a separate window right and below), which does not follow
    // the rounded corners: off for this process's menus (its copy of the system class); DWM draws a soft one instead
    let st = GetClassLongPtrW(h, GCL_STYLE);
    if st & CS_DROPSHADOW as usize != 0 { SetClassLongPtrW(h, GCL_STYLE, (st & !(CS_DROPSHADOW as usize)) as isize); }
    SetWindowSubclass(h, Some(frame_proc), 1, 0);
    if made { RedrawWindow(h, ptr::null(), ptr::null_mut(), RDW_FRAME | RDW_INVALIDATE | RDW_UPDATENOW); }
}

thread_local! {
    static DONE: std::cell::RefCell<Vec<isize>> = const { std::cell::RefCell::new(Vec::new()) }; // menu windows already styled
}

/// Menu windows are created on this thread while TrackPopupMenu runs: style each as it is made, before it is shown (the
/// first menu otherwise still got the square shadow). A CBT hook sees the creation; a message hook does not see menus'.
unsafe extern "system" fn hook(code: i32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        if code == HCBT_CREATEWND as i32 {
            style(wp as HWND, false);
        }
        CallNextHookEx(ptr::null_mut(), code, wp, lp)
    }
}

/// WM_ENTERIDLE while a menu is open: makes sure every menu window of this thread is styled (rounded) by now.
pub fn round_open() {
    unsafe extern "system" fn each(h: HWND, _: LPARAM) -> windows_sys::core::BOOL {
        unsafe { style(h, true) };
        1
    }
    unsafe { EnumThreadWindows(GetCurrentThreadId(), Some(each), 0) };
}

/// Shows the menu at the cursor and returns the picked id (0 = none). Destroys the menu (and its submenus).
#[allow(unsafe_op_in_unsafe_fn)]
pub unsafe fn track(m: HMENU, owner: HWND) -> i32 {
    let br = CreateSolidBrush(colors().0);
    let mut info: MENUINFO = std::mem::zeroed();
    info.cbSize = size_of::<MENUINFO>() as u32;
    info.fMask = MIM_BACKGROUND | MIM_STYLE | MIM_APPLYTOSUBMENUS;
    info.hbrBack = br;
    info.dwStyle = MNS_NOCHECK; // we draw the marks; no empty check column
    SetMenuInfo(m, &info);
    let hk = SetWindowsHookExW(WH_CBT, Some(hook), ptr::null_mut(), GetCurrentThreadId());
    let mut pt: POINT = std::mem::zeroed();
    GetCursorPos(&mut pt);
    let id = TrackPopupMenu(m, TPM_RETURNCMD | TPM_RIGHTBUTTON, pt.x, pt.y, 0, owner, ptr::null());
    if !hk.is_null() { UnhookWindowsHookEx(hk); }
    DestroyMenu(m);
    DeleteObject(br as HGDIOBJ);
    ITEMS.with_borrow_mut(|v| v.clear());
    DONE.with_borrow_mut(|d| d.clear());
    id
}

/// WM_MEASUREITEM for our rows. false = not ours.
#[allow(unsafe_op_in_unsafe_fn)]
pub unsafe fn measure(lp: LPARAM) -> bool {
    let mi = &mut *(lp as *mut MEASUREITEMSTRUCT);
    if mi.CtlType != ODT_MENU || !ours(mi.itemData) { return false; }
    let it = &*(mi.itemData as *const Item);
    if it.sep {
        mi.itemWidth = ui::sc(10) as u32;
        mi.itemHeight = ui::sc(9) as u32;
        return true;
    }
    let dc = GetDC(ptr::null_mut());
    let f = ui::font(14, 400, winrust_common::style::ui_face()) as HFONT;
    let of = SelectObject(dc, f as HGDIOBJ);
    let t = w(&it.text);
    let mut sz: SIZE = std::mem::zeroed();
    GetTextExtentPoint32W(dc, t.as_ptr(), (t.len() - 1) as i32, &mut sz);
    SelectObject(dc, of);
    DeleteObject(f as HGDIOBJ);
    ReleaseDC(ptr::null_mut(), dc);
    // left: padding, icon, gap; right: room for the chevron
    mi.itemWidth = (ui::sc(46) + sz.cx + ui::sc(48)).max(ui::sc(220)) as u32;
    mi.itemHeight = ui::sc(34) as u32;
    true
}

/// WM_DRAWITEM for our rows. false = not ours.
#[allow(unsafe_op_in_unsafe_fn)]
pub unsafe fn draw(lp: LPARAM) -> bool {
    let d = &*(lp as *const DRAWITEMSTRUCT);
    if d.CtlType != ODT_MENU || !ours(d.itemData) { return false; }
    let it = &*(d.itemData as *const Item);
    let (dc, r) = (d.hDC, d.rcItem);
    let (bg, hover, fg, dim, line) = colors();
    let b = CreateSolidBrush(bg);
    FillRect(dc, &r, b);
    DeleteObject(b as HGDIOBJ);
    if it.sep {
        let y = (r.top + r.bottom) / 2;
        let l = CreateSolidBrush(line);
        FillRect(dc, &RECT { left: r.left, top: y, right: r.right, bottom: y + ui::sc(1).max(1) }, l);
        DeleteObject(l as HGDIOBJ);
        ExcludeClipRect(dc, r.left, r.top, r.right, r.bottom);
        return true;
    }
    let disabled = d.itemState & ODS_DISABLED != 0;
    if d.itemState & ODS_SELECTED != 0 && !disabled {
        ui::fill_raw(dc, RECT { left: r.left + ui::sc(4), top: r.top + ui::sc(2), right: r.right - ui::sc(4), bottom: r.bottom - ui::sc(2) }, hover, ui::sc(8));
    }
    let c = if disabled { dim } else { fg };
    SetBkMode(dc, TRANSPARENT as i32);
    let x = r.left + ui::sc(14);
    let (fi, ft, fs) = (ui::font(16, 400, winrust_common::style::icon_face()) as HFONT, ui::font(14, 400, winrust_common::style::ui_face()) as HFONT, ui::font(10, 400, winrust_common::style::icon_face()) as HFONT);
    let of = SelectObject(dc, fi as HGDIOBJ);
    let icon = RECT { left: x, top: r.top, right: x + ui::sc(18), bottom: r.bottom };
    match (it.glyph, it.mark) {
        (Some(g), _) => ui::text(dc, g, icon, c, DT_CENTER),
        (None, Mark::Tick) => ui::text(dc, "\u{E73E}", icon, c, DT_CENTER),
        (None, Mark::Dot) => {
            let (cx, cy, dd) = ((icon.left + icon.right) / 2, (r.top + r.bottom) / 2, ui::sc(6));
            ui::fill_raw(dc, RECT { left: cx - dd / 2, top: cy - dd / 2, right: cx - dd / 2 + dd, bottom: cy - dd / 2 + dd }, c, dd);
        }
        (None, Mark::None) => {}
    }
    SelectObject(dc, ft as HGDIOBJ);
    ui::text(dc, &it.text, RECT { left: x + ui::sc(32), top: r.top, right: r.right - ui::sc(34), bottom: r.bottom }, c, DT_LEFT);
    if it.sub {
        SelectObject(dc, fs as HGDIOBJ);
        ui::text(dc, "\u{E76C}", RECT { left: r.right - ui::sc(34), top: r.top, right: r.right - ui::sc(14), bottom: r.bottom }, c, DT_RIGHT);
    }
    SelectObject(dc, of);
    for f in [fi, ft, fs] { DeleteObject(f as HGDIOBJ); }
    ExcludeClipRect(dc, r.left, r.top, r.right, r.bottom); // keeps Windows' own submenu arrow out of the row
    true
}
