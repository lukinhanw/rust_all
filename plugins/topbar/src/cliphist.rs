//! Win+V without Explorer: the clipboard history (#65). Windows keeps its history in a shell panel Explorer opens; with
//! the shell gone nothing shows it. The bar listens to the clipboard itself (text and pictures, the last `MAX` items, in
//! memory like Windows' unpinned items), and Win+V opens a panel next to the text cursor: Up / Down and the pointer pick,
//! Enter or a click pastes into the window that was in front, Delete or × drops an item, Esc or a click elsewhere closes.
use super::*;
use windows_sys::Win32::System::{
    DataExchange::{AddClipboardFormatListener, CloseClipboard, EmptyClipboard, GetClipboardData, IsClipboardFormatAvailable, OpenClipboard, RegisterClipboardFormatW, SetClipboardData},
    Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock},
};

/// How many items the history keeps (Windows keeps 25).
const MAX: usize = 25;
/// Pictures bigger than this are not kept (Windows' own limit is 4 MB).
const MAX_DIB: usize = 4 << 20;
const CF_UNICODETEXT: u32 = 13;
const CF_DIB: u32 = 8;
const WM_CLIPBOARDUPDATE: u32 = 0x031D;

#[derive(Clone, PartialEq)]
enum Item {
    Text(String),
    /// A packed DIB (BITMAPINFOHEADER, colour table if any, pixels): what CF_DIB holds.
    Pic(Vec<u8>),
}

thread_local! {
    static HIST: RefCell<Vec<Item>> = const { RefCell::new(Vec::new()) };
    static LISTENER: Cell<HWND> = const { Cell::new(null_mut()) };
    static PANEL: Cell<HWND> = const { Cell::new(null_mut()) };
    static PREV: Cell<HWND> = const { Cell::new(null_mut()) }; // the window in front when Win+V was pressed: it gets the paste
    static SEL: Cell<usize> = const { Cell::new(0) };
    static TOP: Cell<i32> = const { Cell::new(0) }; // scroll offset of the list, px
    static HOT: Cell<i32> = const { Cell::new(-1) }; // pointer over: item index, CLEAR, or -1
    static OPEN_PT: Cell<Option<(i32, i32)>> = const { Cell::new(None) }; // pointer when the panel opened (not a "move")
}

const CLEAR: i32 = -2; // the "Clear all" button
const PASTE_TIMER: usize = 1;

/// Formats apps put on the clipboard to keep it out of history (password managers): ExcludeClipboardContentFromMonitorProcessing,
/// or CanIncludeInClipboardHistory = 0.
unsafe fn excluded() -> bool {
    let ex = RegisterClipboardFormatW(wide("ExcludeClipboardContentFromMonitorProcessing").as_ptr());
    if IsClipboardFormatAvailable(ex) != 0 {
        return true;
    }
    let can = RegisterClipboardFormatW(wide("CanIncludeInClipboardHistory").as_ptr());
    if IsClipboardFormatAvailable(can) != 0 {
        let h = GetClipboardData(can);
        if !h.is_null() {
            let p = GlobalLock(h) as *const u32;
            let v = if p.is_null() { 1 } else { *p };
            GlobalUnlock(h);
            return v == 0;
        }
    }
    false
}

/// What is on the clipboard now, if it is something the history keeps.
unsafe fn read(owner: HWND) -> Option<Item> {
    let mut opened = false;
    for _ in 0..5 {
        if OpenClipboard(owner) != 0 {
            opened = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20)); // the app that copied may still hold it
    }
    if !opened {
        return None;
    }
    let item = (|| {
        if excluded() {
            return None;
        }
        if IsClipboardFormatAvailable(CF_UNICODETEXT) != 0 {
            let h = GetClipboardData(CF_UNICODETEXT);
            let p = GlobalLock(h) as *const u16;
            if p.is_null() {
                return None;
            }
            let n = (0..GlobalSize(h) / 2).take_while(|&i| *p.add(i) != 0).count();
            let s = String::from_utf16_lossy(std::slice::from_raw_parts(p, n));
            GlobalUnlock(h);
            return (!s.trim().is_empty()).then_some(Item::Text(s));
        }
        if IsClipboardFormatAvailable(CF_DIB) != 0 {
            let h = GetClipboardData(CF_DIB);
            let n = GlobalSize(h);
            if n < size_of::<BITMAPINFOHEADER>() || n > MAX_DIB {
                return None;
            }
            let p = GlobalLock(h) as *const u8;
            if p.is_null() {
                return None;
            }
            let v = std::slice::from_raw_parts(p, n).to_vec();
            GlobalUnlock(h);
            return Some(Item::Pic(v));
        }
        None
    })();
    CloseClipboard();
    item
}

/// Puts an item back on the clipboard (the listener then moves it to the top of the history, like Windows does).
unsafe fn write(owner: HWND, it: &Item) -> bool {
    if OpenClipboard(owner) == 0 {
        return false;
    }
    EmptyClipboard();
    let (fmt, bytes): (u32, Vec<u8>) = match it {
        Item::Text(s) => (CF_UNICODETEXT, s.encode_utf16().chain([0]).flat_map(u16::to_le_bytes).collect()),
        Item::Pic(d) => (CF_DIB, d.clone()),
    };
    let h = GlobalAlloc(GMEM_MOVEABLE, bytes.len());
    let ok = !h.is_null() && {
        let p = GlobalLock(h) as *mut u8;
        if !p.is_null() {
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), p, bytes.len());
            GlobalUnlock(h);
        }
        !p.is_null() && !SetClipboardData(fmt, h).is_null()
    };
    CloseClipboard();
    ok
}

/// The newest item first; a copy of something already kept moves it up instead of adding a twin.
fn remember(it: Item) {
    HIST.with_borrow_mut(|h| {
        h.retain(|x| *x != it);
        h.insert(0, it);
        h.truncate(MAX);
    });
}

unsafe extern "system" fn listener_proc(h: HWND, m: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if m == WM_CLIPBOARDUPDATE {
        if let Some(it) = read(h) {
            remember(it);
            let p = PANEL.get();
            if IsWindowVisible(p) != 0 {
                InvalidateRect(p, null(), 0);
            }
        }
        return 0;
    }
    DefWindowProcW(h, m, wp, lp)
}

/// Starts listening to the clipboard and makes the (hidden) panel. On the bar's thread, once.
pub(crate) unsafe fn init(hi: HINSTANCE) {
    let lc = wide("topbar_cliplisten");
    RegisterClassW(&WNDCLASSW { lpfnWndProc: Some(listener_proc), hInstance: hi, lpszClassName: lc.as_ptr(), ..zeroed() });
    let l = CreateWindowExW(0, lc.as_ptr(), null(), 0, 0, 0, 0, 0, HWND_MESSAGE, null_mut(), hi, null());
    AddClipboardFormatListener(l);
    LISTENER.set(l);
    if let Some(it) = read(l) {
        remember(it); // what is on the clipboard already is the first item, as after a restart under Windows
    }
    let pc = wide("topbar_cliphist");
    RegisterClassW(&WNDCLASSW { lpfnWndProc: Some(panel_proc), hInstance: hi, hCursor: LoadCursorW(null_mut(), IDC_ARROW), lpszClassName: pc.as_ptr(), ..zeroed() });
    PANEL.set(CreateWindowExW(WS_EX_TOOLWINDOW | WS_EX_TOPMOST, pc.as_ptr(), wide("Clipboard").as_ptr(), WS_POPUP, 0, 0, 1, 1, null_mut(), null_mut(), hi, null()));
}

// ---------- the panel ----------

fn panel_w() -> i32 {
    sc(360)
}
fn head_h() -> i32 {
    sc(44)
}
fn pad() -> i32 {
    sc(10)
}
fn item_h(it: &Item) -> i32 {
    match it {
        Item::Text(_) => sc(70),
        Item::Pic(_) => sc(110),
    }
}

/// Item rects in the panel's client area (scrolled by TOP), and the list's full height.
fn layout() -> (Vec<RECT>, i32) {
    let (w, mut y) = (panel_w(), head_h() - TOP.get());
    let mut v = Vec::new();
    HIST.with_borrow(|h| {
        for it in h {
            let ih = item_h(it);
            v.push(RECT { left: pad(), top: y, right: w - pad(), bottom: y + ih });
            y += ih + sc(8);
        }
    });
    let full = y + TOP.get() - head_h() + pad();
    (v, full)
}

fn clear_rect() -> RECT {
    RECT { left: panel_w() - pad() - sc(110), top: sc(8), right: panel_w() - pad(), bottom: head_h() - sc(8) }
}
fn close_x(r: &RECT) -> RECT {
    RECT { left: r.right - sc(30), top: r.top + sc(4), right: r.right - sc(4), bottom: r.top + sc(30) }
}

/// The panel's height: the list (up to a limit), or room for the "nothing here" note.
fn panel_h() -> i32 {
    let (_, full) = layout();
    let list = if HIST.with_borrow(|h| h.is_empty()) { sc(110) } else { full };
    head_h() + list.min(sc(440))
}

/// Win+V: shows the panel next to the text cursor of the window in front (else the pointer), on its monitor.
pub(crate) unsafe fn show() {
    let p = PANEL.get();
    if p.is_null() {
        return;
    }
    if IsWindowVisible(p) != 0 {
        return hide();
    }
    let fg = GetForegroundWindow();
    PREV.set(fg);
    SEL.set(0);
    TOP.set(0);
    HOT.set(-1);
    // the caret of the window in front, like Windows' panel; the pointer when there is none
    let mut at = POINT { x: 0, y: 0 };
    let mut gi: GUITHREADINFO = zeroed();
    gi.cbSize = size_of::<GUITHREADINFO>() as u32;
    let tid = GetWindowThreadProcessId(fg, null_mut());
    let caret = GetGUIThreadInfo(tid, &mut gi) != 0 && !gi.hwndCaret.is_null() && (gi.rcCaret.right > gi.rcCaret.left || gi.rcCaret.bottom > gi.rcCaret.top);
    if caret {
        at = POINT { x: gi.rcCaret.left, y: gi.rcCaret.bottom };
        ClientToScreen(gi.hwndCaret, &mut at);
    } else {
        GetCursorPos(&mut at);
    }
    let mut mi: MONITORINFO = zeroed();
    mi.cbSize = size_of::<MONITORINFO>() as u32;
    GetMonitorInfoW(MonitorFromPoint(at, MONITOR_DEFAULTTONEAREST), &mut mi);
    let wa = mi.rcWork;
    let (w, h) = (panel_w(), panel_h());
    let x = (at.x).clamp(wa.left + sc(8), wa.right - w - sc(8));
    let y = if at.y + sc(6) + h <= wa.bottom - sc(8) { at.y + sc(6) } else { (at.y - h - sc(30)).max(wa.top + sc(8)) };
    let mut cur = POINT { x: 0, y: 0 };
    GetCursorPos(&mut cur);
    OPEN_PT.set(Some((cur.x, cur.y)));
    SetWindowRgn(p, CreateRoundRectRgn(0, 0, w + 1, h + 1, rr(16), rr(16)), 1);
    SetWindowPos(p, HWND_TOPMOST, x, y, w, h, SWP_SHOWWINDOW);
    force_fg(p); // the keyboard goes to the panel: Up / Down / Enter / Delete / Esc
    SetFocus(p);
    InvalidateRect(p, null(), 0);
}

unsafe fn hide() {
    ShowWindow(PANEL.get(), SW_HIDE);
}

/// Re-sizes the panel after the list changed (an item removed, all cleared), keeping its top-left corner.
unsafe fn refit() {
    let p = PANEL.get();
    let mut r: RECT = zeroed();
    GetWindowRect(p, &mut r);
    let (w, h) = (panel_w(), panel_h());
    SetWindowRgn(p, CreateRoundRectRgn(0, 0, w + 1, h + 1, rr(16), rr(16)), 1);
    SetWindowPos(p, HWND_TOPMOST, r.left, r.top, w, h, SWP_NOACTIVATE);
    let (_, full) = layout();
    TOP.set(TOP.get().min((full - (h - head_h())).max(0)));
    InvalidateRect(p, null(), 0);
}

/// Pastes item `i` into the window that was in front: it goes on the clipboard, that window comes back, then Ctrl+V.
unsafe fn paste(i: usize) {
    let Some(it) = HIST.with_borrow(|h| h.get(i).cloned()) else { return };
    hide();
    if !write(LISTENER.get(), &it) {
        return;
    }
    let prev = PREV.get();
    if !prev.is_null() && IsWindow(prev) != 0 {
        force_fg(prev);
    }
    // a moment for the window to take the keyboard back before the keys arrive
    SetTimer(PANEL.get(), PASTE_TIMER, 60, None);
}

unsafe fn remove(i: usize) {
    HIST.with_borrow_mut(|h| {
        if i < h.len() {
            h.remove(i);
        }
    });
    let n = HIST.with_borrow(|h| h.len());
    SEL.set(SEL.get().min(n.saturating_sub(1)));
    refit();
}

/// Keeps the selected item inside the visible part of the list.
unsafe fn scroll_to_sel() {
    let (rects, _) = layout();
    let Some(r) = rects.get(SEL.get()) else { return };
    let mut cr: RECT = zeroed();
    GetClientRect(PANEL.get(), &mut cr);
    if r.top < head_h() {
        TOP.set(TOP.get() - (head_h() - r.top) - sc(4));
    } else if r.bottom > cr.bottom - sc(4) {
        TOP.set(TOP.get() + r.bottom - cr.bottom + pad());
    }
    TOP.set(TOP.get().max(0));
}

unsafe fn hit(x: i32, y: i32) -> i32 {
    let c = clear_rect();
    if x >= c.left && x < c.right && y >= c.top && y < c.bottom {
        return CLEAR;
    }
    if y < head_h() {
        return -1;
    }
    let (rects, _) = layout();
    rects.iter().position(|r| x >= r.left && x < r.right && y >= r.top && y < r.bottom).map_or(-1, |i| i as i32)
}

unsafe extern "system" fn panel_proc(h: HWND, m: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let (x, y) = ((lp & 0xffff) as i16 as i32, (lp >> 16) as i16 as i32);
    match m {
        WM_PAINT => paint(h),
        WM_ERASEBKGND => return 1,
        WM_ACTIVATE if wp & 0xFFFF == 0 => hide(), // a click elsewhere / another window in front: close, like a flyout
        WM_TIMER if wp == PASTE_TIMER => {
            KillTimer(h, PASTE_TIMER);
            keys(&[VK_CONTROL, b'V' as u16]);
        }
        WM_KEYDOWN => {
            let n = HIST.with_borrow(|v| v.len());
            match wp as u16 {
                VK_ESCAPE => hide(),
                VK_DOWN if n > 0 => SEL.set((SEL.get() + 1).min(n - 1)),
                VK_UP => SEL.set(SEL.get().saturating_sub(1)),
                VK_HOME => SEL.set(0),
                VK_END if n > 0 => SEL.set(n - 1),
                VK_RETURN if n > 0 => return { paste(SEL.get()); 0 },
                VK_DELETE if n > 0 => return { remove(SEL.get()); 0 },
                _ => return 0,
            }
            scroll_to_sel();
            InvalidateRect(h, null(), 0);
        }
        WM_MOUSEMOVE => {
            let mut p = POINT { x, y };
            ClientToScreen(h, &mut p);
            if OPEN_PT.get() == Some((p.x, p.y)) {
                return 0; // the pointer has not moved since the panel opened: it does not pick what lies under it
            }
            OPEN_PT.set(None);
            let k = hit(x, y);
            if k >= 0 {
                SEL.set(k as usize);
            }
            if HOT.replace(k) != k || k >= 0 {
                InvalidateRect(h, null(), 0);
            }
            let mut t = TRACKMOUSEEVENT { cbSize: size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: h, dwHoverTime: 0 };
            TrackMouseEvent(&mut t);
        }
        WM_MOUSELEAVE => {
            HOT.set(-1);
            InvalidateRect(h, null(), 0);
        }
        WM_LBUTTONUP => match hit(x, y) {
            CLEAR => {
                HIST.with_borrow_mut(|v| v.clear());
                SEL.set(0);
                TOP.set(0);
                refit();
            }
            k if k >= 0 => {
                let (rects, _) = layout();
                let c = close_x(&rects[k as usize]);
                if x >= c.left && x < c.right && y >= c.top && y < c.bottom {
                    remove(k as usize);
                } else {
                    paste(k as usize);
                }
            }
            _ => {}
        },
        WM_MOUSEWHEEL => {
            let (_, full) = layout();
            let mut cr: RECT = zeroed();
            GetClientRect(h, &mut cr);
            let room = (full - (cr.bottom - head_h())).max(0);
            TOP.set((TOP.get() - (wp >> 16) as i16 as i32 * sc(60) / 120).clamp(0, room));
            InvalidateRect(h, null(), 0);
        }
        _ => return DefWindowProcW(h, m, wp, lp),
    }
    0
}

/// A picture item scaled into `r` (aspect kept, centred).
unsafe fn draw_pic(dc: HDC, dib: &[u8], r: RECT) {
    if dib.len() < size_of::<BITMAPINFOHEADER>() {
        return;
    }
    let bi = &*(dib.as_ptr() as *const BITMAPINFO);
    let hd = &bi.bmiHeader;
    let (bw, bh) = (hd.biWidth, hd.biHeight.abs());
    if bw <= 0 || bh <= 0 {
        return;
    }
    // the pixels follow the header and the colour table (or the three BI_BITFIELDS masks)
    let colors = if hd.biClrUsed > 0 { hd.biClrUsed as usize } else if hd.biBitCount <= 8 { 1usize << hd.biBitCount } else { 0 };
    let masks = if hd.biCompression == 3 && hd.biSize as usize == size_of::<BITMAPINFOHEADER>() { 12 } else { 0 };
    let off = hd.biSize as usize + colors * 4 + masks;
    if off >= dib.len() {
        return;
    }
    let (rw, rh) = (r.right - r.left, r.bottom - r.top);
    let s = (rw as f32 / bw as f32).min(rh as f32 / bh as f32).min(1.0);
    let (dw, dh) = ((bw as f32 * s) as i32, (bh as f32 * s) as i32);
    let (dx, dy) = (r.left + (rw - dw) / 2, r.top + (rh - dh) / 2);
    SetStretchBltMode(dc, HALFTONE);
    StretchDIBits(dc, dx, dy, dw, dh, 0, 0, bw, bh, dib[off..].as_ptr() as _, bi, DIB_RGB_COLORS, SRCCOPY);
}

unsafe fn paint(h: HWND) {
    let mut ps: PAINTSTRUCT = zeroed();
    let wdc = BeginPaint(h, &mut ps);
    let mut cr: RECT = zeroed();
    GetClientRect(h, &mut cr);
    let dc = CreateCompatibleDC(wdc);
    let bmp = CreateCompatibleBitmap(wdc, cr.right, cr.bottom);
    let ob = SelectObject(dc, bmp);
    let p = pal();
    fill(dc, cr, p.bg, 0);
    SetBkMode(dc, TRANSPARENT as i32);
    let font = |px: i32, wt: i32, face: &str| CreateFontW(-sc(px), 0, 0, 0, wt, 0, 0, 0, 1, 0, 0, 5, 0, wide(face).as_ptr());
    let (f_head, f_txt, f_small, f_icon) = (font(14, 600, ui_face()), font(13, 400, ui_face()), font(12, 400, ui_face()), font(11, 400, winrust_common::style::icon_face()));
    let of = SelectObject(dc, f_head);
    text(dc, t!("Буфер обмена", "Clipboard", pt: "Área de transferência"), RECT { left: pad() + sc(4), top: 0, right: cr.right - sc(130), bottom: head_h() }, p.text, DT_LEFT);
    let empty = HIST.with_borrow(|v| v.is_empty());
    if !empty {
        let c = clear_rect();
        if HOT.get() == CLEAR {
            fill(dc, c, p.hover, sc(8));
        }
        SelectObject(dc, f_small);
        text(dc, t!("Очистить всё", "Clear all", pt: "Limpar tudo"), c, p.accent, DT_CENTER);
    }
    // the list, clipped below the header
    let clip = CreateRectRgn(0, head_h(), cr.right, cr.bottom);
    SelectClipRgn(dc, clip);
    if empty {
        SelectObject(dc, f_txt);
        let r = RECT { left: pad(), top: head_h() + sc(10), right: cr.right - pad(), bottom: head_h() + sc(40) };
        text(dc, t!("Здесь пока ничего нет", "Nothing here yet", pt: "Nada aqui ainda"), r, p.text, DT_CENTER);
        SelectObject(dc, f_small);
        let r2 = RECT { top: r.bottom, bottom: r.bottom + sc(40), ..r };
        text(dc, t!("Скопируйте что-нибудь: оно появится здесь", "Copy something and it shows up here", pt: "Copie algo e ele aparece aqui"), r2, p.dim, DT_CENTER);
    }
    let (rects, _) = layout();
    HIST.with_borrow(|v| {
        for (i, (it, r)) in v.iter().zip(&rects).enumerate() {
            if r.bottom < head_h() || r.top > cr.bottom {
                continue;
            }
            let sel = i == SEL.get();
            fill(dc, *r, if sel { p.active } else { card() }, sc(10));
            if sel {
                fill(dc, RECT { left: r.left, top: r.top + sc(12), right: r.left + sc(3), bottom: r.bottom - sc(12) }, p.accent, sc(3));
            }
            let inner = RECT { left: r.left + sc(14), top: r.top + sc(8), right: r.right - sc(34), bottom: r.bottom - sc(8) };
            match it {
                Item::Text(s) => {
                    SelectObject(dc, f_txt);
                    SetTextColor(dc, p.text);
                    // up to three lines of the text, tabs and line breaks kept, the rest cut with "…"
                    let shown: String = s.chars().take(400).collect::<String>().replace("\r\n", "\n").replace('\t', "    ");
                    let w = wide(shown.trim());
                    let mut tr = inner;
                    DrawTextW(dc, w.as_ptr(), -1, &mut tr, DT_LEFT | DT_TOP | DT_WORDBREAK | DT_NOPREFIX | DT_END_ELLIPSIS | DT_EDITCONTROL);
                }
                Item::Pic(d) => draw_pic(dc, d, inner),
            }
            if sel {
                let c = close_x(r);
                SelectObject(dc, f_icon);
                text(dc, "\u{E711}", c, p.dim, DT_CENTER); // ×: remove this item
            }
        }
    });
    SelectClipRgn(dc, null_mut());
    DeleteObject(clip);
    SelectObject(dc, of);
    for f in [f_head, f_txt, f_small, f_icon] {
        DeleteObject(f);
    }
    BitBlt(wdc, 0, 0, cr.right, cr.bottom, dc, 0, 0, SRCCOPY);
    SelectObject(dc, ob);
    DeleteObject(bmp);
    DeleteDC(dc);
    EndPaint(h, &ps);
}

#[cfg(test)]
mod tests {
    use super::{HIST, Item, MAX, remember};

    #[test]
    fn a_copy_again_moves_up_and_the_list_stays_short() {
        HIST.with_borrow_mut(|h| h.clear());
        remember(Item::Text("a".into()));
        remember(Item::Text("b".into()));
        remember(Item::Text("a".into()));
        HIST.with_borrow(|h| assert!(*h == vec![Item::Text("a".into()), Item::Text("b".into())]));
        for i in 0..MAX + 5 {
            remember(Item::Text(i.to_string()));
        }
        HIST.with_borrow(|h| assert_eq!(h.len(), MAX));
    }
}
