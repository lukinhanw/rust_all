//! The user's wallpaper as the BMP the rust-explorer desktop paints (%LOCALAPPDATA%\rs-shell\wallpaper.bmp): without
//! Explorer nothing else draws it, and its transcoder is gone too. rs-shell makes it at sign-in; the desktop remakes it when
//! the wallpaper changes (WM_SETTINGCHANGE), so a new picture shows at once instead of after the next sign-in.
use std::path::{Path, PathBuf};

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain([0]).collect()
}

/// HKCU\Control Panel\Desktop\WallPaper through the registry API: `reg query` prints in the OEM code page, which garbled
/// non-ASCII paths (a Cyrillic user name), and REG_EXPAND_SZ came back unexpanded.
pub fn value() -> Option<String> {
    use windows_sys::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_EXPAND_SZ, RRF_RT_REG_SZ, RegGetValueW};
    let mut buf = vec![0u16; 1024];
    let mut len = (buf.len() * 2) as u32;
    let r = unsafe { RegGetValueW(HKEY_CURRENT_USER, wide(r"Control Panel\Desktop").as_ptr(), wide("WallPaper").as_ptr(), RRF_RT_REG_SZ | RRF_RT_REG_EXPAND_SZ, std::ptr::null_mut(), buf.as_mut_ptr().cast(), &mut len) };
    if r != 0 {
        return None;
    }
    buf.truncate((len as usize / 2).saturating_sub(1));
    Some(String::from_utf16_lossy(&buf)).filter(|s| !s.trim().is_empty())
}

/// The picture to show: the WallPaper value, else Explorer's own decoded copy (Spotlight, slideshow and a moved or deleted
/// source still leave it). None = a solid colour.
pub fn source() -> Option<PathBuf> {
    let transcoded = std::env::var_os("APPDATA").map(|a| PathBuf::from(a).join(r"Microsoft\Windows\Themes\TranscodedWallpaper"));
    value().map(PathBuf::from).into_iter().chain(transcoded).find(|p| p.is_file())
}

/// %LOCALAPPDATA%\rs-shell\wallpaper.bmp: what the rust-explorer desktop paints.
pub fn bmp_path() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA").map(|b| PathBuf::from(b).join("rs-shell").join("wallpaper.bmp"))
}

/// Which picture the BMP was made from, next to it: path, size and time of the source.
fn stamp_path(bmp: &Path) -> PathBuf {
    bmp.with_extension("src.txt")
}

/// What identifies a source picture: its path, size and modification time. Comparing only times (the BMP newer than the
/// source = up to date) kept the old picture whenever the new one was an older file, which most photos are.
fn stamp(src: &Path) -> Option<String> {
    let m = std::fs::metadata(src).ok()?;
    let t = m.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_nanos();
    Some(format!("{}\n{}\n{}", src.to_string_lossy().to_lowercase(), m.len(), t))
}

/// Makes the BMP from the current wallpaper if it was made from something else (or is missing). Returns the file to show
/// (a .bmp wallpaper itself, else the BMP) and whether the BMP changed. A failed conversion leaves no stale or missing path
/// behind: the old code then pointed Windows at a file that did not exist, and the desktop went black (#108, #111).
pub fn sync() -> (Option<PathBuf>, bool) {
    static ONE: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _one = ONE.lock().unwrap_or_else(|e| e.into_inner()); // a burst of WM_SETTINGCHANGE must not write the BMP twice at once
    let (Some(src), Some(bmp)) = (source(), bmp_path()) else { return (None, false) };
    let Some(want) = stamp(&src) else { return (None, false) };
    let is_bmp = src.extension().is_some_and(|e| e.eq_ignore_ascii_case("bmp"));
    let show = |bmp: PathBuf| Some(if is_bmp { src.clone() } else { bmp });
    let stamp_file = stamp_path(&bmp);
    if bmp.is_file() && std::fs::read_to_string(&stamp_file).is_ok_and(|s| s == want) {
        return (show(bmp), false);
    }
    let _ = std::fs::create_dir_all(bmp.parent().unwrap_or(Path::new(".")));
    let _ = std::fs::remove_file(&stamp_file);
    let _ = std::fs::remove_file(&bmp); // the desktop must not keep painting an old conversion
    if is_bmp {
        let _ = std::fs::copy(&src, &bmp);
    } else {
        to_bmp(&src.to_string_lossy(), &bmp);
    }
    if !bmp.is_file() {
        return (None, true);
    }
    let _ = std::fs::write(&stamp_file, want);
    (show(bmp), true)
}

/// GDI+ decodes any format it knows and saves a BMP (what the desktop paints without explorer's transcoder).
pub fn to_bmp(src: &str, dst: &Path) {
    use windows_sys::{Win32::Graphics::GdiPlus::*, core::GUID};
    const BMP: GUID = GUID { data1: 0x557C_F400, data2: 0x1A04, data3: 0x11D3, data4: [0x9A, 0x73, 0x00, 0x00, 0xF8, 0x1E, 0xF3, 0x2E] }; // image/bmp encoder
    unsafe {
        let (mut token, mut input): (usize, GdiplusStartupInput) = (0, std::mem::zeroed());
        input.GdiplusVersion = 1;
        if GdiplusStartup(&mut token, &input, std::ptr::null_mut()) != 0 {
            return;
        }
        let mut img = std::ptr::null_mut();
        if GdipLoadImageFromFile(wide(src).as_ptr(), &mut img) == 0 {
            GdipSaveImageToFile(img, wide(&dst.to_string_lossy()).as_ptr(), &BMP, std::ptr::null());
            GdipDisposeImage(img);
        }
        GdiplusShutdown(token);
    }
}

#[cfg(test)]
mod tests {
    use super::{stamp, to_bmp};

    #[test]
    fn converts_a_jpeg_to_bmp() {
        let src = r"C:\Windows\Web\Wallpaper\Windows\img0.jpg";
        if !std::path::Path::new(src).is_file() {
            return;
        }
        let dst = std::env::temp_dir().join("winrust-wallpaper-test.bmp");
        let _ = std::fs::remove_file(&dst);
        to_bmp(src, &dst);
        assert_eq!(&std::fs::read(&dst).unwrap()[..2], b"BM");
        let _ = std::fs::remove_file(dst);
    }

    #[test]
    fn an_older_picture_is_still_a_different_wallpaper() {
        let dir = std::env::temp_dir().join("winrust-wallpaper-stamp");
        let _ = std::fs::create_dir_all(&dir);
        let (a, b) = (dir.join("a.png"), dir.join("b.png"));
        std::fs::write(&a, b"aaaa").unwrap();
        std::fs::write(&b, b"aaaa").unwrap();
        // b written after a: the old time check would have called a BMP made from b up to date for a
        assert_ne!(stamp(&a), stamp(&b));
        assert_eq!(stamp(&a), stamp(&a));
        assert_eq!(stamp(&dir.join("missing.png")), None);
        let _ = std::fs::remove_dir_all(dir);
    }
}
