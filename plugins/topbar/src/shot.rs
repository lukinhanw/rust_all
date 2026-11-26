//! topbar: shot (split out of main.rs; shares its items through `super::*`).
use super::*;

/// PrtSc without explorer: the whole desktop (all monitors) to the clipboard and to Pictures\Screenshots\<date time>.png.
/// The system's own capture overlay (ms-screenclip:) does not come up without explorer as the shell. Returns the file name.
/// `topbar --shot` does the same from a command line.
pub(crate) unsafe fn screenshot() -> Option<String> {
    let prev = windows_sys::Win32::UI::HiDpi::SetThreadDpiAwarenessContext(-4isize as _); // real pixels of every monitor (#77)
    let (x, y, w, h) = (GetSystemMetrics(76), GetSystemMetrics(77), GetSystemMetrics(78), GetSystemMetrics(79)); // virtual screen
    if w <= 0 || h <= 0 {
        windows_sys::Win32::UI::HiDpi::SetThreadDpiAwarenessContext(prev);
        return None;
    }
    let (scr, mem) = (GetDC(null_mut()), CreateCompatibleDC(GetDC(null_mut())));
    let bmp = CreateCompatibleBitmap(scr, w, h);
    let old = SelectObject(mem, bmp);
    BitBlt(mem, 0, 0, w, h, scr, x, y, SRCCOPY | 0x4000_0000); // CAPTUREBLT: layered windows (the wallpaper, overlays) too
    SelectObject(mem, old);
    DeleteDC(mem);
    ReleaseDC(null_mut(), scr);
    windows_sys::Win32::UI::HiDpi::SetThreadDpiAwarenessContext(prev);
    save_bitmap(bmp)
}

/// Saves a screen bitmap as Pictures\Screenshots\Screenshot <date time>.png and puts it on the clipboard (which then owns it).
/// Returns the file name when either worked.
pub(crate) unsafe fn save_bitmap(bmp: HBITMAP) -> Option<String> {
    use windows_sys::Win32::{
        Graphics::GdiPlus::{GdipCreateBitmapFromHBITMAP, GdipDisposeImage, GdipSaveImageToFile, GpBitmap, GpImage},
        System::DataExchange::{CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData},
    };
    let mut t: SYSTEMTIME = zeroed();
    GetLocalTime(&mut t);
    let name = format!("Screenshot {:04}-{:02}-{:02} {:02}-{:02}-{:02}.png", t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond);
    let dir = PathBuf::from(std::env::var_os("USERPROFILE").unwrap_or_default()).join(r"Pictures\Screenshots");
    let _ = std::fs::create_dir_all(&dir);
    let mut gb: *mut GpBitmap = null_mut();
    let mut saved = false;
    if GdipCreateBitmapFromHBITMAP(bmp, null_mut(), &mut gb) == 0 && !gb.is_null() {
        let png = windows_sys::core::GUID { data1: 0x557c_f406, data2: 0x1a04, data3: 0x11d3, data4: [0x9a, 0x73, 0x00, 0x00, 0xf8, 0x1e, 0xf3, 0x2e] }; // PNG encoder
        saved = GdipSaveImageToFile(gb as *mut GpImage, wide(&dir.join(&name).to_string_lossy()).as_ptr(), &png, null()) == 0;
        GdipDisposeImage(gb as *mut GpImage);
    }
    let mut clip = false;
    if OpenClipboard(null_mut()) != 0 {
        EmptyClipboard();
        clip = !SetClipboardData(2, bmp as _).is_null(); // CF_BITMAP: the clipboard owns the bitmap from here on
        CloseClipboard();
    }
    if !clip {
        DeleteObject(bmp);
    }
    (saved || clip).then_some(name)
}

/// The clipper saved a region (its file name is left in `clip::DONE`): say so above the bar for a moment.
pub(crate) unsafe fn clip_notice(bar: HWND) {
    let name = clip::DONE.lock().ok().and_then(|mut d| d.take()).unwrap_or_default();
    KillTimer(bar, 2);
    show_tip(tf!("Фрагмент сохранён: {name}\nи скопирован в буфер обмена", "Snip saved: {name}\nand copied to the clipboard", ja: "スニップを保存しました: {name}\nクリップボードにもコピーしました", pt: "Recorte salvo: {name}\ne copiado para a área de transferência", zh: "截图已保存：{name}\n并已复制到剪贴板"), GetSystemMetrics(SM_CXSCREEN) / 2);
    SetTimer(bar, 6, 2500, None);
}

/// PrtSc pressed: take the screenshot, then say so above the bar for a moment.
pub(crate) unsafe fn screenshot_notice(bar: HWND) {
    let msg = match screenshot() {
        Some(n) => tf!("Скриншот сохранён: {n}\nи скопирован в буфер обмена", "Screenshot saved: {n}\nand copied to the clipboard", ja: "スクリーンショットを保存しました: {n}\nクリップボードにもコピーしました", pt: "Captura de tela salva: {n}\ne copiada para a área de transferência", zh: "截图已保存：{n}\n并已复制到剪贴板"),
        None => t!("Не удалось сделать скриншот", "Could not take a screenshot", ja: "スクリーンショットを撮影できませんでした", pt: "Não foi possível capturar a tela", zh: "无法截图").to_string(),
    };
    KillTimer(bar, 2);
    show_tip(msg, GetSystemMetrics(SM_CXSCREEN) / 2);
    SetTimer(bar, 6, 2500, None); // timer 6 hides the tip
}
