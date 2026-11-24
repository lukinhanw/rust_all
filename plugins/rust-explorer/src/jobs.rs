//! Background file jobs, faster than the shell's: copying (several files at once on SSDs, unbuffered I/O for big files),
//! ZIP packing (files compressed on all cores, by our own deflate) and unpacking ZIP / 7z. Progress shows in the status line (`status_text`),
//! the end comes to the window as WM_JOB.
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering::Relaxed},
    },
    time::Instant,
};

pub struct Progress {
    pub what: String, // "Copying", "Packing x.zip"...
    pub files: AtomicU64,
    pub total_files: AtomicU64,
    pub bytes: AtomicU64,
    pub total_bytes: AtomicU64,
    cancel: AtomicBool, // Esc: packing and unpacking stop at the next step
    started: Instant,
}

impl Progress {
    fn new(what: String) -> Arc<Progress> {
        Arc::new(Progress { what, files: 0.into(), total_files: 0.into(), bytes: 0.into(), total_bytes: 0.into(), cancel: false.into(), started: Instant::now() })
    }

    fn cancelled(&self) -> bool {
        self.cancel.load(Relaxed)
    }
}

static JOB: Mutex<Option<Arc<Progress>>> = Mutex::new(None);
static RUNNING: AtomicBool = AtomicBool::new(false);

/// The status line text while a job runs.
pub fn status_text() -> Option<String> {
    let p = JOB.lock().unwrap().clone()?;
    let (b, tb, f, tf) = (p.bytes.load(Relaxed), p.total_bytes.load(Relaxed), p.files.load(Relaxed), p.total_files.load(Relaxed));
    let secs = p.started.elapsed().as_secs_f64().max(0.1);
    let mut s = p.what.clone();
    if tf > 0 {
        s += &tf!("  \u{00B7}  файлов: {} из {}", "  \u{00B7}  {} of {} files", pt: "  \u{00B7}  {} de {} arquivos", f, tf);
    }
    if tb > 0 {
        s += &tf!("  \u{00B7}  {} из {} ({}%)  \u{00B7}  {}/с", "  \u{00B7}  {} of {} ({}%)  \u{00B7}  {}/s", pt: "  \u{00B7}  {} de {} ({}%)  \u{00B7}  {}/s", super::size_text(b), super::size_text(tb), b * 100 / tb.max(1), super::size_text((b as f64 / secs) as u64));
    }
    Some(s + "\u{2026}")
}

pub fn busy() -> bool {
    RUNNING.load(Relaxed)
}

/// Asks the running pack / unpack to stop (a copy ignores it). The half-made archive or folder is removed.
pub fn cancel() {
    if let Some(p) = JOB.lock().unwrap().as_ref() {
        p.cancel.store(true, Relaxed);
    }
}

/// Runs `f` on a worker thread; its error (or None) is posted to `h` as WM_JOB (wp = Box<Option<String>>).
fn spawn(h: isize, what: String, f: impl FnOnce(&Progress) -> Result<(), String> + Send + 'static) -> bool {
    if RUNNING.swap(true, Relaxed) {
        return false;
    }
    let p = Progress::new(what);
    *JOB.lock().unwrap() = Some(p.clone());
    let ticker = std::thread::spawn({
        let p = p.clone();
        move || {
            // repaint the status line a few times a second while the job runs
            while RUNNING.load(Relaxed) {
                unsafe { super::ui::status_dirty(h as _) };
                std::thread::sleep(std::time::Duration::from_millis(200));
            }
            drop(p);
        }
    });
    std::thread::spawn(move || {
        let r = f(&p);
        *JOB.lock().unwrap() = None;
        RUNNING.store(false, Relaxed);
        let _ = ticker.join();
        let msg = Box::new(r.err());
        unsafe { windows_sys::Win32::UI::WindowsAndMessaging::PostMessageW(h as _, super::WM_JOB, Box::into_raw(msg) as usize, 0) };
    });
    true
}

// ---------- walking ----------

/// A file or folder to create: (source, path relative to the destination, size; None = folder).
type Entry = (PathBuf, PathBuf, Option<u64>);

/// Everything under `src`, folders before their contents. Links and junctions to folders are not followed (no loops).
fn walk(src: &Path, rel: PathBuf, out: &mut Vec<Entry>) -> std::io::Result<()> {
    let md = fs::symlink_metadata(src)?;
    if md.is_dir() {
        out.push((src.to_path_buf(), rel.clone(), None));
        for e in fs::read_dir(src)? {
            let e = e?;
            walk(&e.path(), rel.join(e.file_name()), out)?;
        }
    } else if md.file_type().is_symlink() && fs::metadata(src).is_ok_and(|m| m.is_dir()) {
        // a directory link: skipped on purpose
    } else {
        out.push((src.to_path_buf(), rel, Some(fs::metadata(src).map_or(md.len(), |m| m.len()))));
    }
    Ok(())
}

/// `name`, or "name (2).ext", "name (3).ext"... the first that does not exist in `dir`.
pub fn free_name(dir: &Path, name: &str, is_dir: bool) -> PathBuf {
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) if !is_dir && !s.is_empty() => (s.to_string(), format!(".{e}")),
        _ => (name.to_string(), String::new()),
    };
    (1..).map(|n| dir.join(if n == 1 { name.to_string() } else { format!("{stem} ({n}){ext}") })).find(|p| fs::symlink_metadata(p).is_err()).unwrap()
}

/// Plain paths on a drive (not shell items like a phone or a library), and the destination not inside a source.
pub fn can_fast_copy(from: &[PathBuf], to: &Path) -> bool {
    to.is_dir() && to.has_root() && !from.is_empty() && from.iter().all(|p| p.has_root() && fs::symlink_metadata(p).is_ok() && !to.starts_with(p))
}

/// The destination accepts new files without elevation (otherwise the shell copy asks for it).
pub fn writable(dir: &Path) -> bool {
    let t = dir.join(format!(".winrust-{}.tmp", std::process::id()));
    let ok = fs::File::create(&t).is_ok();
    let _ = fs::remove_file(&t);
    ok
}

/// Same volume: a move is a rename there, the shell does that instantly.
pub fn same_volume(a: &Path, b: &Path) -> bool {
    let root = |p: &Path| p.components().next().map(|c| c.as_os_str().to_string_lossy().to_lowercase());
    root(a).is_some() && root(a) == root(b)
}

// ---------- copying ----------

/// Whether the volume of `p` is a spinning disk (seeking costs time: copy one file at a time there).
fn seek_penalty(p: &Path) -> bool {
    use windows_sys::Win32::{Foundation::*, Storage::FileSystem::*, System::{IO::DeviceIoControl, Ioctl::*}};
    let Some(letter) = p.to_string_lossy().chars().next().filter(|c| c.is_ascii_alphabetic()) else { return false };
    let dev: Vec<u16> = format!(r"\\.\{letter}:").encode_utf16().chain([0]).collect();
    unsafe {
        let h = CreateFileW(dev.as_ptr(), 0, FILE_SHARE_READ | FILE_SHARE_WRITE, std::ptr::null(), OPEN_EXISTING, 0, std::ptr::null_mut());
        if h == INVALID_HANDLE_VALUE {
            return false;
        }
        let q = STORAGE_PROPERTY_QUERY { PropertyId: StorageDeviceSeekPenaltyProperty, QueryType: PropertyStandardQuery, AdditionalParameters: [0] };
        let mut d: DEVICE_SEEK_PENALTY_DESCRIPTOR = std::mem::zeroed();
        let mut n = 0u32;
        let ok = DeviceIoControl(h, IOCTL_STORAGE_QUERY_PROPERTY, &q as *const _ as _, size_of::<STORAGE_PROPERTY_QUERY>() as u32, &mut d as *mut _ as _, size_of::<DEVICE_SEEK_PENALTY_DESCRIPTOR>() as u32, &mut n, std::ptr::null_mut()) != 0;
        CloseHandle(h);
        ok && d.IncursSeekPenalty
    }
}

const BIG: u64 = 256 << 20;

/// CopyFileExW; big files without the cache (faster for them, and they do not push everything else out of memory).
fn copy_file(src: &Path, dst: &Path, size: u64, p: &Progress) -> std::io::Result<()> {
    use windows_sys::Win32::Storage::FileSystem::{COPY_FILE_FAIL_IF_EXISTS, COPY_FILE_NO_BUFFERING, CopyFileExW};
    unsafe extern "system" fn cb(_total: i64, done: i64, _st: i64, _sd: i64, _id: u32, _r: u32, _s: *mut core::ffi::c_void, _d: *mut core::ffi::c_void, data: *const core::ffi::c_void) -> u32 {
        let (prog, last) = unsafe { &mut *(data as *mut (&Progress, u64)) };
        prog.bytes.fetch_add(done as u64 - *last, Relaxed);
        *last = done as u64;
        0 // PROGRESS_CONTINUE
    }
    let w = |p: &Path| -> Vec<u16> { std::os::windows::ffi::OsStrExt::encode_wide(p.as_os_str()).chain([0]).collect() };
    let mut state: (&Progress, u64) = (p, 0);
    let flags = COPY_FILE_FAIL_IF_EXISTS | if size >= BIG { COPY_FILE_NO_BUFFERING } else { 0 };
    let ok = unsafe { CopyFileExW(w(src).as_ptr(), w(dst).as_ptr(), Some(cb), &mut state as *mut _ as _, std::ptr::null_mut(), flags) } != 0;
    p.bytes.fetch_add(size.saturating_sub(state.1), Relaxed); // empty files and anything the callback did not report
    if ok { Ok(()) } else { Err(std::io::Error::last_os_error()) }
}

/// Copies `from` into `to` (names that exist there get " (2)"); `then_delete` = a move across drives: the sources go away
/// only when every file arrived.
pub fn copy(h: isize, from: Vec<PathBuf>, to: PathBuf, then_delete: bool) -> bool {
    let what = if then_delete { t!("Перемещение", "Moving", pt: "Movendo") } else { t!("Копирование", "Copying", pt: "Copiando") }.to_string();
    spawn(h, what, move |p| copy_now(&from, &to, then_delete, p))
}

fn copy_now(from: &[PathBuf], to: &Path, then_delete: bool, p: &Progress) -> Result<(), String> {
    let mut entries = vec![];
    for src in from {
        let name = src.file_name().ok_or_else(|| tf!("{}: нет имени", "{}: no name", pt: "{}: sem nome", src.display()))?.to_string_lossy().into_owned();
        let top = free_name(to, &name, src.is_dir());
        walk(src, PathBuf::from(top.file_name().unwrap()), &mut entries).map_err(|e| format!("{}: {e}", src.display()))?;
    }
    p.total_files.store(entries.iter().filter(|e| e.2.is_some()).count() as u64, Relaxed);
    p.total_bytes.store(entries.iter().filter_map(|e| e.2).sum(), Relaxed);
    for (_, rel, size) in &entries {
        if size.is_none() {
            fs::create_dir_all(to.join(rel)).map_err(|e| format!("{}: {e}", to.join(rel).display()))?;
        }
    }
    let mut files: Vec<&Entry> = entries.iter().filter(|e| e.2.is_some()).collect();
    files.sort_by_key(|e| std::cmp::Reverse(e.2)); // big ones first: the small ones fill the gaps at the end
    let threads = if seek_penalty(&from[0]) || seek_penalty(to) { 1 } else { std::thread::available_parallelism().map_or(4, |n| n.get()).clamp(2, 8) };
    let next = AtomicU64::new(0);
    let errors: Mutex<Vec<String>> = Mutex::new(vec![]);
    std::thread::scope(|s| {
        for _ in 0..threads {
            s.spawn(|| {
                while let Some(e) = files.get(next.fetch_add(1, Relaxed) as usize) {
                    let dst = to.join(&e.1);
                    match copy_file(&e.0, &dst, e.2.unwrap(), p) {
                        Ok(()) => _ = p.files.fetch_add(1, Relaxed),
                        Err(err) => errors.lock().unwrap().push(format!("{}: {err}", e.0.display())),
                    }
                }
            });
        }
    });
    let errors = errors.into_inner().unwrap();
    if !errors.is_empty() {
        let more = if errors.len() > 5 { tf!("\n... и ещё {}", "\n... and {} more", pt: "\n... e mais {}", errors.len() - 5) } else { String::new() };
        let kept = if then_delete { t!("\nИз источника ничего не удалено.", "\nNothing was deleted from the source.", pt: "\nNada foi apagado da origem.") } else { "" };
        return Err(tf!("Не скопировано файлов: {}\n{}{}{}", "{} file(s) were not copied:\n{}{}{}", pt: "{} arquivo(s) não foram copiados:\n{}{}{}", errors.len(), errors[..errors.len().min(5)].join("\n"), more, kept));
    }
    if then_delete {
        for src in from {
            let r = if fs::symlink_metadata(src).is_ok_and(|m| m.is_dir()) { fs::remove_dir_all(src) } else { fs::remove_file(src) };
            r.map_err(|e| tf!("Скопировано, но {} не удалось удалить: {}", "Copied, but {} could not be removed: {}", pt: "Copiado, mas não foi possível remover {}: {}", src.display(), e))?;
        }
    }
    Ok(())
}

// ---------- ZIP / 7z ----------

/// Files at most this big are compressed on worker threads in memory; bigger ones are streamed by the writer itself.
const PAR_MAX: u64 = 64 << 20;

pub fn zip(h: isize, from: Vec<PathBuf>, dest: PathBuf) -> bool {
    let what = tf!("Упаковка {}", "Packing {}", pt: "Compactando {}", dest.file_name().unwrap_or_default().to_string_lossy());
    spawn(h, what, move |p| zip_now(&from, &dest, p).inspect_err(|_| _ = fs::remove_file(&dest)).or_else(|e| if p.cancelled() { Ok(()) } else { Err(e) }))
}

/// Archive name of an entry: forward slashes, as ZIP wants.
fn zip_name(rel: &Path) -> String {
    rel.components().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/")
}

fn zip_now(from: &[PathBuf], dest: &Path, p: &Progress) -> Result<(), String> {
    use crate::zipx::{self, Prepared, Writer};
    let mut entries = vec![];
    for src in from {
        let name = src.file_name().ok_or_else(|| tf!("{}: нет имени", "{}: no name", pt: "{}: sem nome", src.display()))?;
        walk(src, PathBuf::from(name), &mut entries).map_err(|e| format!("{}: {e}", src.display()))?;
    }
    entries.retain(|e| e.0 != dest); // packing the folder the archive is written into
    p.total_files.store(entries.iter().filter(|e| e.2.is_some()).count() as u64, Relaxed);
    p.total_bytes.store(entries.iter().filter_map(|e| e.2).sum(), Relaxed);
    let file = fs::File::create(dest).map_err(|e| format!("{}: {e}", dest.display()))?;
    let mut w = Writer::new(std::io::BufWriter::with_capacity(1 << 20, file));
    let err = |e: &dyn std::fmt::Display, path: &Path| format!("{}: {e}", path.display());
    for (src, rel, size) in &entries {
        if size.is_none() {
            w.add_dir(&zip_name(rel), zipx::dos_time(src)).map_err(|e| err(&e, src))?;
        }
    }
    let small: Vec<&Entry> = entries.iter().filter(|e| e.2.is_some_and(|s| s <= PAR_MAX)).collect();
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    let next = AtomicU64::new(0);
    let (tx, rx) = std::sync::mpsc::sync_channel::<Result<Prepared, String>>(threads * 2); // bounded: memory stays small
    let written: Result<(), String> = std::thread::scope(|s| {
        for _ in 0..threads {
            let tx = tx.clone();
            let (small, next) = (&small, &next);
            s.spawn(move || {
                while let Some(e) = small.get(next.fetch_add(1, Relaxed) as usize) {
                    let r = zipx::prepare(zip_name(&e.1), &e.0).map_err(|x| format!("{}: {x}", e.0.display()));
                    if tx.send(r).is_err() {
                        break; // the writer gave up
                    }
                }
            });
        }
        drop(tx);
        for r in rx {
            if p.cancelled() {
                return Err(t!("Отменено", "Cancelled", pt: "Cancelado").to_string());
            }
            let f = r?;
            let n = f.uncompressed_size();
            w.add_prepared(f).map_err(|e| e.to_string())?;
            p.bytes.fetch_add(n, Relaxed);
            p.files.fetch_add(1, Relaxed);
        }
        Ok(())
    });
    written?;
    for (src, rel, _) in entries.iter().filter(|e| e.2.is_some_and(|s| s > PAR_MAX)) {
        w.add_stream(zip_name(rel), src, |n| { p.bytes.fetch_add(n, Relaxed); !p.cancelled() }).map_err(|e| err(&e, src))?;
        p.files.fetch_add(1, Relaxed);
    }
    w.finish().map_err(|e| err(&e, dest))?;
    Ok(())
}

pub fn is_archive(p: &Path) -> bool {
    p.extension().is_some_and(|e| e.eq_ignore_ascii_case("zip") || e.eq_ignore_ascii_case("7z"))
}

/// Unpacks each archive into a new folder named after it, next to it.
pub fn extract(h: isize, archives: Vec<PathBuf>, dir: PathBuf) -> bool {
    let what = tf!("Распаковка {}", "Extracting {}", pt: "Extraindo {}", archives.iter().map(|a| a.file_name().unwrap_or_default().to_string_lossy()).collect::<Vec<_>>().join(", "));
    spawn(h, what, move |p| {
        p.total_files.store(archives.len() as u64, Relaxed);
        for a in &archives {
            let stem = a.file_stem().unwrap_or_default().to_string_lossy().into_owned();
            let out = free_name(&dir, &stem, true);
            let r = extract_with(a, &out, p);
            if r.is_err() && p.cancelled() {
                let _ = fs::remove_dir_all(&out); // the folder is the one just made for this archive
                return Ok(());
            }
            r.map_err(|e| format!("{}: {e}", a.display()))?;
            p.files.fetch_add(1, Relaxed);
        }
        Ok(())
    })
}

/// Entry names that would land outside `out` are refused (zip-slip).
#[cfg(test)]
fn extract_one(archive: &Path, out: &Path) -> Result<(), String> {
    extract_with(archive, out, &Progress::new(String::new()))
}

fn extract_with(archive: &Path, out: &Path, p: &Progress) -> Result<(), String> {
    let is7z = archive.extension().is_some_and(|e| e.eq_ignore_ascii_case("7z"));
    if is7z {
        return sevenz_rust2::decompress_file(archive, out).map_err(|e| e.to_string());
    }
    crate::zipx::extract(archive, out, |t| _ = p.total_bytes.fetch_add(t, Relaxed), |n| { p.bytes.fetch_add(n, Relaxed); !p.cancelled() })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("rx-jobs-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn tree(root: &Path) {
        fs::create_dir_all(root.join("src/a/b")).unwrap();
        fs::write(root.join("src/one.txt"), "hello").unwrap();
        fs::write(root.join("src/a/two.bin"), vec![7u8; 300_000]).unwrap();
        fs::write(root.join("src/a/b/empty"), "").unwrap();
    }

    #[test]
    fn free_names() {
        let d = tmp("names");
        fs::write(d.join("x.txt"), "").unwrap();
        fs::create_dir(d.join("dir.v2")).unwrap();
        assert_eq!(free_name(&d, "x.txt", false), d.join("x (2).txt"));
        assert_eq!(free_name(&d, "y.txt", false), d.join("y.txt"));
        assert_eq!(free_name(&d, "dir.v2", true), d.join("dir.v2 (2)"));
        assert_eq!(free_name(&d, ".gitignore", false), d.join(".gitignore"));
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn copies_a_tree_and_moves_it() {
        let d = tmp("copy");
        tree(&d);
        fs::create_dir(d.join("dst")).unwrap();
        let p = Progress::new(String::new());
        copy_now(&[d.join("src")], &d.join("dst"), false, &p).unwrap();
        assert_eq!(fs::read_to_string(d.join("dst/src/one.txt")).unwrap(), "hello");
        assert_eq!(fs::read(d.join("dst/src/a/two.bin")).unwrap().len(), 300_000);
        assert!(d.join("dst/src/a/b/empty").is_file());
        assert_eq!((p.files.load(Relaxed), p.total_files.load(Relaxed)), (3, 3));
        assert_eq!(p.bytes.load(Relaxed), p.total_bytes.load(Relaxed));
        // again: the existing name gets " (2)", and as a move the source goes away
        copy_now(&[d.join("src")], &d.join("dst"), true, &Progress::new(String::new())).unwrap();
        assert!(d.join("dst/src (2)/a/two.bin").is_file() && !d.join("src").exists());
        assert!(!can_fast_copy(&[d.join("dst")], &d.join("dst/src")), "into itself");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn zip_round_trip() {
        let d = tmp("zip");
        tree(&d);
        let z = d.join("out.zip");
        zip_now(&[d.join("src")], &z, &Progress::new(String::new())).unwrap();
        extract_one(&z, &d.join("back")).unwrap();
        assert_eq!(fs::read_to_string(d.join("back/src/one.txt")).unwrap(), "hello");
        assert_eq!(fs::read(d.join("back/src/a/two.bin")).unwrap(), vec![7u8; 300_000]);
        assert!(d.join("back/src/a/b/empty").is_file());
        assert!(extract_one(&d.join("src/one.txt"), &d.join("bad")).is_err(), "not an archive");
        let _ = fs::remove_dir_all(&d);
    }

    /// Our ZIP against an independent implementation, both ways, and against Windows' own tar.
    #[test]
    fn zip_agrees_with_other_implementations() {
        use std::io::{Read, Write};
        let d = tmp("xzip");
        tree(&d);
        let z = d.join("ours.zip");
        zip_now(&[d.join("src")], &z, &Progress::new(String::new())).unwrap();
        let mut a = zip::ZipArchive::new(fs::File::open(&z).unwrap()).unwrap();
        let mut two = vec![];
        a.by_name("src/a/two.bin").unwrap().read_to_end(&mut two).unwrap(); // the crate checks the CRC itself
        assert_eq!(two, vec![7u8; 300_000]);
        assert!(a.by_name("src/a/b/").is_ok() && a.by_name("src/one.txt").is_ok());
        let t = std::process::Command::new("tar").arg("-xf").arg(&z).arg("-C").arg(&d).current_dir(&d).status();
        if let Ok(t) = t {
            assert!(t.success(), "tar could not read our zip");
            assert_eq!(fs::read(d.join("src/a/two.bin")).unwrap().len(), 300_000);
        }
        // theirs, with deflate and with stored entries
        let theirs = d.join("theirs.zip");
        let mut w = zip::ZipWriter::new(fs::File::create(&theirs).unwrap());
        let (def, sto) = (zip::write::SimpleFileOptions::default(), zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored));
        w.start_file("a/x.txt", def).unwrap();
        w.write_all(&b"hello hello hello ".repeat(5000)).unwrap();
        w.start_file("raw.bin", sto).unwrap();
        w.write_all(&[1, 2, 3]).unwrap();
        w.add_directory("empty/", def).unwrap();
        w.finish().unwrap();
        extract_one(&theirs, &d.join("theirs")).unwrap();
        assert_eq!(fs::read(d.join("theirs/a/x.txt")).unwrap().len(), 90_000);
        assert_eq!(fs::read(d.join("theirs/raw.bin")).unwrap(), [1, 2, 3]);
        assert!(d.join("theirs/empty").is_dir());
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn unpacks_7z() {
        let d = tmp("7z");
        tree(&d);
        let a = d.join("t.7z");
        sevenz_rust2::compress_to_path(d.join("src"), &a).unwrap();
        extract_one(&a, &d.join("back")).unwrap();
        let found: Vec<PathBuf> = {
            let mut v = vec![];
            walk(&d.join("back"), PathBuf::new(), &mut v).unwrap();
            v.into_iter().filter(|e| e.2.is_some()).map(|e| e.1).collect()
        };
        assert_eq!(found.len(), 3, "{found:?}");
        let two = found.iter().find(|p| p.ends_with("two.bin")).unwrap();
        assert_eq!(fs::read(d.join("back").join(two)).unwrap(), vec![7u8; 300_000]);
        let _ = fs::remove_dir_all(&d);
    }

    /// By hand, on real data: `set RX_LIVE=<folder>` then `cargo test -p rust-explorer --release live_zip -- --ignored --nocapture`.
    /// Packs the folder with our engine, unpacks it with Windows' tar, PowerShell's Expand-Archive and our own, and compares every file.
    #[test]
    #[ignore]
    fn live_zip() {
        let Some(src) = std::env::var_os("RX_LIVE").map(PathBuf::from) else { return };
        let d = tmp("live");
        let z = d.join("live.zip");
        let t = Instant::now();
        zip_now(&[src.clone()], &z, &Progress::new(String::new())).unwrap();
        let mut files = vec![];
        walk(&src, PathBuf::new(), &mut files).unwrap();
        let raw: u64 = files.iter().filter_map(|e| e.2).sum();
        println!("LIVE packed {} files, {} -> {} bytes in {:?}", files.iter().filter(|e| e.2.is_some()).count(), raw, fs::metadata(&z).unwrap().len(), t.elapsed());
        let name = src.file_name().unwrap();
        let (a, b, c) = (d.join("tar"), d.join("ps"), d.join("ours"));
        for o in [&a, &b] {
            fs::create_dir_all(o).unwrap();
        }
        let t = Instant::now();
        assert!(std::process::Command::new("tar").arg("-xf").arg(&z).arg("-C").arg(&a).status().unwrap().success());
        println!("LIVE tar unpacked in {:?}", t.elapsed());
        let t = Instant::now();
        let ps = std::process::Command::new("powershell").args(["-NoProfile", "-Command", &format!("Expand-Archive -LiteralPath '{}' -DestinationPath '{}' -Force", z.display(), b.display())]).status().unwrap();
        println!("LIVE Expand-Archive {} in {:?}", ps.success(), t.elapsed());
        assert!(ps.success());
        let t = Instant::now();
        extract_one(&z, &c).unwrap();
        println!("LIVE ours unpacked in {:?}", t.elapsed());
        let mut bad = 0;
        for (path, rel, size) in &files {
            if size.is_none() {
                continue;
            }
            let want = fs::read(path).unwrap();
            for root in [&a, &b, &c] {
                let got = fs::read(root.join(name).join(rel.strip_prefix(name).unwrap_or(rel)));
                let got = got.or_else(|_| fs::read(root.join(rel)));
                if got.as_deref().ok() != Some(&want[..]) {
                    bad += 1;
                    println!("LIVE MISMATCH {} in {}", rel.display(), root.display());
                }
            }
        }
        println!("LIVE compared {} files x 3 unpackers, {bad} mismatches", files.iter().filter(|e| e.2.is_some()).count());
        let _ = fs::remove_dir_all(&d);
        assert_eq!(bad, 0);
    }

    /// Old path (IFileOperation, Explorer's engine) against ours on the same data. By hand:
    /// `set RX_BENCH=<empty dir on the disk to test>` and `cargo test -p rust-explorer --release bench -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn bench() {
        let Some(root) = std::env::var_os("RX_BENCH").map(PathBuf::from) else { return };
        let small = root.join("small");
        fs::create_dir_all(&small).unwrap();
        for i in 0..5000 {
            fs::write(small.join(format!("f{i}.bin")), vec![(i % 251) as u8; 4096 + (i * 37) % 60_000]).unwrap();
        }
        let big = root.join("big");
        fs::create_dir_all(&big).unwrap();
        let chunk: Vec<u8> = (0..1u32 << 20).map(|i| (i.wrapping_mul(2654435761) >> 24) as u8).collect();
        let mut f = fs::File::create(big.join("1gb.bin")).unwrap();
        for _ in 0..1024 {
            std::io::Write::write_all(&mut f, &chunk).unwrap();
        }
        drop(f);
        super::super::sh::init();
        for src in [&small, &big] {
            let dst_old = root.join("out-old");
            let dst_new = root.join("out-new");
            let _ = fs::remove_dir_all(&dst_old);
            let _ = fs::remove_dir_all(&dst_new);
            fs::create_dir_all(&dst_old).unwrap();
            fs::create_dir_all(&dst_new).unwrap();
            let t = Instant::now();
            super::super::sh::fileop(std::ptr::null_mut(), super::super::sh::Op::Copy, &[src.clone()], &dst_old).unwrap();
            let old = t.elapsed();
            let t = Instant::now();
            copy_now(&[src.clone()], &dst_new, false, &Progress::new(String::new())).unwrap();
            let new = t.elapsed();
            println!("BENCH copy {}: IFileOperation {:.2} s, ours {:.2} s", src.file_name().unwrap().to_string_lossy(), old.as_secs_f64(), new.as_secs_f64());
        }
        let t = Instant::now();
        zip_now(&[small.clone()], &root.join("ours.zip"), &Progress::new(String::new())).unwrap();
        println!("BENCH zip small: ours {:.2} s", t.elapsed().as_secs_f64());
    }

    #[test]
    fn volumes_and_names() {
        assert!(same_volume(Path::new(r"C:\a"), Path::new(r"c:\b\c")));
        assert!(!same_volume(Path::new(r"C:\a"), Path::new(r"D:\a")));
        assert_eq!(zip_name(Path::new(r"a\b\c.txt")), "a/b/c.txt");
        assert!(is_archive(Path::new("x.ZIP")) && is_archive(Path::new("y.7z")) && !is_archive(Path::new("z.rar")));
    }
}
