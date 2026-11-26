// rs-shell              run as the user's shell: wallpaper, Run keys + Startup folders, topbar (restarted when it dies),
//                       explorer.exe as the fallback shell if topbar keeps dying
// rs-shell --enable     make it THIS user's shell (HKCU Winlogon\Shell; other users and the system are untouched)
// rs-shell --disable    back to the normal shell (deletes that one value)
// rs-shell --dry-run    print what it would start, change nothing
// rs-shell --overlay    run NEXT TO explorer.exe (it stays the shell): topbar + desktop + keep the bar alive. Started at logon by the "rs-overlay" scheduled task (HKCU Run as the fallback).
// Put it next to topbar.exe. Recovery from a black screen: Ctrl+Alt+Del -> Task Manager -> run explorer.exe,
// then `rs-shell --disable`; or sign in as another admin and run `reg delete` on the key printed by --enable.
// Also: starts `rust-explorer.exe --desktop` (desktop icons), Win+D = show desktop / bring the windows back (winrust-common),
// Win+E = file manager (rust-explorer.exe if next to us). Put rust-explorer.exe next to rs-shell.exe.
// shortcuts: Win+D only minimizes ordinary top-level windows (no per-monitor / virtual-desktop logic),
// Run/Startup entries are started through `cmd /c start` (no RunOnce, no startup-delay rules, no disabled-list in
// StartupApproved), topbar's "open" still shells out to explorer.exe for /select; upgrade when those matter.
use std::{os::windows::process::CommandExt, path::PathBuf, process::Command, ptr, thread, time::{Duration, Instant}};
use windows_sys::{core::BOOL, Win32::{Foundation::{HWND, LPARAM, RECT}, UI::{Input::KeyboardAndMouse::{MOD_NOREPEAT, MOD_WIN, RegisterHotKey}, WindowsAndMessaging::*}}};

const WINLOGON: &str = r"HKCU\Software\Microsoft\Windows NT\CurrentVersion\Winlogon";
const NO_WINDOW: u32 = 0x0800_0000;

/// A Command with null std handles. After FreeConsole() the shell has no valid ones, and spawn() inheriting
/// them fails with "The handle is invalid" (this broke every start the first time).
fn quiet(program: impl AsRef<std::ffi::OsStr>) -> Command {
    let mut c = Command::new(program);
    c.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    c
}

fn reg(args: &[&str]) -> Option<String> {
    let o = Command::new("reg").args(args).creation_flags(NO_WINDOW).output().ok()?;
    o.status.success().then(|| String::from_utf8_lossy(&o.stdout).into_owned())
}

/// Commands from `reg query <Run key>` output: "    Name    REG_SZ    command line".
fn parse_run(out: &str) -> Vec<String> {
    parse_run_named(out).into_iter().map(|(_, c)| c).collect()
}

/// (value name, command) pairs from `reg query <Run key>` output.
fn parse_run_named(out: &str) -> Vec<(String, String)> {
    out.lines().filter_map(|l| {
        let (n, v) = l.split_once("    REG_EXPAND_SZ").or_else(|| l.split_once("    REG_SZ"))?;
        Some((n.trim().to_string(), v.trim().to_string())).filter(|(_, v)| !v.is_empty())
    }).collect()
}

/// Names (lowercase) the user turned off in Task Manager's Startup tab: under StartupApproved each entry is a
/// REG_BINARY whose first byte is even when enabled (02, 06) and odd when disabled (03, 07). Explorer skips those.
fn parse_disabled(out: &str) -> std::collections::HashSet<String> {
    out.lines().filter_map(|l| {
        let (n, v) = l.split_once("    REG_BINARY")?;
        let first = u8::from_str_radix(v.trim().get(..2)?, 16).ok()?;
        (first & 1 == 1).then(|| n.trim().to_lowercase())
    }).collect()
}

const APPROVED: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved";

/// The exe a Run command line points at, %VARS% expanded; None when it is not an .exe we can tell apart (scripts, URIs).
fn exe_path(cmd: &str) -> Option<String> {
    let mut c = cmd.trim().to_string();
    while let Some(a) = c.find('%') {
        let b = c[a + 1..].find('%')?;
        let val = std::env::var(&c[a + 1..a + 1 + b]).ok()?;
        c = format!("{}{}{}", &c[..a], val, &c[a + b + 2..]);
    }
    let path = match c.strip_prefix('"') {
        Some(rest) => rest.split('"').next()?.to_string(),
        None => c[..c.to_lowercase().find(".exe")? + 4].to_string(),
    };
    path.to_lowercase().ends_with(".exe").then_some(path)
}

/// Everything the shell should start at logon (commands for `cmd /c start ""`).
fn autostart() -> Vec<String> {
    let mut v = vec![];
    let off = |key: String| reg(&["query", &key]).map(|o| parse_disabled(&o)).unwrap_or_default();
    // each Run key with the StartupApproved list Task Manager keeps for it; 32-bit programs register under WOW6432Node
    let runs = [
        (r"HKLM\Software\Microsoft\Windows\CurrentVersion\Run", format!(r"HKLM\{APPROVED}\Run")),
        (r"HKLM\Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Run", format!(r"HKLM\{APPROVED}\Run32")),
        (r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run", format!(r"HKCU\{APPROVED}\Run")),
    ];
    for (key, approved) in runs {
        let disabled = off(approved);
        v.extend(reg(&["query", key]).map(|o| parse_run_named(&o)).unwrap_or_default().into_iter().filter(|(n, _)| !disabled.contains(&n.to_lowercase())).map(|(_, c)| c));
    }
    let dirs = [
        (std::env::var("APPDATA").map(|a| PathBuf::from(a).join(r"Microsoft\Windows\Start Menu\Programs\Startup")), format!(r"HKCU\{APPROVED}\StartupFolder")),
        (std::env::var("ProgramData").map(|a| PathBuf::from(a).join(r"Microsoft\Windows\Start Menu\Programs\StartUp")), format!(r"HKLM\{APPROVED}\StartupFolder")),
    ];
    for (d, approved) in dirs {
        let Ok(d) = d else { continue };
        let disabled = off(approved); // Startup folder items are listed by file name
        for e in std::fs::read_dir(d).into_iter().flatten().flatten() {
            let name = e.file_name().to_string_lossy().to_lowercase();
            if name == "desktop.ini" || disabled.contains(&name) { continue; }
            v.push(format!("\"{}\"", e.path().display()));
        }
    }
    // the same program in the 64- and 32-bit keys starts once
    let mut seen = std::collections::HashSet::new();
    v.retain(|c| seen.insert(c.to_lowercase()));
    v.retain(|c| !c.to_lowercase().contains("rs-shell")); // never start ourselves twice
    // topbar's own helper (reshell) is started from our folder below, not from the Run entry that points into Program Files
    v.retain(|c| !c.to_lowercase().contains(r"\topbar\"));
    // a Run entry whose exe is gone is skipped silently by explorer; `cmd /c start` would pop an error dialog (Rave did)
    v.retain(|c| exe_path(c).is_none_or(|p| std::path::Path::new(&p).exists()));
    v
}

fn wide(s: &str) -> Vec<u16> { s.encode_utf16().chain([0]).collect() }

fn dir() -> PathBuf { std::env::current_exe().ok().and_then(|e| e.parent().map(|p| p.to_path_buf())).unwrap_or_default() }

/// An exe next to us, else the one the topbar plugin installed (the rust-explorer plugin keeps only its own files in its folder).
fn find_exe(name: &str) -> PathBuf {
    let own = dir().join(name);
    if own.exists() { return own; }
    PathBuf::from(std::env::var("ProgramFiles").unwrap_or_else(|_| r"C:\Program Files".into())).join("topbar").join(name)
}

fn bar_up() -> bool { unsafe { !FindWindowW(wide("topbar").as_ptr(), std::ptr::null()).is_null() } }

/// A minimized window left on screen (or minimized before the hide setting was applied) shows as a caption stub on
/// the desktop: park it where Windows itself parks hidden minimized windows.
unsafe extern "system" fn park(h: HWND, _: LPARAM) -> BOOL {
    unsafe {
        if IsWindowVisible(h) != 0 && IsIconic(h) != 0 {
            let mut r: RECT = std::mem::zeroed();
            GetWindowRect(h, &mut r);
            if r.left > -30000 { SetWindowPos(h, ptr::null_mut(), -32000, -32000, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE); }
        }
        1
    }
}

/// Makes %LOCALAPPDATA%\rs-shell\wallpaper.bmp (what the rust-explorer desktop paints) and returns the file to show.
/// The conversion lives in winrust_common::wallpaper: the desktop also remakes it when the wallpaper changes.
fn wallpaper_bmp() -> Option<PathBuf> {
    winrust_common::wallpaper::sync().0
}

/// Shell mode only: without explorer's transcoder the system desktop paints only BMP. Next to Explorer (`--overlay`) the
/// system wallpaper is never touched: Explorer shows the original itself.
fn set_wallpaper() {
    let Some(path) = wallpaper_bmp() else { return };
    unsafe { SystemParametersInfoW(0x14, 0, wide(&path.to_string_lossy()).as_ptr() as _, 0) }; // SPI_SETDESKWALLPAPER
}

/// Appends a line to %LOCALAPPDATA%\rs-shell\log.txt: a shell has no console, so this is the only way to see what it did.
fn log(msg: &str) {
    use std::io::Write;
    let Ok(base) = std::env::var("LOCALAPPDATA") else { return };
    let dir = PathBuf::from(base).join("rs-shell");
    let _ = std::fs::create_dir_all(&dir);
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(dir.join("log.txt")) { let _ = writeln!(f, "[{t}] {msg}"); }
}

fn run_shell() {
    unsafe { windows_sys::Win32::System::Console::FreeConsole() };
    log("start");
    // without explorer a minimized window is drawn as a little caption stub on the desktop (apps started with
    // --minimized left grey rectangles over the icons): hide them, the bar lists the windows anyway
    let mut mm = MINIMIZEDMETRICS { cbSize: size_of::<MINIMIZEDMETRICS>() as u32, iWidth: 0, iHorzGap: 0, iVertGap: 0, iArrange: ARW_HIDE as i32 };
    unsafe { SystemParametersInfoW(SPI_SETMINIMIZEDMETRICS, mm.cbSize, &mut mm as *mut _ as _, 0) };
    // the desktop is blank without explorer: repaint the user's wallpaper (again below, once the session has settled)
    thread::spawn(set_wallpaper); // the first conversion takes a second or two; do not hold up autostart
    let list = autostart();
    log(&format!("autostart: {} entries", list.len()));
    for c in list {
        let r = quiet("cmd").raw_arg(format!("/c start \"\" {c}")).creation_flags(NO_WINDOW).spawn();
        log(&format!("start {c} -> {:?}", r.map(|_| ())));
    }
    thread::sleep(Duration::from_secs(4)); // topbar may already have been started by its own Run entry
    set_wallpaper();
    // no bar yet (its logon task is gone in user mode): start it and its hotkey helper from our folder, with our normal rights
    if !bar_up() { log(&format!("topbar spawn -> {:?}", quiet(find_exe("topbar.exe")).spawn().map(|_| ()))); }
    let hotkeys = find_exe("reshell.exe"); // lives in topbar's folder, not ours
    if hotkeys.exists() { log(&format!("reshell spawn -> {:?}", quiet(&hotkeys).spawn().map(|_| ()))); }
    let desktop = dir().join("rust-explorer.exe");
    // the rust-explorer desktop: class "Progman" (so Wallpaper Engine finds it), title "rs-desktop" (explorer's is "Program Manager")
    let desktop_up = || unsafe { !FindWindowW(wide("Progman").as_ptr(), wide("rs-desktop").as_ptr()).is_null() };
    log(&format!("desktop exe {} exists={} up={}", desktop.display(), desktop.exists(), desktop_up()));
    if desktop.exists() && !desktop_up() { log(&format!("desktop spawn -> {:?}", quiet(&desktop).arg("--desktop").spawn().map(|_| ()))); }
    // the supervisor runs on its own thread; this one owns the hotkeys (RegisterHotKey needs the thread's message loop)
    thread::spawn(supervise);
    hotkey_loop();
}

/// Win+D (show desktop) and Win+E (file manager) for sessions where Explorer is not the shell. Never returns.
fn hotkey_loop() {
    unsafe {
        RegisterHotKey(ptr::null_mut(), 1, MOD_WIN | MOD_NOREPEAT, 'D' as u32);
        RegisterHotKey(ptr::null_mut(), 2, MOD_WIN | MOD_NOREPEAT, 'E' as u32);
        let mut m: MSG = std::mem::zeroed();
        while GetMessageW(&mut m, ptr::null_mut(), 0, 0) > 0 {
            match (m.message == WM_HOTKEY).then_some(m.wParam) {
                Some(1) => winrust_common::desktop::toggle(),
                Some(2) => { let fm = dir().join("rust-explorer.exe"); let _ = quiet(if fm.exists() { fm } else { "explorer.exe".into() }).spawn(); }
                _ => {}
            }
        }
    }
}

/// Keeps topbar alive; if it will not stay up, hands the session to explorer.exe and ends the shell.
fn supervise() {
    let (mut starts, mut since, mut misses) = (0, Instant::now(), 0);
    loop {
        unsafe { EnumWindows(Some(park), 0) };
        if bar_up() { misses = 0; } else { misses += 1; }
        if misses >= 6 { // ~6 s: the loop ticks every second now
            if since.elapsed() > Duration::from_secs(120) { (starts, since) = (0, Instant::now()); }
            starts += 1;
            if starts > 3 || quiet(find_exe("topbar.exe")).spawn().is_err() {
                // topbar will not stay up: hand the session to explorer so the user is never left with nothing
                let _ = quiet(find_exe("topbar.exe")).arg("--restore").creation_flags(NO_WINDOW).status();
                let _ = quiet("explorer.exe").spawn();
                std::process::exit(0);
            }
            misses = 0;
        }
        thread::sleep(Duration::from_secs(1));
    }
}

/// `rs-shell --overlay`: runs next to explorer.exe, which stays the real shell (Win+D / Win+E / wallpaper / Run keys are
/// its job). Starts topbar and the rust-explorer desktop on top of it and keeps the bar alive. Started from the user's
/// Run key, so it can live in a folder the user owns (Windows stopped starting our own shell at logon).
/// The rust-explorer plugin's "replace Explorer" switch (HKCU\Software\rs-shell\ReplaceExplorer = 1).
fn replace_on() -> bool {
    winrust_common::reg::get_dword(r"HKCU\Software\rs-shell", "ReplaceExplorer") == Some(1)
}

/// Ends the Explorer that runs as the shell (started without arguments). Folder windows (started with a path) and the COM
/// host (`/factory ... -Embedding`) are other explorer.exe processes and keep running.
fn kill_shell_explorer() {
    winrust_common::proc::kill_shell_explorers();
    log("explorer shell stopped");
}

/// How long after the overlay starts Explorer may be ended even without the markers below: newer Windows builds (26300)
/// no longer write them, and the overlay then waited forever. Explorer gets through the autostart well within this.
const AUTOSTART_GRACE: Duration = Duration::from_secs(90);

/// How long Explorer must have started no new program before it may go. It starts the autostart one program at a time,
/// about a second apart (up to 4 s between the Run keys and the Startup folder on a Windows 10 PC).
const AUTOSTART_QUIET: Duration = Duration::from_secs(8);

/// True once Explorer has begun this session's Run keys and Startup folder. The overlay starts at logon (a scheduled task),
/// before Explorer gets to them: ending Explorer earlier would lose the user's autostart apps. Explorer writes each marker
/// just before its pass, not after it (StartupHasBeenRun 30 ms before the first Startup folder app), so a marker alone
/// does not mean the programs are running: `may_end_explorer` also waits until Explorer stops starting new ones.
fn startup_run() -> bool {
    let mut sid = 0;
    unsafe { windows_sys::Win32::System::RemoteDesktop::ProcessIdToSessionId(std::process::id(), &mut sid) };
    let k = format!(r"HKCU\Software\Microsoft\Windows\CurrentVersion\Explorer\SessionInfo\{sid}");
    // read in place: this runs every 3 s during logon (it started two reg.exe processes each time)
    ["RunStuffHasBeenRun", "StartupHasBeenRun"].iter().all(|n| winrust_common::reg::exists(&format!(r"{k}\{n}"), None))
}

/// What Explorer's autostart should leave running: the exe names (lowercase) of the entries we can recognise, and how many
/// we cannot (Startup folder shortcuts, scripts, URIs).
fn expected_autostart(list: &[String]) -> (Vec<String>, usize) {
    let exes: Vec<String> = list
        .iter()
        .filter_map(|c| {
            // Squirrel apps (Discord, Slack...) autostart as `Update.exe --processStart "App.exe"`: Update.exe quits at once,
            // the app is what keeps running
            let lc = c.to_lowercase();
            if let Some(i) = lc.find("--processstart") {
                let app = c[i + "--processstart".len()..].trim_start().trim_start_matches('"');
                return app.split(['"', ' ']).next().filter(|a| a.to_lowercase().ends_with(".exe")).map(str::to_lowercase);
            }
            exe_path(c).and_then(|p| std::path::Path::new(&p).file_name().map(|n| n.to_string_lossy().to_lowercase()))
        })
        .collect();
    let untracked = list.len() - exes.len();
    (exes, untracked)
}

/// The expected exes that are not running yet.
fn missing_autostart<'a>(expected: &'a [String], running: &[(u32, String)]) -> Vec<&'a str> {
    expected.iter().filter(|e| !running.iter().any(|(_, n)| n.eq_ignore_ascii_case(e))).map(String::as_str).collect()
}

/// Explorer may go once it has started the autostart: its markers say so (older builds), or every program we expect is
/// running and Explorer had its startup delay (~10 s; 30 s when there are entries we cannot watch, such as Startup
/// folder shortcuts, which it runs in the same pass), or the grace time ran out (an app that starts and quits at once).
/// In every case Explorer must have started nothing new for `quiet`: ending it in the middle of a pass loses the rest (#159).
fn may_end_explorer(markers: bool, since_start: Duration, all_running: bool, untracked: usize, quiet: Duration) -> bool {
    let settle = Duration::from_secs(if untracked > 0 { 30 } else { 15 });
    quiet >= AUTOSTART_QUIET && (markers || since_start >= AUTOSTART_GRACE || (all_running && since_start >= settle))
}

/// When Explorer last started a program: `kids` are its children now, `seen` the ones of the last look. Returns the
/// new `last` (now if a child is new) and keeps `kids` as the next `seen`.
fn last_start(seen: &mut Vec<u32>, kids: Vec<u32>, last: Instant, now: Instant) -> Instant {
    let new = kids.iter().any(|k| !seen.contains(k));
    *seen = kids;
    if new { now } else { last }
}

/// Gives the user Explorer back (with a normal, not elevated, token: the shell must never run elevated).
fn start_shell_explorer() {
    let r = winrust_common::proc::start_shell_explorer();
    log(&format!("explorer shell started -> {:?}", r.map(|_| ())));
}

fn overlay() {
    unsafe { windows_sys::Win32::System::Console::FreeConsole() };
    let replace = replace_on();
    log(&format!("overlay start (replace explorer: {replace})"));
    let wp = thread::spawn(wallpaper_bmp); // the BMP the desktop window paints; the system wallpaper stays Explorer's
    if !bar_up() { log(&format!("topbar spawn -> {:?}", quiet(find_exe("topbar.exe")).spawn().map(|_| ()))); }
    // Win+I / Win+E / Win+D live in reshell: its Run entry may be missing (another account's HKCU), and without it Explorer's own Win+I
    // fails once the shell is gone. A second copy exits at once (one hook per session).
    let hk = find_exe("reshell.exe");
    if hk.exists() { log(&format!("reshell spawn -> {:?}", quiet(&hk).spawn().map(|_| ()))); }
    let _ = wp.join(); // the BMP conversion (only on a fresh install or a new wallpaper) finishes before the desktop paints it
    let desktop = dir().join("rust-explorer.exe");
    let up = || unsafe { !FindWindowW(wide("Progman").as_ptr(), wide("rs-desktop").as_ptr()).is_null() };
    if desktop.exists() && !up() { log(&format!("desktop spawn -> {:?}", quiet(&desktop).arg("--desktop").spawn().map(|_| ()))); }
    if replace {
        // Explorer is being replaced: this thread watches, the main one owns Win+D / Win+E
        thread::spawn(move || overlay_watch(replace, desktop, up));
        hotkey_loop();
    } else {
        overlay_watch(replace, desktop, up);
    }
}

/// Keeps topbar and the desktop alive. With `replace` it also ends the shell Explorer once both are up, and gives it back
/// (so the user is never left without a shell) if the bar will not stay up.
fn overlay_watch(replace: bool, desktop: PathBuf, up: impl Fn() -> bool) {
    let (mut starts, mut misses, mut dmiss, mut stable, mut killed) = (0, 0, 0, 0, false);
    let began = Instant::now();
    // newer builds write no markers: watch for the autostart programs themselves instead of always waiting the full grace
    let (expected, untracked) = if replace { expected_autostart(&autostart()) } else { (vec![], 0) };
    if replace { log(&format!("autostart to wait for: {} exe(s) {:?}, {} other entr(ies)", expected.len(), expected, untracked)); }
    // what the shell Explorer has started so far, and when it last started something (Startup folder shortcuts and Store
    // apps' startup tasks are its children too, even though we cannot name them in advance)
    let (mut kids, mut last) = (vec![], began);
    loop {
        thread::sleep(Duration::from_secs(3));
        if replace && !killed {
            // both our surfaces must be on screen before the system's one goes away
            stable = if bar_up() && up() { stable + 1 } else { 0 };
            let missing = missing_autostart(&expected, &winrust_common::proc::list());
            last = last_start(&mut kids, winrust_common::proc::children(&winrust_common::proc::shell_explorers()), last, Instant::now());
            if stable >= 3 && may_end_explorer(startup_run(), began.elapsed(), missing.is_empty(), untracked, last.elapsed()) {
                log(&format!("autostart done (markers: {}, after {} s, last start {} s ago, not running: {:?})", startup_run(), began.elapsed().as_secs(), last.elapsed().as_secs(), missing));
                kill_shell_explorer();
                killed = true;
            }
        }
        // after we ended Explorer, a shell Explorer is back only because Settings needs it (common/src/settings.rs ends our
        // desktop for that and brings it back when Settings closes): a desktop raised under it leaves Store apps cloaked
        // asked only when the desktop is missing: a process snapshot plus a PEB read per explorer.exe every 3 s was for nothing
        // the marker first: Settings sets it before it ends our desktop, so the gap until Explorer shows up is covered too
        let lent = || winrust_common::settings::explorer_lent() || (replace && killed && !winrust_common::proc::shell_explorers().is_empty());
        dmiss = if desktop.exists() && !up() && !lent() { dmiss + 1 } else { 0 };
        if dmiss >= 2 { log(&format!("desktop respawn -> {:?}", quiet(&desktop).arg("--desktop").spawn().map(|_| ()))); dmiss = 0; }
        if bar_up() { misses = 0; continue; }
        misses += 1;
        if misses < 2 { continue; }
        starts += 1;
        if starts > 3 || quiet(find_exe("topbar.exe")).spawn().is_err() {
            // the bar will not stay up: give the system's taskbar (and, if we ended it, Explorer) back and stop
            let _ = quiet(find_exe("topbar.exe")).arg("--restore").creation_flags(NO_WINDOW).status();
            if replace { start_shell_explorer(); }
            log("overlay: topbar keeps dying, system shell restored");
            return;
        }
        misses = 0;
    }
}

/// Windows will not start a shell from a folder the user can write to (a boot from %LOCALAPPDATA% fell back to explorer),
/// so the copy in Program Files is only a loader: it runs the real rs-shell from %LOCALAPPDATA%\plugin-launcher\bin, which
/// can be updated without admin. true = the user-folder shell took over (the loader must not run the shell logic as well).
/// A missing copy or one that dies within 3 s (it started nothing yet) leaves false: the loader then does the job itself.
fn bootstrap() -> bool {
    let Ok(base) = std::env::var("LOCALAPPDATA") else { return false };
    let real = PathBuf::from(base).join(r"plugin-launcher\bin\rs-shell.exe");
    let me = std::env::current_exe().and_then(|p| p.canonicalize()).ok();
    let age = |p: &PathBuf| p.metadata().and_then(|m| m.modified()).ok();
    // an older user copy is a leftover (nothing writes there any more): the installed shell runs itself
    if !real.exists() || real.canonicalize().ok() == me || age(&real) < std::env::current_exe().ok().as_ref().and_then(age) {
        return false;
    }
    let t = Instant::now();
    match quiet(&real).spawn() {
        Ok(mut child) => {
            log(&format!("loader: started {}", real.display()));
            let _ = child.wait();
            t.elapsed() > Duration::from_secs(3)
        }
        Err(e) => {
            log(&format!("loader: cannot start {}: {e}", real.display()));
            false
        }
    }
}

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let code = match a.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        [] => { if !bootstrap() { run_shell(); } 0 }
        ["--overlay"] => { overlay(); 0 }
        ["--dry-run"] => {
            println!("wallpaper: {:?}", reg(&["query", r"HKCU\Control Panel\Desktop", "/v", "WallPaper"]).map(|o| parse_run(&o)));
            autostart().iter().for_each(|c| println!("autostart: {c}"));
            println!("topbar: {} ({})", find_exe("topbar.exe").display(), if find_exe("topbar.exe").exists() { "found" } else { "MISSING" });
            println!("explorer ran this session's autostart: {}", startup_run());
            0
        }
        ["--enable"] => {
            let exe = std::env::current_exe().unwrap_or_default();
            if !find_exe("topbar.exe").exists() {
                eprintln!("refusing: topbar.exe is not next to {} (no bar = no way to launch anything)", exe.display());
                1
            } else if reg(&["add", WINLOGON, "/v", "Shell", "/t", "REG_SZ", "/d", &exe.to_string_lossy(), "/f"]).is_some() {
                println!("done: you get rs-shell instead of explorer at the next sign-in.\nundo: rs-shell --disable   (or: reg delete \"{WINLOGON}\" /v Shell /f)");
                0
            } else { eprintln!("reg add failed"); 1 }
        }
        ["--disable"] => {
            // deleting a value that is not there is fine too
            let _ = reg(&["delete", WINLOGON, "/v", "Shell", "/f"]);
            println!("done: normal shell at the next sign-in");
            0
        }
        _ => { eprintln!("usage: rs-shell [--enable | --disable | --dry-run]"); 2 }
    };
    std::process::exit(code);
}

#[cfg(test)]
mod tests {
    use super::{exe_path, may_end_explorer, parse_run};
    use std::time::Duration;
    #[test]
    fn ends_explorer_on_markers_or_after_grace() {
        let (s, q) = (Duration::from_secs, Duration::from_secs(8)); // q: Explorer has started nothing new for 8 s
        assert!(may_end_explorer(true, s(9), false, 0, q));
        assert!(!may_end_explorer(false, s(30), false, 0, q)); // something still missing
        assert!(may_end_explorer(false, s(90), false, 3, q)); // grace over, whatever is missing
        // everything we watch is running: after Explorer's startup delay, longer with shortcuts we cannot watch
        assert!(!may_end_explorer(false, s(9), true, 0, q));
        assert!(may_end_explorer(false, s(15), true, 0, q));
        assert!(!may_end_explorer(false, s(20), true, 1, q));
        assert!(may_end_explorer(false, s(30), true, 1, q));
    }
    #[test]
    fn waits_while_explorer_still_starts_programs() {
        // #159: the markers are written before Explorer starts the programs; ending it then lost the rest of the pass
        let recent = Duration::from_secs(2);
        assert!(!may_end_explorer(true, Duration::from_secs(20), true, 0, recent));
        assert!(!may_end_explorer(false, Duration::from_secs(90), false, 0, recent));
        assert!(!may_end_explorer(false, Duration::from_secs(30), true, 1, recent));
    }
    #[test]
    fn notices_each_program_explorer_starts() {
        let t0 = std::time::Instant::now();
        let (t1, t2, t3) = (t0 + Duration::from_secs(3), t0 + Duration::from_secs(6), t0 + Duration::from_secs(9));
        let mut seen = vec![];
        assert_eq!(super::last_start(&mut seen, vec![10, 11], t0, t1), t1); // the first look: everything is new
        assert_eq!(super::last_start(&mut seen, vec![10, 11], t1, t2), t1); // nothing new
        assert_eq!(super::last_start(&mut seen, vec![11], t1, t2), t1); // one quit: still nothing new
        assert_eq!(super::last_start(&mut seen, vec![11, 12], t1, t3), t3); // a new one
    }
    #[test]
    fn watches_the_autostart_exes() {
        let list = [r#""C:\a b\Steam.exe" -silent"#.to_string(), r"C:\d\y.exe /background".to_string(), r#""C:\Users\u\Startup\x.lnk""#.to_string()];
        let (exes, untracked) = super::expected_autostart(&list);
        assert_eq!((exes.clone(), untracked), (vec!["steam.exe".to_string(), "y.exe".to_string()], 1));
        let running = [(1, "STEAM.EXE".to_string()), (2, "explorer.exe".to_string())];
        assert_eq!(super::missing_autostart(&exes, &running), ["y.exe"]);
        assert!(super::missing_autostart(&exes, &[(1, "steam.exe".into()), (3, "y.exe".into())]).is_empty());
        // Squirrel: the app it starts, not Update.exe
        let sq = [r#"C:\Users\u\AppData\Local\Discord\Update.exe --processStart Discord.exe"#.to_string(), r#""C:\S\Update.exe" --processStart "slack.exe" --process-start-args "--startup""#.to_string()];
        assert_eq!(super::expected_autostart(&sq).0, ["discord.exe", "slack.exe"]);
    }
    #[test]
    fn finds_the_exe_in_run_commands() {
        assert_eq!(exe_path(r#""C:\a b\x.exe" --hidden"#).as_deref(), Some(r"C:\a b\x.exe"));
        assert_eq!(exe_path(r"C:\d\y.exe /background").as_deref(), Some(r"C:\d\y.exe"));
        assert_eq!(exe_path("script.bat"), None);
        assert!(exe_path(r"%windir%\system32\z.exe").unwrap().to_lowercase().ends_with(r"system32\z.exe"));
    }
    #[test]
    fn parses_reg_query_lines() {
        let out = "\nHKEY_CURRENT_USER\\X\n    Foo    REG_SZ    foo.exe --x\n    Bar    REG_EXPAND_SZ    %APPDATA%\\bar.exe\n    Empty    REG_SZ    \n";
        assert_eq!(parse_run(out), ["foo.exe --x", "%APPDATA%\\bar.exe"]);
        // a name with spaces stays whole
        assert_eq!(super::parse_run_named("    Steam Client    REG_SZ    \"C:\\s\\steam.exe\" -silent\n"), [("Steam Client".to_string(), "\"C:\\s\\steam.exe\" -silent".to_string())]);
    }
    #[test]
    fn reads_task_managers_disabled_entries() {
        let out = "\nHKEY_CURRENT_USER\\...\\StartupApproved\\Run\n    Discord    REG_BINARY    030000006A1B2C3D4E5F6071\n    OneDrive    REG_BINARY    020000000000000000000000\n    Spotify Web    REG_BINARY    07000000FFFFFFFF00000000\n    Odd    REG_BINARY    06\n    Junk    REG_BINARY    \n";
        let off = super::parse_disabled(out);
        assert!(off.contains("discord") && off.contains("spotify web"));
        assert!(!off.contains("onedrive") && !off.contains("odd") && !off.contains("junk"));
    }
}
