//! topbar: window cards with live DWM previews: the Alt+Tab grid and the previews above an app button (#92).
//! (split out of main.rs; shares its items through `super::*`)
use super::*;
use windows_sys::Win32::{
    Foundation::SIZE,
    Graphics::Dwm::{DWM_THUMBNAIL_PROPERTIES, DWM_TNP_RECTDESTINATION, DWM_TNP_SOURCECLIENTAREAONLY, DWM_TNP_VISIBLE, DwmQueryThumbnailSourceSize, DwmRegisterThumbnail, DwmUnregisterThumbnail, DwmUpdateThumbnailProperties},
};

/// Card size and spacing before scaling (sc()).
pub(crate) const CARD_W: i32 = 220;
pub(crate) const CARD_H: i32 = 160;
pub(crate) const GAP: i32 = 10;
const TITLE: i32 = 28;

/// `n` cards of `cw` x `ch` in rows that fit `aw` x `ah`: the rows are balanced, a short last row is centered, and the
/// cards shrink (down to a third) when there are too many. Returns the card rects from (0,0) and the total width / height.
pub(crate) fn grid(n: usize, aw: i32, ah: i32, cw: i32, ch: i32, gap: i32) -> (Vec<RECT>, i32, i32) {
    if n == 0 {
        return (vec![], 0, 0);
    }
    let mut s = 1.0f32;
    let (w, h, rows, cols) = loop {
        let (w, h) = ((cw as f32 * s) as i32, (ch as f32 * s) as i32);
        let fit = ((aw + gap) / (w + gap)).max(1) as usize;
        let rows = n.div_ceil(fit);
        if rows as i32 * (h + gap) - gap <= ah || s < 0.35 {
            break (w, h, rows, n.div_ceil(rows));
        }
        s *= 0.9;
    };
    let (tw, th) = (cols as i32 * (w + gap) - gap, rows as i32 * (h + gap) - gap);
    let mut out = Vec::with_capacity(n);
    for r in 0..rows {
        let count = cols.min(n - r * cols);
        let x0 = (tw - (count as i32 * (w + gap) - gap)) / 2;
        for c in 0..count {
            let (x, y) = (x0 + c as i32 * (w + gap), r as i32 * (h + gap));
            out.push(RECT { left: x, top: y, right: x + w, bottom: y + h });
        }
    }
    (out, tw, th)
}

/// The little × in a card's top right corner.
pub(crate) fn close_rect(r: &RECT) -> RECT {
    let s = sc(TITLE) - sc(8);
    RECT { left: r.right - s - sc(4), top: r.top + sc(4), right: r.right - sc(4), bottom: r.top + sc(4) + s }
}

/// The ← and → left of the ×: they move a window one place in its app's previews.
pub(crate) fn move_rects(r: &RECT) -> (RECT, RECT) {
    let c = close_rect(r);
    let s = c.right - c.left;
    let right = RECT { left: c.left - s - sc(2), right: c.left - sc(2), ..c };
    (RECT { left: right.left - s - sc(2), right: right.left - sc(2), ..c }, right)
}

fn hit(r: &RECT, x: i32, y: i32) -> bool {
    x >= r.left && x < r.right && y >= r.top && y < r.bottom
}

/// Where the preview goes inside a card: under the title row.
fn preview_area(r: &RECT) -> RECT {
    RECT { left: r.left + sc(8), top: r.top + sc(TITLE), right: r.right - sc(8), bottom: r.bottom - sc(8) }
}

pub(crate) fn card_at(rects: &[RECT], x: i32, y: i32) -> Option<usize> {
    rects.iter().position(|r| x >= r.left && x < r.right && y >= r.top && y < r.bottom)
}

/// Live previews of `wins` drawn by DWM into `host` over the cards. 0 where there is none (minimized windows show
/// nothing in DWM: their card keeps the big icon painted underneath).
pub(crate) unsafe fn register(host: HWND, wins: &[HWND], rects: &[RECT]) -> Vec<isize> {
    wins.iter().zip(rects).map(|(&w, r)| {
        let mut id = 0isize;
        if IsIconic(w) != 0 || DwmRegisterThumbnail(host, w, &mut id) != 0 {
            return 0;
        }
        let mut src = SIZE { cx: 0, cy: 0 };
        DwmQueryThumbnailSourceSize(id, &mut src);
        let a = preview_area(r);
        let (aw, ah) = ((a.right - a.left) as f32, (a.bottom - a.top) as f32);
        let k = if src.cx > 0 && src.cy > 0 { (aw / src.cx as f32).min(ah / src.cy as f32).min(1.0) } else { 1.0 };
        let (dw, dh) = ((src.cx as f32 * k) as i32, (src.cy as f32 * k) as i32);
        let (dx, dy) = (a.left + (aw as i32 - dw) / 2, a.top + (ah as i32 - dh) / 2);
        let p = DWM_THUMBNAIL_PROPERTIES {
            dwFlags: DWM_TNP_RECTDESTINATION | DWM_TNP_VISIBLE | DWM_TNP_SOURCECLIENTAREAONLY,
            rcDestination: RECT { left: dx, top: dy, right: dx + dw, bottom: dy + dh },
            rcSource: zeroed(),
            opacity: 255,
            fVisible: 1,
            fSourceClientAreaOnly: 0,
        };
        DwmUpdateThumbnailProperties(id, &p);
        id
    }).collect()
}

pub(crate) unsafe fn unregister(ids: &mut Vec<isize>) {
    ids.drain(..).filter(|&id| id != 0).for_each(|id| {
        DwmUnregisterThumbnail(id);
    });
}

/// Paints the cards (background, title row with icon and close box, big icon where DWM shows no preview) of `host`.
/// `sel` gets the accent ring (and, with `arrows`, the ← → that move it).
pub(crate) unsafe fn paint(host: HWND, wins: &[HWND], rects: &[RECT], ids: &[isize], sel: Option<usize>, arrows: bool) {
    let mut ps: PAINTSTRUCT = zeroed();
    let dc = BeginPaint(host, &mut ps);
    let mut r: RECT = zeroed();
    GetClientRect(host, &mut r);
    let (mem, bmp) = (CreateCompatibleDC(dc), CreateCompatibleBitmap(dc, r.right, r.bottom));
    let ob = SelectObject(mem, bmp);
    fill(mem, r, pal().line, sc(18));
    fill(mem, RECT { left: 1, top: 1, right: r.right - 1, bottom: r.bottom - 1 }, pal().bg, sc(18));
    let f = CreateFontW(-sc(12), 0, 0, 0, 400, 0, 0, 0, 1, 0, 0, 5, 0, wide(ui_face()).as_ptr());
    let of = SelectObject(mem, f);
    SetBkMode(mem, TRANSPARENT as i32);
    for (k, (&w, c)) in wins.iter().zip(rects).enumerate() {
        let on = sel == Some(k);
        if on {
            fill(mem, *c, pal().accent, sc(12));
        }
        fill(mem, RECT { left: c.left + sc(2), top: c.top + sc(2), right: c.right - sc(2), bottom: c.bottom - sc(2) }, if on { pal().active } else { pal().hover }, sc(10));
        let ic = icon_of(w);
        let ic = if ic.is_null() { LoadIconW(null_mut(), IDI_APPLICATION) } else { ic };
        let t = sc(TITLE);
        DrawIconEx(mem, c.left + sc(8), c.top + (t - sc(16)) / 2, ic, sc(16), sc(16), 0, null_mut(), DI_NORMAL);
        let mut b = [0u16; 256];
        let n = GetWindowTextW(w, b.as_mut_ptr(), 256).max(0) as usize;
        let x = close_rect(c);
        let (ml, mr) = move_rects(c);
        let arr = on && arrows && wins.len() > 1;
        let tr = if arr { ml.left } else { x.left } - sc(4);
        text(mem, &String::from_utf16_lossy(&b[..n]), RECT { left: c.left + sc(30), top: c.top, right: tr, bottom: c.top + t }, pal().text, 0);
        if on {
            text(mem, "\u{2715}", x, pal().text, DT_CENTER);
        }
        if arr && k > 0 {
            text(mem, "\u{2190}", ml, pal().text, DT_CENTER);
        }
        if arr && k + 1 < wins.len() {
            text(mem, "\u{2192}", mr, pal().text, DT_CENTER);
        }
        if ids.get(k).is_none_or(|&id| id == 0) {
            let a = preview_area(c);
            let s = sc(48).min(a.right - a.left).min(a.bottom - a.top).max(sc(16));
            DrawIconEx(mem, (a.left + a.right - s) / 2, (a.top + a.bottom - s) / 2, ic, s, s, 0, null_mut(), DI_NORMAL);
        }
    }
    SelectObject(mem, of);
    DeleteObject(f);
    BitBlt(dc, 0, 0, r.right, r.bottom, mem, 0, 0, SRCCOPY);
    SelectObject(mem, ob);
    DeleteObject(bmp);
    DeleteDC(mem);
    EndPaint(host, &ps);
}

// ---------- previews above an app button ----------

thread_local! {
    pub(crate) static PVWND: Cell<HWND> = const { Cell::new(null_mut()) };
    static PV: RefCell<(Vec<HWND>, Vec<RECT>, Vec<isize>, Option<usize>)> = const { RefCell::new((Vec::new(), Vec::new(), Vec::new(), None)) };
    static PV_BTN: Cell<i32> = const { Cell::new(NONE) };
    static PV_AWAY: Cell<u64> = const { Cell::new(0) };
}

/// Shows the windows of app button `btn` as live previews above it. False when it has no windows (the caller shows the tip).
pub(crate) unsafe fn show_preview(btn: i32, cx: i32) -> bool {
    let wins: Vec<HWND> = BTNS.with_borrow(|b| b.get(btn as usize).map(|b| b.wins.clone()).unwrap_or_default()).into_iter().filter(|&w| IsWindow(w) != 0).collect();
    let pv = PVWND.get();
    if wins.is_empty() || pv.is_null() || btn <= 0 {
        return false;
    }
    PV_BTN.set(btn);
    PV_AWAY.set(0);
    layout(pv, wins, cx);
    SetTimer(pv, 1, 150, None); // hides it once the pointer has left both the button and the previews
    true
}

unsafe fn layout(pv: HWND, wins: Vec<HWND>, cx: i32) {
    let pad = sc(GAP);
    let (scw, sch) = (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN));
    let (mut rects, tw, th) = grid(wins.len(), scw - sc(16) - 2 * pad, sch / 2, sc(CARD_W), sc(CARD_H), sc(GAP));
    rects.iter_mut().for_each(|r| *r = RECT { left: r.left + pad, top: r.top + pad, right: r.right + pad, bottom: r.bottom + pad });
    let (w, h) = (tw + 2 * pad, th + 2 * pad);
    let old = PV.take();
    let mut ids = old.2;
    unregister(&mut ids);
    SetWindowRgn(pv, CreateRoundRectRgn(0, 0, w + 1, h + 1, rr(18), rr(18)), 1);
    if vert() {
        let y = (cx - h / 2).clamp(sc(4), sch - h - sc(4));
        SetWindowPos(pv, HWND_TOPMOST, side_x(w), y, w, h, SWP_NOACTIVATE | SWP_SHOWWINDOW);
    } else {
        let x = (cx - w / 2).clamp(sc(4), scw - w - sc(4));
        SetWindowPos(pv, HWND_TOPMOST, x, if at_top() { bar_h() - bar_off() + sc(6) } else { bar_top() + bar_off() - h - sc(6) }, w, h, SWP_NOACTIVATE | SWP_SHOWWINDOW);
    }
    let ids = register(pv, &wins, &rects);
    PV.set((wins, rects, ids, None));
    InvalidateRect(pv, null(), 0);
}

/// Lays the previews out again for `wins`, centered where the window is now.
unsafe fn relayout(pv: HWND, wins: Vec<HWND>) {
    let mut wr: RECT = zeroed();
    GetWindowRect(pv, &mut wr);
    layout(pv, wins, if vert() { (wr.top + wr.bottom) / 2 } else { (wr.left + wr.right) / 2 });
}

pub(crate) unsafe fn hide_preview() {
    let pv = PVWND.get();
    if pv.is_null() || IsWindowVisible(pv) == 0 {
        return;
    }
    KillTimer(pv, 1);
    ShowWindow(pv, SW_HIDE);
    let (_, _, mut ids, _) = PV.take();
    unregister(&mut ids);
    PV_BTN.set(NONE);
}

pub(crate) unsafe extern "system" fn pv_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let (x, y) = ((lp & 0xffff) as i16 as i32, (lp >> 16) as i16 as i32);
    match msg {
        WM_PAINT => PV.with_borrow(|(w, r, ids, sel)| paint(hwnd, w, r, ids, *sel, true)),
        WM_MOUSEACTIVATE => return MA_NOACTIVATE as isize,
        WM_MOUSEMOVE => {
            let k = PV.with_borrow(|p| card_at(&p.1, x, y));
            if PV.with_borrow_mut(|p| std::mem::replace(&mut p.3, k)) != k {
                InvalidateRect(hwnd, null(), 0);
            }
        }
        WM_LBUTTONUP | WM_MBUTTONUP => {
            let Some((k, w, r)) = PV.with_borrow(|p| card_at(&p.1, x, y).map(|k| (k, p.0[k], p.1[k]))) else { return 0 };
            let c = close_rect(&r);
            let (ml, mr) = move_rects(&r);
            let mut v = PV.with_borrow(|p| p.0.clone());
            let to = if msg != WM_LBUTTONUP || v.len() < 2 {
                None
            } else if k > 0 && hit(&ml, x, y) {
                Some(k - 1)
            } else if k + 1 < v.len() && hit(&mr, x, y) {
                Some(k + 1)
            } else {
                None
            };
            if let Some(j) = to {
                // the one way to change the order: swap the two in the remembered order, so it sticks
                ORDER.with_borrow_mut(|o| {
                    if let (Some(a), Some(b)) = (o.iter().position(|&h| h == v[k]), o.iter().position(|&h| h == v[j])) {
                        o.swap(a, b);
                    }
                });
                refresh_buttons();
                v.swap(k, j);
                relayout(hwnd, v);
                PV.with_borrow_mut(|p| p.3 = Some(j)); // the arrows stay on the moved card
            } else if msg == WM_MBUTTONUP || hit(&c, x, y) {
                PostMessageW(w, WM_CLOSE, 0, 0);
                v.remove(k);
                if v.is_empty() {
                    hide_preview();
                } else {
                    relayout(hwnd, v);
                }
            } else {
                hide_preview();
                if IsIconic(w) != 0 {
                    ShowWindow(w, SW_RESTORE);
                }
                force_fg(w);
            }
        }
        WM_TIMER if wp == 1 => {
            let mut pt = POINT { x: 0, y: 0 };
            GetCursorPos(&mut pt);
            let mut wr: RECT = zeroed();
            GetWindowRect(hwnd, &mut wr);
            let inside = pt.x >= wr.left && pt.x < wr.right && pt.y >= wr.top && pt.y < wr.bottom;
            if inside || HOT.get() == PV_BTN.get() {
                PV_AWAY.set(0);
            } else if PV_AWAY.get() == 0 {
                PV_AWAY.set(now_ms());
            } else if now_ms().saturating_sub(PV_AWAY.get()) > 300 {
                hide_preview(); // a short grace period: the pointer crosses the gap between the button and the previews
            }
        }
        _ => return DefWindowProcW(hwnd, msg, wp, lp),
    }
    0
}

#[cfg(test)]
mod tests {
    use super::{close_rect, grid, move_rects};
    use windows_sys::Win32::Foundation::RECT;

    #[test]
    fn move_arrows_sit_left_of_the_close_box_without_overlap() {
        let card = RECT { left: 0, top: 0, right: 220, bottom: 160 };
        let (c, (l, r)) = (close_rect(&card), move_rects(&card));
        assert!(l.right <= r.left && r.right <= c.left, "{} {} {} {}", l.right, r.left, r.right, c.left);
        assert!(l.left > 30, "the title keeps room after the icon");
        assert_eq!((l.top, r.bottom), (c.top, c.bottom));
    }

    #[test]
    fn grid_balances_rows_and_centers_the_last_one() {
        // 5 cards of 100 in 350 px: at most 3 fit per row -> 2 rows of 3 + 2, the 2 centered under the 3
        let (r, tw, th) = grid(5, 350, 1000, 100, 50, 10);
        assert_eq!((tw, th), (320, 110));
        assert_eq!(r[0].left, 0);
        assert_eq!(r[3].left, 55);
        assert_eq!(r[4].top, 60);
    }

    #[test]
    fn grid_shrinks_cards_when_they_do_not_fit() {
        let (r, _, th) = grid(40, 500, 200, 100, 50, 10);
        assert_eq!(r.len(), 40);
        assert!(th <= 200, "{th}");
        assert!(r[0].right < 100);
    }

    #[test]
    fn grid_of_nothing_is_empty() {
        assert_eq!(grid(0, 100, 100, 10, 10, 1).0.len(), 0);
    }
}
