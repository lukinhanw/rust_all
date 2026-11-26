//! The window: drawing (GDI+ shapes, GDI text), layout, dialogs and the message loop handler.
use crate::*;

// ---------- drawing ----------
//
// One back buffer per paint: GDI+ for the anti-aliased shapes, then GDI (ClearType) for text on top.

pub(crate) fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain([0]).collect()
}

/// COLORREF (0x00BBGGRR) + alpha -> GDI+ ARGB.
pub(crate) const fn argb(c: u32, a: u8) -> u32 {
    (a as u32) << 24 | (c & 255) << 16 | (c >> 8 & 255) << 8 | (c >> 16 & 255)
}

pub(crate) fn lerp(a: u32, b: u32, t: f32) -> u32 {
    let m = |sh: u32| (((a >> sh & 255) as f32) * (1.0 - t) + ((b >> sh & 255) as f32) * t) as u32;
    m(0) | m(8) << 8 | m(16) << 16
}

pub(crate) type Box4 = [f32; 4]; // left, top, right, bottom in logical px

pub(crate) struct Ui {
    dc: HDC,
    g: *mut GpGraphics,
    s: f32,
}
impl Ui {
    fn px(&self, v: f32) -> i32 {
        (v * self.s).round() as i32
    }
    unsafe fn shape(&self, b: Box4, rad: f32, color: u32) {
        let (s, mut path) = (self.s, null_mut::<GpPath>());
        let (x, y, w, h) = (b[0] * s, b[1] * s, (b[2] - b[0]) * s, (b[3] - b[1]) * s);
        let d = (rad * 2.0 * s).min(h).min(w).max(0.01);
        GdipCreatePath(FillModeAlternate, &mut path);
        GdipAddPathArc(path, x, y, d, d, 180.0, 90.0);
        GdipAddPathArc(path, x + w - d, y, d, d, 270.0, 90.0);
        GdipAddPathArc(path, x + w - d, y + h - d, d, d, 0.0, 90.0);
        GdipAddPathArc(path, x, y + h - d, d, d, 90.0, 90.0);
        GdipClosePathFigure(path);
        let mut br = null_mut::<GpSolidFill>();
        GdipCreateSolidFill(color, &mut br);
        GdipFillPath(self.g, br as *mut GpBrush, path);
        GdipDeleteBrush(br as *mut GpBrush);
        GdipDeletePath(path);
    }
    /// Rounded box with a 1 px outline (`line` == `fill` for none).
    unsafe fn round(&self, b: Box4, rad: f32, fill: u32, line: u32) {
        if line != fill {
            self.shape(b, rad, argb(line, 255));
            let i = 1.0 / self.s;
            self.shape([b[0] + i, b[1] + i, b[2] - i, b[3] - i], rad - i, argb(fill, 255));
        } else {
            self.shape(b, rad, argb(fill, 255));
        }
    }
    unsafe fn font(&self, size: f32, weight: i32, face: &str) -> HFONT {
        CreateFontW(-self.px(size), 0, 0, 0, weight, 0, 0, 0, 1, 0, 0, 5, 0, wide(face).as_ptr())
    }
    unsafe fn text_in(&self, s: &str, b: Box4, size: f32, weight: i32, color: u32, flags: u32, face: &str) {
        let f = self.font(size, weight, face);
        let of = SelectObject(self.dc, f);
        SetTextColor(self.dc, color);
        SetBkMode(self.dc, TRANSPARENT as i32);
        let mut r = RECT { left: self.px(b[0]), top: self.px(b[1]), right: self.px(b[2]), bottom: self.px(b[3]) };
        DrawTextW(self.dc, wide(s).as_ptr(), -1, &mut r, flags | DT_NOPREFIX);
        SelectObject(self.dc, of);
        DeleteObject(f);
    }
    unsafe fn text(&self, s: &str, b: Box4, size: f32, weight: i32, color: u32, flags: u32) {
        self.text_in(s, b, size, weight, color, flags, winrust_common::style::sans_face());
    }
    /// Width of a single line of text, logical px.
    unsafe fn width(&self, s: &str, size: f32, weight: i32) -> f32 {
        let f = self.font(size, weight, winrust_common::style::sans_face());
        let of = SelectObject(self.dc, f);
        let mut r: RECT = zeroed();
        DrawTextW(self.dc, wide(s).as_ptr(), -1, &mut r, DT_CALCRECT | DT_SINGLELINE | DT_NOPREFIX);
        SelectObject(self.dc, of);
        DeleteObject(f);
        r.right as f32 / self.s
    }
}

// layout, logical px: a setup wizard. A dark header with the accent line under it, the steps on the left, one page at a
// time on the right, the buttons at the bottom right.
pub(crate) const WIN_W: f32 = 800.0;
pub(crate) const WIN_H: f32 = 650.0;
const HEAD_H: f32 = 64.0;
const SIDE_W: f32 = 210.0;
const PAD: f32 = 32.0;
const X0: f32 = SIDE_W + PAD; // left edge of the page
const X1: f32 = WIN_W - PAD; // right edge of the page
const FOOT_Y: f32 = WIN_H - 68.0; // the footer's hairline
pub(crate) const BTN_H: f32 = 34.0;
const BTN_Y: f32 = WIN_H - 50.0;
pub(crate) const BTN_W: f32 = 130.0;
pub(crate) const N_READY: usize = {
    let (mut n, mut i) = (0, 0);
    while i < PLUGINS.len() {
        if PLUGINS[i].actions.is_some() && PLUGINS[i].bulk {
            n += 1;
        }
        i += 1;
    }
    n
};
/// Tools that change system behaviour (debloat) or remove apps for good (purge): their own group, never ticked by default.
pub(crate) const N_EXTRA: usize = {
    let (mut n, mut i) = (0, 0);
    while i < PLUGINS.len() {
        if PLUGINS[i].actions.is_some() && !PLUGINS[i].bulk {
            n += 1;
        }
        i += 1;
    }
    n
};
pub(crate) const ROW_H: f32 = 34.0;
const LIST_TOP: f32 = 180.0; // the custom page's first row
const EXTRA_SEC_Y: f32 = LIST_TOP + N_READY as f32 * ROW_H + 12.0;
const EXTRA_TOP: f32 = EXTRA_SEC_Y + 26.0;
const NOTE_Y: f32 = FOOT_Y - 50.0; // status / the hovered component's description
const _: () = assert!(EXTRA_TOP + N_EXTRA as f32 * ROW_H <= NOTE_Y, "the custom list runs into the note: make WIN_H taller");

// clickable things
pub(crate) const ID_MAIN: usize = 0; // the primary button of the page
pub(crate) const ID_ROLLBACK: usize = 1;
pub(crate) const ID_OK: usize = 2;
pub(crate) const ID_CANCEL: usize = 3;
pub(crate) const ID_SWITCH: usize = 4;
pub(crate) const ID_GEAR: usize = 5;
pub(crate) const ID_LANG: usize = 6;
pub(crate) const ID_SHELL: usize = 7; // settings panel: replace Explorer with rust-explorer
pub(crate) const ID_ALL: usize = 8; // "select all / none" on the custom page
pub(crate) const ID_UPDATE: usize = 9; // "update available" link at the bottom of the step list
const ID_BACK: usize = 10;
const ID_EXPRESS: usize = 11; // the two choice cards of the first page
const ID_CUSTOM: usize = 12;
pub(crate) const ID_CHECK: usize = 100; // + plugin index: the whole row toggles it
pub(crate) const STARTING: usize = usize::MAX - 1; // JOB before apply() picks its first step

pub(crate) const HEAD: u32 = rgb(16, 16, 16); // header and title bar
const SIDE: u32 = rgb(24, 24, 24); // the step list
const ON_ACCENT: u32 = rgb(12, 12, 12); // text and ticks on the accent colour

thread_local! {
    static PAGE: Cell<u8> = const { Cell::new(0) }; // 0 options, 1 components, 2 working, 3 done
    static CUSTOM: Cell<bool> = const { Cell::new(false) }; // the first page's choice
    static REMOVING_ALL: Cell<bool> = const { Cell::new(false) }; // the job is "remove WinRust"
}
/// What the last job did, (plugin, install?, error) in order: the done page lists it.
pub(crate) static RESULTS: Mutex<Vec<(usize, bool, Option<String>)>> = Mutex::new(Vec::new());

/// Installable components, in install order (topbar comes before rust-explorer, which needs it).
pub(crate) fn ready() -> impl Iterator<Item = usize> {
    (0..PLUGINS.len()).filter(|&i| PLUGINS[i].actions.is_some() && PLUGINS[i].bulk)
}

/// The second group: on request only (command line `install debloat` or its box here).
pub(crate) fn extras() -> impl Iterator<Item = usize> {
    (0..PLUGINS.len()).filter(|&i| PLUGINS[i].actions.is_some() && !PLUGINS[i].bulk)
}

/// Every row of the custom page: the components, then the extras.
pub(crate) fn rows() -> impl Iterator<Item = usize> {
    ready().chain(extras())
}

/// The rows that are ticked, in install order (extras last).
pub(crate) fn chosen() -> impl Iterator<Item = usize> {
    rows().filter(|&i| SELECTED[i].load(Relaxed))
}

/// What the button would do now: (to remove, to install).
pub(crate) fn diff() -> (Vec<usize>, Vec<usize>) {
    plan(&rows().collect::<Vec<_>>(), |i| INSTALLED[i].load(Relaxed), |i| SELECTED[i].load(Relaxed))
}

/// The ticks as the custom page starts: what is installed, plus the recommended components on a fresh machine.
pub(crate) fn reset_selection() {
    for i in ready() {
        SELECTED[i].store(selected_by_default(PLUGINS[i].name, INSTALLED[i].load(Relaxed)), Relaxed);
    }
    for i in extras() {
        SELECTED[i].store(INSTALLED[i].load(Relaxed), Relaxed); // an applied extra shows as on; a new one is never pre-ticked
    }
}

/// What "Express" installs (the components not there yet), in install order.
fn express_adds() -> Vec<usize> {
    rows().filter(|&i| !INSTALLED[i].load(Relaxed) && express_pick(PLUGINS[i].name, PLUGINS[i].bulk, false)).collect()
}

/// The running job's steps, (plugin, install?) in order: the progress page reads it.
pub(crate) static STEPS: Mutex<Vec<(usize, bool)>> = Mutex::new(Vec::new());

fn primary_box() -> Box4 {
    [X1 - BTN_W, BTN_Y, X1, BTN_Y + BTN_H]
}

fn back_box() -> Box4 {
    let p = primary_box();
    [p[0] - 10.0 - BTN_W, p[1], p[0] - 10.0, p[3]]
}

/// "Remove WinRust": plain red text at the bottom left of the first page.
pub(crate) fn rollback_box(_w: f32) -> Box4 {
    [X0 - 4.0, BTN_Y, X0 + 170.0, BTN_Y + BTN_H]
}

/// The choice cards of the first page: 0 express, 1 custom.
fn card_box(k: usize) -> Box4 {
    let t = 206.0 + k as f32 * 104.0;
    [X0, t, X1, t + 90.0]
}

/// Row `k` of rows() on the custom page: the first N_READY in the components group, the rest in the extras group.
pub(crate) fn row_box(k: usize, _w: f32) -> Box4 {
    let t = if k < N_READY { LIST_TOP + k as f32 * ROW_H } else { EXTRA_TOP + (k - N_READY) as f32 * ROW_H };
    [X0, t, X1, t + ROW_H]
}

const NAME_X: f32 = X0 + 54.0; // the name column of a row
const DESC_X: f32 = X0 + 232.0; // the short description column

/// Settings gear after the name of the plugin that has settings (topbar): (plugin index, box).
pub(crate) fn gear_box(w: f32) -> Option<(usize, Box4)> {
    let (k, i) = ready().enumerate().find(|&(_, i)| PLUGINS[i].settings)?;
    let r = row_box(k, w);
    Some((i, [DESC_X - 30.0, r[1] + 5.0, DESC_X - 6.0, r[3] - 5.0]))
}

/// "Select all / none" at the right end of the components caption.
pub(crate) fn all_box(_w: f32) -> Box4 {
    [X1 - 160.0, LIST_TOP - 28.0, X1, LIST_TOP - 6.0]
}

/// On/off switch inside the settings panel's first row.
pub(crate) fn switch_box(w: f32) -> Box4 {
    let (d, _, _) = dialog_boxes(w);
    let mid = d[1] + 72.0 + 32.0;
    [d[2] - 24.0 - 16.0 - 40.0, mid - 12.0, d[2] - 24.0 - 16.0, mid + 12.0]
}

/// Second switch row of the settings panel: which shell to install.
pub(crate) fn shell_switch_box(w: f32) -> Box4 {
    let b = switch_box(w);
    [b[0], b[1] + 64.0, b[2], b[3] + 64.0]
}

/// Index of rust-explorer in PLUGINS.
fn explorer_idx() -> Option<usize> {
    PLUGINS.iter().position(|p| p.name == crate::selection::RUST_EXPLORER)
}

/// "Update available: v0.6.0" at the bottom of the step list.
pub(crate) fn update_box(_w: f32) -> Box4 {
    [20.0, WIN_H - 78.0, SIDE_W - 12.0, WIN_H - 56.0]
}

/// The language drop-down at the right end of the header, and one item of its list.
pub(crate) fn lang_box(_w: f32) -> Box4 {
    [X1 - 132.0, 19.0, X1, 45.0]
}
const LANG_ITEM_H: f32 = 26.0; // one item of the open list
const LANG_LIST: &str = "WinRustLangList";

/// Dialog panel and its (cancel, ok) buttons.
pub(crate) fn dialog_boxes(w: f32) -> (Box4, Box4, Box4) {
    let (dw, dh) = (520.0, 440.0);
    let (l, t) = ((w - dw) / 2.0, (WIN_H - dh) / 2.0);
    let (bt, r) = (t + dh - 20.0 - BTN_H, l + dw - 20.0);
    ([l, t, l + dw, t + dh], [r - BTN_W - 8.0 - BTN_W, bt, r - BTN_W - 8.0, bt + BTN_H], [r - BTN_W, bt, r, bt + BTN_H])
}

/// Pixels per logical px: the screen's DPI, shrunk so the whole window fits the work area (small screens, big DPI).
pub(crate) fn scale() -> f32 {
    unsafe {
        let dc = GetDC(null_mut());
        let mut s = GetDeviceCaps(dc, LOGPIXELSX as i32) as f32 / 96.0;
        ReleaseDC(null_mut(), dc);
        let mut wa: RECT = zeroed();
        if SystemParametersInfoW(SPI_GETWORKAREA, 0, &mut wa as *mut _ as *mut _, 0) != 0 {
            let frame = GetSystemMetrics(SM_CYCAPTION) + 2 * GetSystemMetrics(SM_CYFIXEDFRAME) + 8;
            s = s.min(((wa.bottom - wa.top - frame) as f32 / WIN_H).max(0.5));
        }
        s
    }
}

/// A button: `kind` 0 = primary (accent), 1 = secondary (outlined), 2 = danger (red), 3 = disabled.
pub(crate) unsafe fn button(ui: &Ui, b: Box4, label: &str, kind: u8, hot: bool) {
    let (fill, line, fg, wt) = match kind {
        0 => (ACCENT, ACCENT, ON_ACCENT, 700),
        1 => (BG, rgb(120, 120, 120), TEXT, 600),
        2 => (BAD, BAD, rgb(255, 255, 255), 700),
        _ => (rgb(52, 52, 52), rgb(52, 52, 52), lerp(DIM, BG, 0.3), 600),
    };
    let (fill, line) = if hot && kind != 3 { (lerp(fill, rgb(255, 255, 255), 0.12), lerp(line, rgb(255, 255, 255), 0.2)) } else { (fill, line) };
    ui.round(b, 3.0, fill, line);
    ui.text(&label.to_uppercase(), b, 12.5, wt, fg, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
}

/// The on/off switch of the settings panel.
unsafe fn switch(ui: &Ui, b: Box4, on: bool, dim: bool) {
    let track = if on { ACCENT } else { rgb(72, 72, 74) };
    let track = if dim { lerp(track, PANEL, 0.5) } else { track };
    let h = b[3] - b[1];
    ui.round(b, h / 2.0, track, track);
    let k = h - 4.0;
    let x = if on { b[2] - 2.0 - k } else { b[0] + 2.0 };
    ui.round([x, b[1] + 2.0, x + k, b[3] - 2.0], k / 2.0, rgb(255, 255, 255), rgb(255, 255, 255));
}

/// A square check box: accent with a tick when on.
unsafe fn check(ui: &Ui, b: Box4, on: bool, dim: bool) {
    let c = if dim { lerp(ACCENT, BG, 0.5) } else { ACCENT };
    if on {
        ui.round(b, 2.0, c, c);
        ui.text_in("\u{E73E}", b, 11.0, 700, ON_ACCENT, DT_CENTER | DT_VCENTER | DT_SINGLELINE, winrust_common::style::icon_face());
    } else {
        ui.round(b, 2.0, BG, if dim { BORDER } else { rgb(120, 120, 120) });
    }
}

/// A round option button centred on (x, y).
unsafe fn radio(ui: &Ui, x: f32, y: f32, on: bool) {
    let r = 9.0;
    ui.round([x - r, y - r, x + r, y + r], r, BG, if on { ACCENT } else { rgb(120, 120, 120) });
    if on {
        ui.round([x - 5.0, y - 5.0, x + 5.0, y + 5.0], 5.0, ACCENT, ACCENT);
    }
}

/// The coloured glyph of a component.
fn tile_color(name: &str) -> u32 {
    match name {
        "rust-search" => rgb(10, 132, 255),
        "rtaskmgr-win32" => rgb(48, 209, 88),
        "topbar" => rgb(94, 92, 230),
        "rust-explorer" => rgb(255, 159, 10),
        "wiztree" => rgb(191, 90, 242),
        "photoview" => rgb(255, 55, 95),
        "debloat" => rgb(100, 210, 255),
        "purge" => rgb(255, 69, 58),
        _ => rgb(142, 142, 147),
    }
}

/// The page's title and the line under it.
unsafe fn title(ui: &Ui, t: &str, sub: &str) {
    ui.text(t, [X0, 90.0, X1, 122.0], 22.0, 600, TEXT, DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS);
    ui.text(sub, [X0, 122.0, X1, 144.0], 13.0, 400, DIM, DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS);
}

/// The status line (the last job's message, a language switch) or `fallback`, wrapped over two lines above the footer.
unsafe fn note(ui: &Ui, fallback: &str) {
    let (m, ok) = MSG.lock().unwrap().clone();
    let (m, fg) = if m.is_empty() { (fallback.to_string(), DIM) } else if ok { (m, GOOD) } else { (m, BAD) };
    ui.text(&m, [X0, NOTE_Y, X1, FOOT_Y - 8.0], 12.0, 400, fg, DT_LEFT | DT_WORDBREAK | DT_EDITCONTROL | DT_END_ELLIPSIS);
}

fn names(v: &[usize]) -> String {
    v.iter().map(|&i| PLUGINS[i].name).collect::<Vec<_>>().join(", ")
}

pub(crate) unsafe fn paint(hwnd: HWND) {
    let mut ps: PAINTSTRUCT = zeroed();
    let hdc = BeginPaint(hwnd, &mut ps);
    let mut rc: RECT = zeroed();
    GetClientRect(hwnd, &mut rc);
    let (cw, ch) = (rc.right, rc.bottom);
    let mem = CreateCompatibleDC(hdc);
    let bmp = CreateCompatibleBitmap(hdc, cw, ch);
    let ob = SelectObject(mem, bmp);
    let s = scale();
    let bgb = CreateSolidBrush(BG);
    FillRect(mem, &rc, bgb);
    DeleteObject(bgb);
    let mut g = null_mut::<GpGraphics>();
    GdipCreateFromHDC(mem, &mut g);
    GdipSetSmoothingMode(g, SmoothingModeAntiAlias);
    let ui = Ui { dc: mem, g, s };
    let w = cw as f32 / s;
    let hover = HOVER.get();
    let hot = |id: usize| hover == Some(id);
    let icons = winrust_common::style::icon_face();
    let hair = 1.0 / s;
    let page = PAGE.get();
    let custom = CUSTOM.get();

    // header: the name on the left, the language on the right, the accent line under it
    ui.shape([0.0, 0.0, w, HEAD_H], 0.0, argb(HEAD, 255));
    ui.shape([0.0, HEAD_H, w, HEAD_H + 3.0], 0.0, argb(ACCENT, 255));
    let mark = [PAD - 6.0, 19.0, PAD + 20.0, 45.0];
    ui.round(mark, 3.0, ACCENT, ACCENT);
    ui.text("W", mark, 16.0, 800, ON_ACCENT, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
    ui.text("WinRust", [PAD + 32.0, 0.0, X1, HEAD_H], 20.0, 700, TEXT, DT_LEFT | DT_SINGLELINE | DT_VCENTER);
    let tw = ui.width("WinRust", 20.0, 700);
    ui.text(t!("Установка", "Setup", ja: "セットアップ", pt: "Instalação", zh: "安装程序"), [PAD + 40.0 + tw, 0.0, X1 - 140.0, HEAD_H], 16.0, 300, DIM, DT_LEFT | DT_SINGLELINE | DT_VCENTER);
    let lb = lang_box(w);
    let open = LANG_OPEN.get().is_some();
    ui.round(lb, 3.0, if hot(ID_LANG) || open { rgb(40, 40, 40) } else { HEAD }, if open { ACCENT } else { rgb(70, 70, 70) });
    ui.text(i18n::lang().name(), [lb[0] + 10.0, lb[1], lb[2] - 26.0, lb[3]], 13.0, 400, TEXT, DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS);
    ui.text_in(if open { "\u{E70E}" } else { "\u{E70D}" }, [lb[2] - 22.0, lb[1], lb[2] - 8.0, lb[3]], 9.0, 400, DIM, DT_RIGHT | DT_SINGLELINE | DT_VCENTER, icons);

    // the steps on the left: the current one lit, the done ones ticked
    ui.shape([0.0, HEAD_H + 3.0, SIDE_W, WIN_H + 1.0], 0.0, argb(SIDE, 255));
    let steps_names = [
        t!("Параметры", "Options", ja: "オプション", pt: "Opções", zh: "选项"),
        t!("Компоненты", "Components", ja: "コンポーネント", pt: "Componentes", zh: "组件"),
        t!("Установка", "Installing", ja: "インストール", pt: "Instalação", zh: "安装"),
        t!("Готово", "Finish", ja: "完了", pt: "Concluir", zh: "完成"),
    ];
    for (k, label) in steps_names.iter().enumerate() {
        let y = 100.0 + k as f32 * 46.0;
        let cur = k as u8 == page;
        // Express picks the components itself: that step counts as done once it is past
        let done = (k as u8) < page;
        if cur {
            ui.shape([0.0, y, 4.0, y + 38.0], 0.0, argb(ACCENT, 255));
            ui.shape([4.0, y, SIDE_W, y + 38.0], 0.0, argb(rgb(34, 34, 34), 255));
        }
        let c = [22.0, y + 8.0, 44.0, y + 30.0];
        if done {
            ui.round(c, 11.0, ACCENT, ACCENT);
            ui.text_in("\u{E73E}", c, 10.0, 700, ON_ACCENT, DT_CENTER | DT_VCENTER | DT_SINGLELINE, icons);
        } else {
            let line = if cur { ACCENT } else { rgb(80, 80, 80) };
            ui.round(c, 11.0, SIDE, line);
            ui.text(&(k + 1).to_string(), c, 11.0, 600, if cur { TEXT } else { DIM }, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
        }
        let fg = if cur { TEXT } else if done { lerp(TEXT, DIM, 0.3) } else { DIM };
        ui.text(label, [56.0, y, SIDE_W - 8.0, y + 38.0], 13.5, if cur { 600 } else { 400 }, fg, DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS);
    }
    if let Some(v) = update::NEWER.lock().unwrap().clone() {
        let t = tf!("Новая версия {} ›", "New version {} ›", ja: "新しいバージョン {} ›", pt: "Nova versão {} ›", zh: "新版本 {} ›", v);
        ui.text(&t, update_box(w), 12.0, 600, if hot(ID_UPDATE) { TEXT } else { ACCENT }, DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS);
    }
    ui.text(&update::current(), [20.0, WIN_H - 48.0, SIDE_W - 12.0, WIN_H - 24.0], 12.0, 400, lerp(DIM, SIDE, 0.25), DT_LEFT | DT_SINGLELINE | DT_VCENTER);
    ui.shape([SIDE_W, FOOT_Y, w, FOOT_Y + hair], 0.0, argb(BORDER, 255));

    let n = PLUGINS.len();
    let n_inst = rows().filter(|&i| INSTALLED[i].load(Relaxed)).count();
    match page {
        0 => {
            let sub = if n_inst == 0 {
                t!("Выберите, как установить WinRust.", "Choose how to install WinRust.", ja: "WinRust のインストール方法を選んでください。", pt: "Escolha como instalar a WinRust.", zh: "请选择 WinRust 的安装方式。").to_string()
            } else {
                tf!("Установлено компонентов: {}. Выберите, что сделать.", "{} components installed. Choose what to do.", ja: "インストール済みのコンポーネント: {} 個。操作を選んでください。", pt: "{} componentes instalados. Escolha o que fazer.", zh: "已安装 {} 个组件。请选择要执行的操作。", n_inst)
            };
            title(&ui, t!("Параметры установки", "Installation options", ja: "インストール オプション", pt: "Opções de instalação", zh: "安装选项"), &sub);
            // the system check
            let admin = winrust_common::proc::elevated();
            let checks = [
                (admin, if admin { t!("Права администратора есть", "Running as administrator", ja: "管理者として実行中", pt: "Executando como administrador", zh: "正以管理员身份运行") } else { t!("Нет прав администратора: запустите от имени администратора", "Not an administrator: run the installer as administrator", ja: "管理者ではありません: 管理者として実行してください", pt: "Sem direitos de administrador: execute como administrador", zh: "没有管理员权限：请以管理员身份运行") }),
                (true, t!("Перед изменениями создаётся точка восстановления", "A restore point is created before any change", ja: "変更の前に復元ポイントを作成します", pt: "Um ponto de restauração é criado antes de qualquer alteração", zh: "任何更改之前都会创建还原点")),
            ];
            for (k, (ok, text)) in checks.iter().enumerate() {
                let y = 156.0 + k as f32 * 22.0;
                ui.text_in(if *ok { "\u{E73E}" } else { "\u{E711}" }, [X0, y, X0 + 16.0, y + 20.0], 11.0, 700, if *ok { ACCENT } else { BAD }, DT_LEFT | DT_SINGLELINE | DT_VCENTER, icons);
                ui.text(text, [X0 + 22.0, y, X1, y + 20.0], 12.5, 400, DIM, DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS);
            }
            let adds = express_adds();
            let express_text = if adds.is_empty() {
                t!("Всё рекомендуемое уже стоит: установленные компоненты обновятся из этой версии, настройки сохранятся.", "Everything recommended is already there: the installed components are updated from this version, settings are kept.", ja: "推奨コンポーネントはすべてインストール済みです。インストール済みのものをこのバージョンで更新し、設定は保持されます。", pt: "Tudo o que é recomendado já está instalado: os componentes instalados são atualizados a partir desta versão, as configurações são mantidas.", zh: "推荐的组件都已安装：将用此版本更新已安装的组件，设置会保留。").to_string()
            } else {
                tf!("Установит: {}. Проводник и очистка Windows не трогаются, ничего не удаляется.", "Installs {}. Explorer and the Windows clean-up are left alone, nothing is removed.", ja: "インストールするもの: {}。エクスプローラーと Windows のクリーンアップには触れず、何も削除しません。", pt: "Instala: {}. O Explorer e a limpeza do Windows ficam como estão, nada é removido.", zh: "将安装：{}。不动资源管理器和 Windows 清理，也不会删除任何东西。", names(&adds))
            };
            let cards = [
                (t!("Экспресс (рекомендуется)", "Express (recommended)", ja: "高速 (推奨)", pt: "Expressa (recomendada)", zh: "快速（推荐）"), express_text),
                (
                    t!("Выборочная установка (для опытных)", "Custom (advanced)", ja: "カスタム (上級者向け)", pt: "Personalizada (avançada)", zh: "自定义（高级）"),
                    t!("Выберите компоненты сами: в том числе замену Проводника, очистку Windows и удаление того, что уже стоит.", "Pick the components yourself, including the Explorer replacement, the Windows clean-up and removing what is installed.", ja: "コンポーネントを自分で選びます。エクスプローラーの置き換え、Windows のクリーンアップ、インストール済みのものの削除も含みます。", pt: "Escolha os componentes você mesmo, incluindo a substituição do Explorer, a limpeza do Windows e a remoção do que já está instalado.", zh: "自己挑选组件，包括替换资源管理器、清理 Windows 以及移除已安装的组件。").to_string(),
                ),
            ];
            for (k, (head, text)) in cards.iter().enumerate() {
                let b = card_box(k);
                let sel = custom == (k == 1);
                let h = hot(ID_EXPRESS + k);
                ui.round(b, 3.0, if sel { lerp(BG, ACCENT, 0.07) } else if h { rgb(44, 44, 44) } else { PANEL }, if sel { ACCENT } else { BORDER });
                radio(&ui, b[0] + 24.0, b[1] + 26.0, sel);
                ui.text(head, [b[0] + 46.0, b[1] + 14.0, b[2] - 16.0, b[1] + 38.0], 15.0, 600, TEXT, DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS);
                ui.text(text, [b[0] + 46.0, b[1] + 40.0, b[2] - 16.0, b[3] - 8.0], 12.5, 400, DIM, DT_LEFT | DT_WORDBREAK | DT_EDITCONTROL | DT_END_ELLIPSIS);
            }
            note(&ui, "");
            if n_inst > 0 {
                let rb = rollback_box(w);
                ui.text(t!("Удалить WinRust", "Remove WinRust", ja: "WinRust を削除", pt: "Remover a WinRust", zh: "移除 WinRust"), rb, 13.0, 400, if hot(ID_ROLLBACK) { lerp(BAD, TEXT, 0.3) } else { BAD }, DT_LEFT | DT_SINGLELINE | DT_VCENTER);
            }
            let label = if custom {
                t!("Далее", "Next", ja: "次へ", pt: "Avançar", zh: "下一步")
            } else if adds.is_empty() && n_inst > 0 {
                t!("Обновить", "Update", ja: "更新", pt: "Atualizar", zh: "更新")
            } else {
                t!("Установить", "Install", ja: "インストール", pt: "Instalar", zh: "安装")
            };
            button(&ui, primary_box(), label, 0, hot(ID_MAIN));
        }
        1 => {
            title(&ui, t!("Выборочная установка", "Custom installation", ja: "カスタム インストール", pt: "Instalação personalizada", zh: "自定义安装"), t!("Отметьте, что должно стоять. Снятая галочка у установленного компонента удалит его.", "Tick what should be installed. Unticking an installed component removes it.", ja: "インストールするものにチェックを入れてください。インストール済みのもののチェックを外すと削除されます。", pt: "Marque o que deve ficar instalado. Desmarcar um componente instalado o remove.", zh: "勾选要安装的组件。取消勾选已安装的组件会将其移除。"));
            ui.text(t!("Компоненты", "Components", ja: "コンポーネント", pt: "Componentes", zh: "组件"), [X0, LIST_TOP - 28.0, X1, LIST_TOP - 6.0], 12.0, 700, DIM, DT_LEFT | DT_SINGLELINE | DT_VCENTER);
            let all_on = chosen().filter(|&i| PLUGINS[i].bulk).count() == N_READY;
            ui.text(if all_on { t!("Снять все", "Select none", ja: "すべて選択解除", pt: "Desmarcar tudo", zh: "全不选") } else { t!("Выбрать все", "Select all", ja: "すべて選択", pt: "Selecionar tudo", zh: "全选") }, all_box(w), 12.5, 600, if hot(ID_ALL) { TEXT } else { ACCENT }, DT_RIGHT | DT_SINGLELINE | DT_VCENTER);
            if N_EXTRA > 0 {
                ui.text(t!("Дополнительно", "Extras", ja: "オプション", pt: "Extras", zh: "附加功能"), [X0, EXTRA_SEC_Y, X1, EXTRA_SEC_Y + 22.0], 12.0, 700, DIM, DT_LEFT | DT_SINGLELINE | DT_VCENTER);
                ui.text(t!("Только по вашему выбору", "Only when you choose", ja: "選択したときのみ", pt: "Somente quando você escolher", zh: "仅在手动选择时"), [X0, EXTRA_SEC_Y, X1, EXTRA_SEC_Y + 22.0], 12.0, 400, lerp(DIM, BG, 0.3), DT_RIGHT | DT_SINGLELINE | DT_VCENTER);
            }
            let (to_rm, to_add) = diff();
            let gear = gear_box(w);
            for (k, i) in rows().enumerate() {
                let p = &PLUGINS[i];
                let r = row_box(k, w);
                let mid = (r[1] + r[3]) / 2.0;
                if hot(ID_CHECK + i) {
                    ui.shape(r, 0.0, argb(rgb(44, 44, 44), 255));
                }
                ui.shape([r[0], r[3] - hair, r[2], r[3]], 0.0, argb(rgb(48, 48, 48), 255));
                check(&ui, [X0 + 8.0, mid - 9.0, X0 + 26.0, mid + 9.0], SELECTED[i].load(Relaxed), locked(i));
                ui.text_in(p.glyph, [X0 + 32.0, mid - 10.0, X0 + 50.0, mid + 10.0], 14.0, 400, tile_color(p.name), DT_CENTER | DT_VCENTER | DT_SINGLELINE, icons);
                ui.text(p.name, [NAME_X, r[1], DESC_X - 34.0, r[3]], 13.5, 600, TEXT, DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS);
                if let Some((_, gb)) = gear.filter(|&(gi, _)| gi == i) {
                    ui.text_in("\u{E713}", gb, 14.0, 400, if hot(ID_GEAR) { TEXT } else { DIM }, DT_CENTER | DT_VCENTER | DT_SINGLELINE, icons);
                }
                let installed = INSTALLED[i].load(Relaxed);
                let status = if to_rm.contains(&i) {
                    Some((t!("Будет удалён", "Will be removed", ja: "削除されます", pt: "Será removido", zh: "将被移除"), BAD))
                } else if to_add.contains(&i) {
                    Some((t!("Будет установлен", "Will be installed", ja: "インストールされます", pt: "Será instalado", zh: "将被安装"), ACCENT))
                } else if installed {
                    Some((if p.bulk { t!("Установлен", "Installed", ja: "インストール済み", pt: "Instalado", zh: "已安装") } else { t!("Применён", "Applied", ja: "適用済み", pt: "Aplicado", zh: "已应用") }, DIM))
                } else {
                    None
                };
                let mut right = r[2] - 8.0;
                if let Some((st, c)) = status {
                    let sw = ui.width(st, 12.0, 400);
                    ui.text(st, [right - sw - 2.0, r[1], right, r[3]], 12.0, 400, c, DT_RIGHT | DT_SINGLELINE | DT_VCENTER);
                    right -= sw + 14.0;
                }
                let desc = p.desc.get();
                let short = desc.split(". ").next().unwrap_or(desc).trim_end_matches('.');
                ui.text(short, [DESC_X, r[1], right, r[3]], 12.0, 400, DIM, DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS);
            }
            // the hovered component's full description, else the status line
            match hover.filter(|&h| h >= ID_CHECK).map(|h| h - ID_CHECK) {
                Some(i) => ui.text(PLUGINS[i].desc.get(), [X0, NOTE_Y, X1, FOOT_Y - 8.0], 12.0, 400, DIM, DT_LEFT | DT_WORDBREAK | DT_EDITCONTROL | DT_END_ELLIPSIS),
                None => note(&ui, t!("Наведите на компонент, чтобы прочитать, что он меняет.", "Point at a component to read what it changes.", ja: "コンポーネントにポインターを合わせると、何が変わるかが表示されます。", pt: "Aponte para um componente para ler o que ele altera.", zh: "将指针移到组件上，即可查看它会更改什么。")),
            }
            button(&ui, back_box(), t!("Назад", "Back", ja: "戻る", pt: "Voltar", zh: "上一步"), 1, hot(ID_BACK));
            let (label, kind) = match (to_rm.len(), to_add.len()) {
                (0, 0) if n_inst == 0 => (t!("Ничего не выбрано", "Nothing selected", ja: "未選択", pt: "Nada selecionado", zh: "未选择").to_string(), 3),
                (0, 0) => (t!("Обновить", "Update", ja: "更新", pt: "Atualizar", zh: "更新").to_string(), 0),
                (0, a) => (tf!("Установить ({a})", "Install ({a})", ja: "インストール ({a})", pt: "Instalar ({a})", zh: "安装（{a}）"), 0),
                (r, 0) => (tf!("Удалить ({r})", "Remove ({r})", ja: "削除 ({r})", pt: "Remover ({r})", zh: "移除（{r}）"), 2),
                (r, a) => (tf!("Применить +{a} −{r}", "Apply +{a} −{r}", ja: "適用 +{a} −{r}", pt: "Aplicar +{a} −{r}", zh: "应用 +{a} −{r}"), 0),
            };
            button(&ui, primary_box(), &label, kind, hot(ID_MAIN) && kind != 3);
        }
        2 => {
            let all = REMOVING_ALL.get();
            title(
                &ui,
                if all { t!("Удаление WinRust", "Removing WinRust", ja: "WinRust を削除しています", pt: "Removendo a WinRust", zh: "正在移除 WinRust") } else { t!("Установка WinRust", "Installing WinRust", ja: "WinRust をインストールしています", pt: "Instalando a WinRust", zh: "正在安装 WinRust") },
                t!("Не закрывайте окно до конца. Экран может мигнуть, пока меняются части Windows.", "Keep the window open until it finishes. The screen may flicker while Windows parts are swapped.", ja: "完了するまでウィンドウを閉じないでください。Windows の部品を入れ替える間、画面がちらつくことがあります。", pt: "Mantenha a janela aberta até terminar. A tela pode piscar enquanto partes do Windows são trocadas.", zh: "完成之前请不要关闭窗口。替换 Windows 部件时屏幕可能会闪烁。"),
            );
            let steps = STEPS.lock().unwrap().clone();
            let job = JOB.load(Relaxed);
            let pos = steps.iter().position(|&(i, _)| i == job);
            let total = steps.len().max(1);
            let bar = [X0, 186.0, X1, 194.0];
            ui.round(bar, 1.0, rgb(58, 58, 58), rgb(58, 58, 58));
            let (now_text, frac) = if all || pos.is_none() {
                // no steps to count: a segment sweeps along the track
                let t = (std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis()) % 1400) as f32 / 1400.0;
                let (a, b) = ((-0.3 + 1.3 * t).max(0.0), (1.3 * t).min(1.0));
                let bw = bar[2] - bar[0];
                ui.round([bar[0] + bw * a, bar[1], bar[0] + bw * b, bar[3]], 1.0, ACCENT, ACCENT);
                (if all { t!("Возвращаются стандартные части Windows…", "Bringing the stock Windows parts back…", ja: "Windows 標準の部品を戻しています…", pt: "Trazendo de volta as partes originais do Windows…", zh: "正在恢复系统自带的 Windows 部件…").to_string() } else { t!("Подготовка и точка восстановления…", "Preparing and making a restore point…", ja: "準備と復元ポイントの作成中…", pt: "Preparando e criando um ponto de restauração…", zh: "正在准备并创建还原点…").to_string() }, None)
            } else {
                let q = pos.unwrap_or(0);
                let f = (q as f32 + 0.5) / total as f32;
                ui.round([bar[0], bar[1], bar[0] + (bar[2] - bar[0]) * f, bar[3]], 1.0, ACCENT, ACCENT);
                let (i, install) = steps[q];
                let verb = if install { t!("Устанавливается", "Installing", ja: "インストール中", pt: "Instalando", zh: "正在安装") } else { t!("Удаляется", "Removing", ja: "削除中", pt: "Removendo", zh: "正在移除") };
                (format!("{verb}: {}  ({} / {total})", PLUGINS[i].name, q + 1), Some(f))
            };
            ui.text(&now_text, [X0, 160.0, X1 - 60.0, 182.0], 13.0, 400, TEXT, DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS);
            if let Some(f) = frac {
                ui.text(&format!("{}%", (f * 100.0) as u32), [X1 - 60.0, 160.0, X1, 182.0], 13.0, 600, ACCENT, DT_RIGHT | DT_SINGLELINE | DT_VCENTER);
            }
            for (k, &(i, install)) in steps.iter().enumerate() {
                let y = 216.0 + k as f32 * 30.0;
                let (glyph, gc, word) = match pos {
                    Some(q) if k < q => ("\u{E73E}", ACCENT, if install { t!("Готово", "Done", ja: "完了", pt: "Pronto", zh: "完成") } else { t!("Удалён", "Removed", ja: "削除済み", pt: "Removido", zh: "已移除") }),
                    Some(q) if k == q => ("\u{E768}", ACCENT, if install { t!("Установка…", "Installing…", ja: "インストール中…", pt: "Instalando…", zh: "正在安装…") } else { t!("Удаление…", "Removing…", ja: "削除中…", pt: "Removendo…", zh: "正在移除…") }),
                    _ => ("\u{E915}", rgb(90, 90, 90), t!("В очереди", "Queued", ja: "待機中", pt: "Na fila", zh: "已排队")),
                };
                ui.text_in(glyph, [X0, y, X0 + 18.0, y + 24.0], 11.0, 700, gc, DT_LEFT | DT_SINGLELINE | DT_VCENTER, icons);
                ui.text(PLUGINS[i].name, [X0 + 26.0, y, X0 + 260.0, y + 24.0], 13.0, 400, TEXT, DT_LEFT | DT_SINGLELINE | DT_VCENTER);
                ui.text(word, [X0 + 260.0, y, X1, y + 24.0], 12.5, 400, DIM, DT_RIGHT | DT_SINGLELINE | DT_VCENTER);
            }
            let _ = n;
        }
        _ => {
            let (msg, ok) = MSG.lock().unwrap().clone();
            let all = REMOVING_ALL.get();
            let results = RESULTS.lock().unwrap().clone();
            let c = [X0, 92.0, X0 + 44.0, 136.0];
            let col = if ok { ACCENT } else { BAD };
            ui.round(c, 22.0, col, col);
            ui.text_in(if ok { "\u{E73E}" } else { "\u{E711}" }, c, 18.0, 700, ON_ACCENT, DT_CENTER | DT_VCENTER | DT_SINGLELINE, icons);
            let head = match (ok, all) {
                (true, true) => t!("WinRust удалён", "WinRust has been removed", ja: "WinRust を削除しました", pt: "A WinRust foi removida", zh: "WinRust 已移除"),
                (true, false) => t!("Установка WinRust завершена", "WinRust setup has finished", ja: "WinRust のセットアップが完了しました", pt: "A instalação da WinRust terminou", zh: "WinRust 安装已完成"),
                (false, _) => t!("Готово, но не всё получилось", "Finished, but not everything worked", ja: "完了しましたが、一部失敗しました", pt: "Concluído, mas nem tudo funcionou", zh: "已完成，但有部分失败"),
            };
            ui.text(head, [c[2] + 16.0, 92.0, X1, 118.0], 21.0, 600, TEXT, DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS);
            let sub = if !ok {
                t!("Ошибка указана ниже. Остальное установлено; можно вернуться к параметрам и повторить.", "The error is shown below. The rest is installed; go back to the options to try again.", ja: "エラーは下に表示されています。それ以外はインストール済みです。オプションに戻って再試行できます。", pt: "O erro aparece abaixo. O resto foi instalado; volte às opções para tentar de novo.", zh: "错误显示在下方。其余部分已安装；可以返回选项重试。")
            } else if all {
                t!("Вернулись поиск Windows, стандартный диспетчер задач, панель задач и Проводник.", "Windows Search, the stock Task Manager, the taskbar and Explorer are back.", ja: "Windows 検索、標準のタスク マネージャー、タスクバー、エクスプローラーが戻りました。", pt: "A Pesquisa do Windows, o Gerenciador de Tarefas padrão, a barra de tarefas e o Explorer voltaram.", zh: "Windows 搜索、系统任务管理器、任务栏和资源管理器都已恢复。")
            } else {
                t!("Компоненты уже работают. Перезагрузка не обязательна.", "The components are already running. No restart is needed.", ja: "コンポーネントはすでに動作しています。再起動は不要です。", pt: "Os componentes já estão funcionando. Não é preciso reiniciar.", zh: "组件已在运行，无需重启。")
            };
            ui.text(sub, [c[2] + 16.0, 118.0, X1, 138.0], 12.5, 400, DIM, DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS);
            if !results.is_empty() {
                ui.text(t!("Компонент", "Component", ja: "コンポーネント", pt: "Componente", zh: "组件"), [X0, 166.0, X0 + 260.0, 188.0], 12.0, 700, DIM, DT_LEFT | DT_SINGLELINE | DT_VCENTER);
                ui.text(t!("Состояние", "Status", ja: "状態", pt: "Status", zh: "状态"), [X0 + 260.0, 166.0, X1, 188.0], 12.0, 700, DIM, DT_RIGHT | DT_SINGLELINE | DT_VCENTER);
                ui.shape([X0, 190.0, X1, 190.0 + hair], 0.0, argb(BORDER, 255));
            }
            for (k, (i, install, err)) in results.iter().enumerate() {
                let y = 196.0 + k as f32 * 28.0;
                if y + 26.0 > NOTE_Y {
                    break;
                }
                ui.text(PLUGINS[*i].name, [X0, y, X0 + 260.0, y + 26.0], 13.0, 400, TEXT, DT_LEFT | DT_SINGLELINE | DT_VCENTER);
                let (word, c) = match err {
                    Some(_) => (t!("Ошибка", "Failed", ja: "失敗", pt: "Falhou", zh: "失败"), BAD),
                    None if *install => (t!("Установлен", "Installed", ja: "インストール済み", pt: "Instalado", zh: "已安装"), ACCENT),
                    None => (t!("Удалён", "Removed", ja: "削除済み", pt: "Removido", zh: "已移除"), DIM),
                };
                ui.text(word, [X0 + 260.0, y, X1, y + 26.0], 13.0, 600, c, DT_RIGHT | DT_SINGLELINE | DT_VCENTER);
            }
            ui.text(&msg, [X0, NOTE_Y, X1, FOOT_Y - 8.0], 12.0, 400, if ok { DIM } else { BAD }, DT_LEFT | DT_WORDBREAK | DT_EDITCONTROL | DT_END_ELLIPSIS);
            button(&ui, back_box(), t!("К параметрам", "Options", ja: "オプション", pt: "Opções", zh: "选项"), 1, hot(ID_BACK));
            button(&ui, primary_box(), t!("Закрыть", "Close", ja: "閉じる", pt: "Fechar", zh: "关闭"), 0, hot(ID_MAIN));
        }
    }

    // confirmation dialog over the dimmed window
    let dialog = DIALOG.with_borrow(|d| d.as_ref().map(|d| (d.title.clone(), d.text.clone(), d.ok.clone(), d.danger)));
    if let Some((title, text, ok_label, danger)) = dialog {
        ui.shape([0.0, 0.0, w, WIN_H + 1.0], 0.0, argb(0, 160));
        let (d, cancel, okb) = dialog_boxes(w);
        ui.round(d, 4.0, PANEL, BORDER);
        ui.shape([d[0], d[1], d[2], d[1] + 3.0], 0.0, argb(if danger { BAD } else { ACCENT }, 255));
        ui.text(&title, [d[0] + 24.0, d[1] + 22.0, d[2] - 24.0, d[1] + 50.0], 16.0, 600, TEXT, DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS);
        ui.text(&text, [d[0] + 24.0, d[1] + 60.0, d[2] - 24.0, cancel[1] - 14.0], 12.5, 400, DIM, DT_LEFT | DT_WORDBREAK | DT_EDITCONTROL | DT_END_ELLIPSIS);
        button(&ui, cancel, t!("Отмена", "Cancel", ja: "キャンセル", pt: "Cancelar", zh: "取消"), 1, hot(ID_CANCEL));
        button(&ui, okb, &ok_label, if danger { 2 } else { 0 }, hot(ID_OK));
    }

    // settings panel of a plugin (topbar: OneDrive switch, which shell)
    if let Some(i) = SETTINGS.get() {
        ui.shape([0.0, 0.0, w, WIN_H + 1.0], 0.0, argb(0, 160));
        let (d, _, closeb) = dialog_boxes(w);
        ui.round(d, 4.0, PANEL, BORDER);
        ui.shape([d[0], d[1], d[2], d[1] + 3.0], 0.0, argb(ACCENT, 255));
        ui.text(&tf!("Настройки {}", "{} settings", ja: "{} の設定", pt: "Configurações de {}", zh: "{} 设置", PLUGINS[i].name), [d[0] + 24.0, d[1] + 22.0, d[2] - 24.0, d[1] + 50.0], 16.0, 600, TEXT, DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS);
        let group = [d[0] + 24.0, d[1] + 72.0, d[2] - 24.0, d[1] + 72.0 + 128.0];
        let inner = rgb(56, 56, 56);
        ui.round(group, 3.0, inner, inner);
        ui.shape([group[0] + 16.0, group[1] + 64.0, group[2], group[1] + 64.0 + hair], 0.0, argb(rgb(72, 72, 74), 255));
        let rows = [
            (t!("Скрыть значок OneDrive", "Hide the OneDrive icon", ja: "OneDrive アイコンを隠す", pt: "Ocultar o ícone do OneDrive", zh: "隐藏 OneDrive 图标"), t!("Синхронизация продолжает работать", "Sync keeps working", ja: "同期は引き続き動作します", pt: "A sincronização continua funcionando", zh: "同步功能仍可正常使用"), od_hidden(), switch_box(w)),
            (t!("Заменить Проводник", "Replace Explorer", ja: "エクスプローラーを置き換える", pt: "Substituir o Explorer", zh: "替换资源管理器"), t!("Оболочкой становится rust-explorer", "rust-explorer becomes the shell", ja: "rust-explorer がシェルになります", pt: "O rust-explorer passa a ser o shell", zh: "rust-explorer 将成为外壳"), explorer_idx().is_some_and(|e| SELECTED[e].load(Relaxed)), shell_switch_box(w)),
        ];
        for (k, (name, note, on, sw)) in rows.into_iter().enumerate() {
            let t = group[1] + k as f32 * 64.0;
            ui.text(name, [group[0] + 16.0, t + 12.0, sw[0] - 12.0, t + 32.0], 14.0, 500, TEXT, DT_LEFT | DT_SINGLELINE | DT_VCENTER);
            ui.text(note, [group[0] + 16.0, t + 32.0, sw[0] - 12.0, t + 52.0], 12.0, 400, DIM, DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS);
            switch(&ui, sw, on, false);
        }
        button(&ui, closeb, t!("Готово", "Done", ja: "完了", pt: "Concluído", zh: "完成"), 0, hot(ID_CANCEL));
    }

    GdipDeleteGraphics(g);
    BitBlt(hdc, 0, 0, cw, ch, mem, 0, 0, SRCCOPY);
    SelectObject(mem, ob);
    DeleteObject(bmp);
    DeleteDC(mem);
    EndPaint(hwnd, &ps);
}

// ---------- window ----------

/// What a confirmed dialog runs.
#[derive(Clone, Copy)]
pub(crate) enum Act {
    InstallAll,
    Rollback,
}
pub(crate) struct Dialog {
    title: String,
    text: String,
    ok: String,
    danger: bool,
    act: Act,
}
thread_local! { pub(crate) static DIALOG: std::cell::RefCell<Option<Dialog>> = const { std::cell::RefCell::new(None) }; }

/// Clickable thing under the cursor (ID_*). While a panel is up only its buttons count; while a job runs nothing does.
pub(crate) unsafe fn hit(hwnd: HWND, lp: isize) -> Option<usize> {
    let (x, y) = ((lp & 0xFFFF) as i16 as f32, ((lp >> 16) & 0xFFFF) as i16 as f32);
    let mut rc: RECT = zeroed();
    let s = scale();
    GetClientRect(hwnd, &mut rc);
    let w = rc.right as f32 / s;
    let inside = |b: Box4| x >= b[0] * s && x <= b[2] * s && y >= b[1] * s && y <= b[3] * s;
    if SETTINGS.get().is_some() {
        let (_, _, closeb) = dialog_boxes(w);
        return if inside(switch_box(w)) { Some(ID_SWITCH) } else if inside(shell_switch_box(w)) { Some(ID_SHELL) } else if inside(closeb) { Some(ID_CANCEL) } else { None };
    }
    if DIALOG.with_borrow(|d| d.is_some()) {
        let (_, cancel, ok) = dialog_boxes(w);
        return if inside(ok) { Some(ID_OK) } else if inside(cancel) { Some(ID_CANCEL) } else { None };
    }
    if BUSY.load(Relaxed) {
        return None;
    }
    if inside(lang_box(w)) {
        return Some(ID_LANG);
    }
    if update::NEWER.lock().unwrap().is_some() && inside(update_box(w)) {
        return Some(ID_UPDATE);
    }
    let n_inst = rows().any(|i| INSTALLED[i].load(Relaxed));
    match PAGE.get() {
        0 => {
            if inside(card_box(0)) {
                return Some(ID_EXPRESS);
            }
            if inside(card_box(1)) {
                return Some(ID_CUSTOM);
            }
            if n_inst && inside(rollback_box(w)) {
                return Some(ID_ROLLBACK);
            }
            inside(primary_box()).then_some(ID_MAIN)
        }
        1 => {
            if gear_box(w).is_some_and(|(_, b)| inside(b)) {
                return Some(ID_GEAR);
            }
            if inside(all_box(w)) {
                return Some(ID_ALL);
            }
            if let Some(i) = rows().enumerate().find_map(|(k, i)| inside(row_box(k, w)).then_some(i)) {
                return Some(ID_CHECK + i);
            }
            if inside(back_box()) {
                return Some(ID_BACK);
            }
            (inside(primary_box()) && !(diff() == (vec![], vec![]) && !n_inst)).then_some(ID_MAIN)
        }
        3 => {
            if inside(back_box()) {
                return Some(ID_BACK);
            }
            inside(primary_box()).then_some(ID_MAIN)
        }
        _ => None,
    }
}

/// Runs `job` off the UI thread; its Ok/Err text becomes the status line and the done page follows. `which` = plugin
/// index, n = rollback.
pub(crate) fn start(hwnd: HWND, which: usize, job: impl FnOnce() -> Result<String, String> + Send + 'static) {
    JOB.store(which, Relaxed);
    BUSY.store(true, Relaxed);
    REMOVING_ALL.set(which == PLUGINS.len());
    RESULTS.lock().unwrap().clear();
    MSG.lock().unwrap().0.clear();
    HOVER.set(None);
    PAGE.set(2);
    unsafe {
        SetTimer(hwnd, 1, 40, None); // the progress page animates while it waits
        InvalidateRect(hwnd, null(), 0);
    }
    let h = hwnd as isize;
    thread::spawn(move || {
        *MSG.lock().unwrap() = match job() {
            Ok(m) => (m, true),
            Err(e) => (tf!("Ошибка: {e}", "Error: {e}", ja: "エラー: {e}", pt: "Erro: {e}", zh: "错误：{e}"), false),
        };
        refresh();
        BUSY.store(false, Relaxed);
        unsafe { PostMessageW(h as HWND, WM_DONE, 0, 0) };
    });
}

/// The job ended (WM_DONE): the done page.
pub(crate) unsafe fn finished(hwnd: HWND) {
    KillTimer(hwnd, 1);
    PAGE.set(3);
    InvalidateRect(hwnd, null(), 0);
}

/// The button's job: removes what was unticked, installs what was ticked, nothing else. With no change it updates the
/// installed components from this installer (not the extras: purge would run again). A failed step does not stop the
/// others; the failures are listed at the end. `h` = window to repaint between steps.
pub(crate) fn apply(h: isize) -> Result<String, String> {
    let (rm, add) = diff();
    let update = rm.is_empty() && add.is_empty();
    let steps: Vec<(usize, bool)> = if update {
        ready().filter(|&i| INSTALLED[i].load(Relaxed)).map(|i| (i, true)).collect()
    } else {
        rm.iter().map(|&i| (i, false)).chain(add.iter().map(|&i| (i, true))).collect()
    };
    *STEPS.lock().unwrap() = steps.clone();
    let (mut added, mut removed, mut failed, mut warned) = (Vec::new(), Vec::new(), Vec::new(), false);
    for (i, install) in steps {
        JOB.store(i, Relaxed);
        unsafe { InvalidateRect(h as HWND, null(), 0) };
        let r = if install { install_plugin(i) } else { uninstall_plugin(i) };
        match r {
            Ok(_) => {
                warned |= !WARN.lock().unwrap().is_empty(); // each step clears the list first
                if install { added.push(PLUGINS[i].name) } else { removed.push(PLUGINS[i].name) }
                RESULTS.lock().unwrap().push((i, install, None));
            }
            Err(e) => {
                failed.push(format!("{}: {e}", PLUGINS[i].name));
                RESULTS.lock().unwrap().push((i, install, Some(e)));
            }
        }
        refresh();
    }
    STEPS.lock().unwrap().clear();
    let mut done = vec![];
    if !added.is_empty() {
        done.push(format!("{} {}", if update { t!("Обновлено:", "Updated:", ja: "更新:", pt: "Atualizados:", zh: "已更新：") } else { t!("Установлено:", "Installed:", ja: "インストール:", pt: "Instalados:", zh: "已安装：") }, added.join(", ")));
    }
    if !removed.is_empty() {
        done.push(format!("{} {}", t!("Удалено:", "Removed:", ja: "削除:", pt: "Removidos:", zh: "已移除："), removed.join(", ")));
    }
    let done = done.join(". ");
    if !failed.is_empty() {
        return Err(format!("{}.{}", failed.join("; "), if done.is_empty() { String::new() } else { format!(" {done}.") }));
    }
    let note = if warned { t!(" Есть предупреждения (см. %ProgramData%\\plugin-launcher\\install.log).", " There are warnings (see %ProgramData%\\plugin-launcher\\install.log).", ja: " 警告があります (%ProgramData%\\plugin-launcher\\install.log を参照)。", pt: " Há avisos (veja %ProgramData%\\plugin-launcher\\install.log).", zh: " 有警告（见 %ProgramData%\\plugin-launcher\\install.log）。") } else { "" };
    Ok(format!("{done}.{note}"))
}

/// A purge that has run stays on: unticking it would only delete purge.exe, the apps do not come back.
fn locked(i: usize) -> bool {
    PLUGINS[i].name == "purge" && INSTALLED[i].load(Relaxed)
}

pub(crate) unsafe fn ask(hwnd: HWND, d: Dialog) {
    DIALOG.set(Some(d));
    HOVER.set(None);
    InvalidateRect(hwnd, null(), 0);
}

pub(crate) unsafe fn close_dialog(hwnd: HWND, run: bool) {
    let Some(d) = DIALOG.take() else { return };
    HOVER.set(None);
    InvalidateRect(hwnd, null(), 0);
    if !run {
        return;
    }
    match d.act {
        Act::Rollback => start(hwnd, PLUGINS.len(), rollback),
        Act::InstallAll => {
            let h = hwnd as isize;
            start(hwnd, STARTING, move || apply(h))
        }
    }
}

pub(crate) unsafe fn click(hwnd: HWND, id: usize) {
    if SETTINGS.get().is_some() {
        if id == ID_SWITCH {
            if let Err(e) = set_od_hidden(!od_hidden()) {
                *MSG.lock().unwrap() = (tf!("Ошибка: {e}", "Error: {e}", ja: "エラー: {e}", pt: "Erro: {e}", zh: "错误：{e}"), false);
            }
        } else if id == ID_SHELL {
            if let Some(e) = explorer_idx() {
                let on = !SELECTED[e].fetch_xor(true, Relaxed);
                if let Some((other, v)) = linked(PLUGINS[e].name, on) {
                    PLUGINS.iter().position(|p| p.name == other).inspect(|&o| SELECTED[o].store(v, Relaxed));
                }
            }
        } else if id == ID_CANCEL {
            SETTINGS.set(None);
        }
        HOVER.set(None);
        InvalidateRect(hwnd, null(), 0);
        return;
    }
    if id == ID_OK || id == ID_CANCEL {
        return close_dialog(hwnd, id == ID_OK);
    }
    if BUSY.load(Relaxed) {
        return;
    }
    let repaint = || InvalidateRect(hwnd, null(), 0);
    match (PAGE.get(), id) {
        (_, ID_LANG) => {
            open_lang_list(hwnd);
            HOVER.set(None);
            repaint();
        }
        (_, ID_UPDATE) => {
            let _ = Command::new("explorer.exe").arg(format!("{}/latest", update::BASE)).spawn();
        }
        (0, ID_EXPRESS) | (0, ID_CUSTOM) => {
            CUSTOM.set(id == ID_CUSTOM);
            repaint();
        }
        (0, ID_MAIN) if CUSTOM.get() => {
            MSG.lock().unwrap().0.clear();
            PAGE.set(1);
            HOVER.set(None);
            repaint();
        }
        (0, ID_MAIN) => {
            // Express: the recommended set on top of what is there, no questions (nothing is removed or replaced beyond
            // the components' own job); with everything there already it is an update
            for i in rows() {
                SELECTED[i].store(express_pick(PLUGINS[i].name, PLUGINS[i].bulk, INSTALLED[i].load(Relaxed)), Relaxed);
            }
            let h = hwnd as isize;
            start(hwnd, STARTING, move || apply(h));
        }
        (0, ID_ROLLBACK) => ask(hwnd, Dialog {
            title: t!("Удалить WinRust?", "Remove WinRust?", ja: "WinRust を削除しますか?", pt: "Remover a WinRust?", zh: "移除 WinRust？").into(),
            text: t!("Все компоненты будут удалены. Вернутся поиск Windows, стандартный диспетчер задач, панель задач и обычный Проводник.", "All components will be removed. Windows Search, the stock Task Manager, the taskbar and regular Explorer come back.", ja: "すべてのコンポーネントが削除されます。Windows 検索、標準のタスク マネージャー、タスクバー、通常のエクスプローラーが元に戻ります。", pt: "Todos os componentes serão removidos. A Pesquisa do Windows, o Gerenciador de Tarefas padrão, a barra de tarefas e o Explorer comum voltam.", zh: "所有组件都将被移除。Windows 搜索、系统自带的任务管理器、任务栏和常规资源管理器都会恢复。").into(),
            ok: t!("Удалить всё", "Remove all", ja: "すべて削除", pt: "Remover tudo", zh: "全部移除").into(),
            danger: true,
            act: Act::Rollback,
        }),
        (1, ID_BACK) => {
            PAGE.set(0);
            HOVER.set(None);
            repaint();
        }
        (1, ID_GEAR) => {
            SETTINGS.set(PLUGINS.iter().position(|p| p.settings));
            HOVER.set(None);
            repaint();
        }
        (1, ID_ALL) => {
            let on = chosen().filter(|&i| PLUGINS[i].bulk).count() < N_READY;
            ready().for_each(|i| SELECTED[i].store(on, Relaxed));
            repaint();
        }
        (1, i) if i >= ID_CHECK => {
            let i = i - ID_CHECK;
            if !locked(i) {
                let on = !SELECTED[i].fetch_xor(true, Relaxed);
                if let Some((other, v)) = linked(PLUGINS[i].name, on) {
                    PLUGINS.iter().position(|p| p.name == other).inspect(|&o| SELECTED[o].store(v, Relaxed));
                }
            }
            repaint();
        }
        (1, ID_MAIN) if diff() == (vec![], vec![]) => ask(hwnd, Dialog {
            title: t!("Обновить WinRust?", "Update WinRust?", ja: "WinRust を更新しますか?", pt: "Atualizar a WinRust?", zh: "更新 WinRust？").into(),
            text: t!("Установленные компоненты будут переустановлены из этой версии установщика. Настройки сохранятся.", "The installed components will be reinstalled from this version of the installer. Settings are kept.", ja: "インストール済みのコンポーネントを、このバージョンのインストーラーから再インストールします。設定は保持されます。", pt: "Os componentes instalados serão reinstalados a partir desta versão do instalador. As configurações são mantidas.", zh: "已安装的组件将用此版本的安装程序重新安装。设置会被保留。").into(),
            ok: t!("Обновить", "Update", ja: "更新", pt: "Atualizar", zh: "更新").into(),
            danger: false,
            act: Act::InstallAll,
        }),
        (1, ID_MAIN) => {
            let (rm, add) = diff();
            let mut text = String::new();
            if !add.is_empty() {
                text.push_str(&tf!("Будет установлено: {}.", "To be installed: {}.", ja: "インストールされるもの: {}。", pt: "Serão instalados: {}.", zh: "将安装：{}。", names(&add)));
            }
            if !rm.is_empty() {
                if !text.is_empty() {
                    text.push('\n');
                }
                text.push_str(&tf!("Будет удалено: {} (вернутся стандартные части Windows).", "To be removed: {} (the stock Windows parts come back).", ja: "削除されるもの: {} (Windows 標準の部品が元に戻ります)。", pt: "Serão removidos: {} (as partes originais do Windows voltam).", zh: "将移除：{}（系统自带的 Windows 部件会恢复）。", names(&rm)));
            }
            // what the big ones change gets its own warning
            for &i in add.iter().filter(|&&i| PLUGINS[i].name == RUST_EXPLORER || !PLUGINS[i].bulk) {
                text.push_str(&format!("\n\n{}: {}", PLUGINS[i].name, PLUGINS[i].warn.get()));
            }
            if !add.is_empty() {
                text.push_str(t!("\n\nПеред установкой создаётся точка восстановления.", "\n\nA restore point is created before installing.", ja: "\n\nインストールの前に復元ポイントを作成します。", pt: "\n\nUm ponto de restauração é criado antes de instalar.", zh: "\n\n安装前会创建还原点。"));
            }
            let purge = add.iter().any(|&i| PLUGINS[i].name == "purge");
            let (title, ok, danger) = if purge {
                (t!("Удалить приложения навсегда?", "Remove apps for good?", ja: "アプリを完全に削除しますか?", pt: "Remover os apps de vez?", zh: "永久删除应用？"), t!("Удалить навсегда", "Remove for good", ja: "完全に削除", pt: "Remover de vez", zh: "永久删除"), true)
            } else if add.is_empty() {
                (t!("Удалить выбранное?", "Remove the selected?", ja: "選択したものを削除しますか?", pt: "Remover os selecionados?", zh: "移除所选项目？"), t!("Удалить", "Remove", ja: "削除", pt: "Remover", zh: "移除"), true)
            } else if rm.is_empty() {
                (t!("Установить выбранное?", "Install the selected?", ja: "選択したものをインストールしますか?", pt: "Instalar os selecionados?", zh: "安装所选项目？"), t!("Установить", "Install", ja: "インストール", pt: "Instalar", zh: "安装"), false)
            } else {
                (t!("Применить изменения?", "Apply the changes?", ja: "変更を適用しますか?", pt: "Aplicar as alterações?", zh: "应用更改？"), t!("Применить", "Apply", ja: "適用", pt: "Aplicar", zh: "应用"), false)
            };
            ask(hwnd, Dialog { title: title.into(), text, ok: ok.into(), danger, act: Act::InstallAll });
        }
        (3, ID_BACK) => {
            reset_selection();
            MSG.lock().unwrap().0.clear();
            PAGE.set(0);
            HOVER.set(None);
            repaint();
        }
        (3, ID_MAIN) => {
            DestroyWindow(hwnd);
        }
        _ => {}
    }
}

thread_local! { static LANG_OPEN: Cell<Option<HWND>> = const { Cell::new(None) }; } // the open language list
thread_local! { static LANG_PARENT: Cell<Option<HWND>> = const { Cell::new(None) }; } // the window it belongs to
thread_local! { static LANG_HOVER: Cell<usize> = const { Cell::new(usize::MAX) }; }
thread_local! { static LANG_ANIM: Cell<Option<(std::time::Instant, i32, i32)>> = const { Cell::new(None) }; } // opened at, final x, y
const LANG_ANIM_MS: f32 = 160.0;
pub(crate) const WM_LANG_DONE: u32 = WM_APP + 9; // a language was picked in the list: say that a restart is needed
const LANGS: [i18n::Lang; 5] = [i18n::Lang::En, i18n::Lang::Ru, i18n::Lang::Ja, i18n::Lang::Pt, i18n::Lang::Zh];

unsafe fn lang_scale() -> f32 {
    let dc = GetDC(null_mut());
    let s = GetDeviceCaps(dc, LOGPIXELSX as i32) as f32 / 96.0;
    ReleaseDC(null_mut(), dc);
    s
}

/// The item of the open list under `y` (device px), if any.
fn lang_item_at(y: i32, s: f32) -> Option<usize> {
    let pad = (4.0 * s) as i32;
    let h = (LANG_ITEM_H * s) as i32;
    let i = (y - pad) / h.max(1);
    (y >= pad && (i as usize) < LANGS.len()).then_some(i as usize)
}

unsafe fn paint_lang_list(h: HWND) {
    let mut ps: PAINTSTRUCT = zeroed();
    let dc = BeginPaint(h, &mut ps);
    let mut rc: RECT = zeroed();
    GetClientRect(h, &mut rc);
    let s = lang_scale();
    let bg = CreateSolidBrush(PANEL);
    FillRect(dc, &rc, bg);
    DeleteObject(bg);
    let border = CreateSolidBrush(BORDER);
    FillRect(dc, &RECT { right: rc.right, bottom: (s as i32).max(1), ..rc }, border);
    FillRect(dc, &RECT { left: rc.left, top: rc.bottom - (s as i32).max(1), right: rc.right, bottom: rc.bottom }, border);
    FillRect(dc, &RECT { left: rc.left, top: rc.top, right: (s as i32).max(1), bottom: rc.bottom }, border);
    FillRect(dc, &RECT { left: rc.right - (s as i32).max(1), top: rc.top, right: rc.right, bottom: rc.bottom }, border);
    DeleteObject(border);
    let pad = (4.0 * s) as i32;
    let ih = (LANG_ITEM_H * s) as i32;
    let hover = LANG_HOVER.get();
    let font = CreateFontW(-(13.0 * s) as i32, 0, 0, 0, 400, 0, 0, 0, 1, 0, 0, 5, 0, wide(winrust_common::style::sans_face()).as_ptr());
    for (i, l) in LANGS.iter().enumerate() {
        let r = RECT { left: (4.0 * s) as i32, top: pad + ih * i as i32, right: rc.right - (4.0 * s) as i32, bottom: pad + ih * (i as i32 + 1) };
        if i == hover {
            let b = CreateSolidBrush(lerp(PANEL, TEXT, 0.1));
            FillRect(dc, &r, b);
            DeleteObject(b);
        }
        let cur = *l == i18n::lang();
        let f = SelectObject(dc, font);
        SetBkMode(dc, TRANSPARENT as i32);
        SetTextColor(dc, if cur { ACCENT } else { TEXT });
        let mut tr = r;
        DrawTextW(dc, wide(l.name()).as_ptr(), -1, &mut tr, DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX);
        if cur {
            let icon = CreateFontW(-(12.0 * s) as i32, 0, 0, 0, 400, 0, 0, 0, 1, 0, 0, 5, 0, wide(winrust_common::style::icon_face()).as_ptr());
            let fi = SelectObject(dc, icon);
            let mut tr = r;
            DrawTextW(dc, wide("\u{E73E}").as_ptr(), -1, &mut tr, DT_RIGHT | DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX);
            SelectObject(dc, fi);
            DeleteObject(icon);
        }
        SelectObject(dc, f);
    }
    DeleteObject(font);
    EndPaint(h, &mut ps);
}

unsafe extern "system" fn lang_list_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_PAINT => {
            paint_lang_list(h);
            0
        }
        WM_MOUSEMOVE => {
            let s = lang_scale();
            let y = (lp >> 16) as i16 as i32;
            let i = lang_item_at(y, s).unwrap_or(usize::MAX);
            if i != LANG_HOVER.get() {
                LANG_HOVER.set(i);
                InvalidateRect(h, null(), 0);
            }
            let mut t = TRACKMOUSEEVENT { cbSize: size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: h, dwHoverTime: 0 };
            TrackMouseEvent(&mut t);
            0
        }
        WM_MOUSELEAVE => {
            LANG_HOVER.set(usize::MAX);
            InvalidateRect(h, null(), 0);
            0
        }
        WM_LBUTTONUP => {
            let s = lang_scale();
            let y = (lp >> 16) as i16 as i32;
            if let Some(i) = lang_item_at(y, s) {
                let written = i18n::set_lang(LANGS[i]);
                // the installer repaints itself; the other programs read lang.txt when they start
                if let Some(parent) = LANG_PARENT.get() {
                    PostMessageW(parent, WM_LANG_DONE, written as usize, 0);
                }
            }
            DestroyWindow(h);
            0
        }
        WM_KEYDOWN if wp as u16 == VK_ESCAPE => {
            DestroyWindow(h);
            0
        }
        WM_KILLFOCUS => {
            DestroyWindow(h);
            0
        }
        WM_TIMER => {
            // opening animation: fades in while sliding down into place, ease-out
            if let Some((t0, x, y)) = LANG_ANIM.get() {
                let t = (t0.elapsed().as_secs_f32() * 1000.0 / LANG_ANIM_MS).min(1.0);
                let e = 1.0 - (1.0 - t).powi(3);
                SetLayeredWindowAttributes(h, 0, (255.0 * e) as u8, LWA_ALPHA);
                SetWindowPos(h, null_mut(), x, y - (8.0 * lang_scale() * (1.0 - e)) as i32, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
                if t >= 1.0 {
                    KillTimer(h, 1);
                    LANG_ANIM.set(None);
                }
            }
            0
        }
        WM_DESTROY => {
            LANG_OPEN.set(None);
            LANG_ANIM.set(None);
            if let Some(p) = LANG_PARENT.get() {
                InvalidateRect(p, null(), 0); // the caret points down again
            }
            0
        }
        _ => DefWindowProcW(h, msg, wp, lp),
    }
}

/// Opens the language list under the drop-down box in the top right corner.
pub(crate) unsafe fn open_lang_list(hwnd: HWND) {
    if LANG_OPEN.get().is_some() {
        return;
    }
    let s = lang_scale();
    let hi = GetModuleHandleW(null());
    let cls = wide(LANG_LIST);
    let wc = WNDCLASSW { lpfnWndProc: Some(lang_list_proc), hInstance: hi, hCursor: LoadCursorW(null_mut(), IDC_ARROW), lpszClassName: cls.as_ptr(), ..zeroed() };
    RegisterClassW(&wc);
    let b = lang_box(WIN_W);
    let mut pt = POINT { x: (b[0] * s) as i32, y: (b[3] * s) as i32 + 2 };
    ClientToScreen(hwnd, &mut pt);
    let w = ((b[2] - b[0]) * s) as i32;
    let h = ((LANG_ITEM_H * LANGS.len() as f32 + 8.0) * s) as i32;
    let list = CreateWindowExW(WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_LAYERED, cls.as_ptr(), null(), WS_POPUP, pt.x, pt.y, w, h, null_mut(), null_mut(), hi, null());
    LANG_HOVER.set(usize::MAX);
    LANG_PARENT.set(Some(hwnd));
    LANG_OPEN.set(Some(list));
    SetLayeredWindowAttributes(list, 0, 0, LWA_ALPHA);
    LANG_ANIM.set(Some((std::time::Instant::now(), pt.x, pt.y)));
    SetTimer(list, 1, 10, None);
    ShowWindow(list, SW_SHOW);
    SetForegroundWindow(list);
    InvalidateRect(hwnd, null(), 0); // the caret points up while the list is open
}

/// The status line after a language switch: the other programs pick lang.txt up when they start. `written` is false
/// when lang.txt could not be written (the installer runs without administrator rights), which is said instead.
pub(crate) fn lang_changed(hwnd: HWND, written: bool) {
    let (text, ok) = if written {
        (
            t!(
                "Язык изменён. Перезапустите программы WinRust (или выйдите из системы), чтобы он применился везде.",
                "Language changed. Restart the WinRust programs (or sign out) to apply it everywhere.",
                ja: "言語を変更しました。すべてに反映するには WinRust の各プログラムを再起動 (またはサインアウト) してください。",
                pt: "Idioma alterado. Reinicie os programas da WinRust (ou saia da sessão) para aplicá-lo em todos.",
                zh: "语言已切换。请重启 WinRust 程序（或注销后重新登录），让所有程序都生效。"
            ),
            true,
        )
    } else {
        (
            t!(
                "Язык переключён только в этом окне: %ProgramData%\\WinRust\\lang.txt защищён от записи. Запустите установщик от имени администратора, чтобы он применился везде.",
                "Only this window switched: %ProgramData%\\WinRust\\lang.txt could not be written. Run the installer as administrator to apply the language everywhere.",
                ja: "このウィンドウだけが切り替わりました: %ProgramData%\\WinRust\\lang.txt に書き込めません。すべてに反映するにはインストーラーを管理者として実行してください。",
                pt: "Somente esta janela mudou: não foi possível gravar %ProgramData%\\WinRust\\lang.txt. Execute o instalador como administrador para aplicar o idioma em todos.",
                zh: "只有这个窗口切换了：无法写入 %ProgramData%\\WinRust\\lang.txt。请以管理员身份运行安装程序，才能让所有程序都生效。"
            ),
            false,
        )
    };
    *MSG.lock().unwrap() = (text.into(), ok);
    unsafe { InvalidateRect(hwnd, null(), 0) };
}

pub(crate) unsafe extern "system" fn proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_PAINT => paint(hwnd),
        WM_ERASEBKGND => return 1,
        WM_MOUSEMOVE => {
            let h = hit(hwnd, lp);
            if h != HOVER.get() {
                HOVER.set(h);
                InvalidateRect(hwnd, null(), 0);
            }
            let mut t = TRACKMOUSEEVENT { cbSize: size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: hwnd, dwHoverTime: 0 };
            TrackMouseEvent(&mut t);
        }
        WM_SETCURSOR if (lp & 0xFFFF) as u32 == HTCLIENT => {
            SetCursor(LoadCursorW(null_mut(), if HOVER.get().is_some() { IDC_HAND } else { IDC_ARROW }));
            return 1;
        }
        WM_MOUSELEAVE => {
            HOVER.set(None);
            InvalidateRect(hwnd, null(), 0);
        }
        WM_LBUTTONUP => {
            if let Some(i) = hit(hwnd, lp) {
                click(hwnd, i);
            }
        }
        WM_KEYDOWN if SETTINGS.get().is_some() && wp as u16 == VK_ESCAPE => {
            SETTINGS.set(None);
            InvalidateRect(hwnd, null(), 0);
        }
        WM_KEYDOWN if DIALOG.with_borrow(|d| d.is_some()) => match wp as u16 {
            VK_ESCAPE => close_dialog(hwnd, false),
            VK_RETURN if DIALOG.with_borrow(|d| d.as_ref().is_some_and(|d| !d.danger)) => close_dialog(hwnd, true), // a dangerous one needs the click

            _ => {}
        },
        WM_LANG_DONE => lang_changed(hwnd, wp != 0),
        WM_DONE => finished(hwnd),
        WM_TIMER if wp == 1 => {
            InvalidateRect(hwnd, null(), 0); // the progress page's animation
        }
        WM_CLOSE if BUSY.load(Relaxed) => {} // closing mid-install would leave the system half-changed
        WM_DESTROY => {
            LANG_OPEN.with(|o| {
                if let Some(h) = o.take() {
                    DestroyWindow(h);
                }
            });
            PostQuitMessage(0)
        }
        _ => return DefWindowProcW(hwnd, msg, wp, lp),
    }
    0
}

