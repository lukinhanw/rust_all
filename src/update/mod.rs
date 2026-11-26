//! Automatic updates without a UAC prompt: a scheduled task runs `WinRust.exe autoupdate` as SYSTEM at boot and once a day.
//! It reads the latest release's signed manifest (manifest.rs), downloads only the exes whose hash differs from the
//! installed copy, checks each against the manifest and swaps it in. Nothing new is installed and no setting is changed:
//! only files that are already in Program Files get replaced.
//!
//! A running exe cannot be overwritten but can be renamed: the old one becomes `<name>.exe.old` (deleted on the next run)
//! and the new one is used from the program's next start (sign-in, reboot, or when rs-shell restarts it).
use crate::*;
mod manifest;
mod swap;
use manifest::{sha256_hex, verify, ver};
use swap::swap_all;

/// The owner's ed25519 public key (`winrust-sign keygen`). The private half never enters the repository.
const KEY: &str = include_str!("../../update-key.pub");
pub(crate) const BASE: &str = "https://github.com/Lemxy/WinRust/releases";
const TASK: &str = "WinRust-update";
/// The "Installed apps" entry: WinRust can be removed from Windows Settings even when the downloaded exe is gone (#178).
const ARP: &str = r"HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\WinRust";

fn home() -> PathBuf {
    program_files().join("WinRust")
}

fn self_exe() -> PathBuf {
    home().join("WinRust.exe")
}

/// Where a released file lives once installed (debloat.exe in two places). Names are the ones in payload.txt.
fn targets(name: &str) -> Vec<PathBuf> {
    let pf = program_files();
    match name {
        "WinRust.exe" => vec![self_exe()],
        "rs.exe" | "rs-launcher.exe" => vec![prog_dir().join(name)],
        "Task manager.exe" => vec![tm_dir().join("rtaskmgr-win32.exe")],
        "rs-du.exe" => vec![tm_dir().join(name)],
        "debloat.exe" => vec![tm_dir().join(name), db_dir().join(name)],
        "topbar.exe" | "reshell.exe" => vec![bar_dir().join(name)],
        "rust-explorer.exe" | "rs-shell.exe" => vec![rx_dir().join(name)],
        "wiztree.exe" | "photoview.exe" | "rust-settings.exe" | "purge.exe" => vec![pf.join(name.trim_end_matches(".exe")).join(name)],
        _ => vec![],
    }
}

/// curl.exe ships with Windows 10 1803+ and Windows 11 (schannel TLS, the system's certificates).
/// shortcut: no proxy from the user's settings (the task runs as SYSTEM); switch to WinHTTP if someone needs one.
fn fetch(url: &str, dst: &Path) -> Result<(), String> {
    let curl = PathBuf::from(env::var("SystemRoot").unwrap_or(r"C:\Windows".into())).join(r"System32\curl.exe");
    run(&curl.to_string_lossy(), &["-fsSL", "--retry", "2", "--max-time", "600", "-o", &dst.to_string_lossy(), url])
}

/// Leftovers of earlier swaps whose program has exited since.
fn sweep() {
    let names = PAYLOAD.iter().map(|(n, _)| *n).chain(["WinRust.exe"]);
    let dirs: Vec<PathBuf> = names.flat_map(targets).filter_map(|t| t.parent().map(Path::to_path_buf)).collect();
    for d in dirs {
        for e in fs::read_dir(&d).into_iter().flatten().flatten() {
            let n = e.file_name().to_string_lossy().to_lowercase();
            if n.contains(".exe.old") || n.ends_with(".exe.new") {
                let _ = fs::remove_file(e.path());
            }
        }
    }
}

/// `autoupdate`: one check. Each file is on its own: a failed download leaves the others updated and itself as it was.
pub(crate) fn autoupdate() -> Result<String, String> {
    sweep();
    let work = home().join("update"); // under Program Files: a normal user cannot swap a file between check and copy
    let _ = fs::remove_dir_all(&work);
    fs::create_dir_all(&work).map_err(|e| format!("{}: {e}", work.display()))?;
    let r = check(&work);
    let own = from_self(&work);
    let _ = fs::remove_dir_all(&work);
    for r in [&r, &own] {
        match r {
            Ok(m) if m.is_empty() => {}
            Ok(m) => log(&format!("autoupdate: {m}")),
            Err(e) => log(&tf!("autoupdate: ОШИБКА {e}", "autoupdate: ERROR {e}", ja: "自動更新: エラー {e}", pt: "autoupdate: ERRO {e}", zh: "autoupdate: 错误 {e}")),
        }
    }
    // a new WinRust.exe was swapped in: it brings its plugins now (from_self), not at the next daily run
    if r.as_ref().is_ok_and(|m| m.contains("WinRust.exe")) {
        let _ = std::process::Command::new(self_exe()).arg("autoupdate").spawn();
    }
    r.and(own)
}

/// A release may ship WinRust.exe alone (the plugins ride inside it). Once the auto-updater has put such a release in
/// (version.txt is this copy's own version), this copy replaces installed plugins that differ from the ones it carries,
/// once per version (plugins.txt). A plugin updated by hand afterwards (`update`, a dev build) is left alone.
fn from_self(work: &Path) -> Result<String, String> {
    let own = format!("v{}", env!("CARGO_PKG_VERSION"));
    let read = |f: &str| fs::read_to_string(home().join(f)).map(|s| s.trim().to_string()).unwrap_or_default();
    let me = env::current_exe().ok().and_then(|p| p.canonicalize().ok());
    if read("version.txt") != own || read("plugins.txt") == own || me != self_exe().canonicalize().ok() {
        return Ok(String::new());
    }
    let (mut done, mut errs) = (vec![], vec![]);
    for (name, bytes) in PAYLOAD.iter() {
        let sha = sha256_hex(bytes);
        let stale: Vec<PathBuf> = targets(name).into_iter().filter(|t| fs::read(t).is_ok_and(|b| sha256_hex(&b) != sha)).collect();
        if stale.is_empty() {
            continue;
        }
        let tmp = work.join(name);
        match fs::write(&tmp, bytes).map_err(|e| format!("{name}: {e}")).and_then(|_| swap_all(&tmp, &stale)) {
            Ok(()) => done.push(*name),
            Err(e) => errs.push(e),
        }
    }
    let msg = if done.is_empty() { String::new() } else { tf!("{own}: обновлено {}", "{own}: updated {}", ja: "{own}: {} を更新しました", pt: "{own}: atualizado {}", zh: "{own}：已更新 {}", done.join(", ")) };
    if errs.is_empty() {
        let _ = fs::write(home().join("plugins.txt"), &own); // failed files are retried on the next run
        Ok(msg)
    } else { Err(format!("{msg}; {}", errs.join("; "))) }
}

/// A newer signed release than this one, found by the installer window (None until found, or when up to date).
pub(crate) static NEWER: Mutex<Option<String>> = Mutex::new(None);

/// This build's version, or the installed one when the auto-updater has put a newer one in (`v0.5.0`).
pub(crate) fn current() -> &'static str {
    static V: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    V.get_or_init(|| {
        let own = format!("v{}", env!("CARGO_PKG_VERSION"));
        match fs::read_to_string(home().join("version.txt")).map(|s| s.trim().to_string()) {
            Ok(inst) if ver(&inst) > ver(&own) => inst,
            _ => own,
        }
    })
}

/// Asks GitHub for the latest signed manifest (only to compare versions, nothing is installed); remembers a newer one in `NEWER`.
pub(crate) fn look_for_update() -> Option<String> {
    let work = env::temp_dir().join(format!("winrust-vercheck-{}", std::process::id()));
    fs::create_dir_all(&work).ok()?;
    let r = (|| {
        let (mt, ms) = (work.join("manifest.txt"), work.join("manifest.sig"));
        fetch(&format!("{BASE}/latest/download/manifest.txt"), &mt).ok()?;
        fetch(&format!("{BASE}/latest/download/manifest.sig"), &ms).ok()?;
        let m = verify(&fs::read(&mt).ok()?, &fs::read_to_string(&ms).ok()?, KEY).ok()?;
        (ver(&m.version)? > ver(current())?).then_some(m.version)
    })();
    let _ = fs::remove_dir_all(&work);
    *NEWER.lock().unwrap() = r.clone();
    r
}

fn check(work: &Path) -> Result<String, String> {
    let (mt, ms) = (work.join("manifest.txt"), work.join("manifest.sig"));
    fetch(&format!("{BASE}/latest/download/manifest.txt"), &mt)?;
    fetch(&format!("{BASE}/latest/download/manifest.sig"), &ms)?;
    let text = fs::read(&mt).map_err(|e| e.to_string())?;
    let sig = fs::read_to_string(&ms).map_err(|e| e.to_string())?;
    let m = verify(&text, &sig, KEY)?;
    // Only a newer release replaces anything: an older signed manifest is a rollback attack, an equal one would
    // overwrite a local build of the same version.
    let new_v = ver(&m.version).ok_or("manifest version is not like v1.2.3")?;
    let vfile = home().join("version.txt");
    if let Some(cur) = fs::read_to_string(&vfile).ok().and_then(|s| ver(s.trim())) {
        if new_v < cur {
            return Err(tf!("{}: релиз старше установленного, отказ", "{}: older than the installed release, refused", ja: "{}: インストール済みのリリースより古いため拒否しました", pt: "{}: mais antiga que a versão instalada, recusada", zh: "{}：比已安装的版本旧，已拒绝", m.version));
        }
        if new_v == cur {
            return Ok(tf!("{}: всё актуально", "{}: up to date", ja: "{}: 最新です", pt: "{}: atualizado", zh: "{}：已是最新", m.version));
        }
    }
    let (mut done, mut errs) = (vec![], vec![]);
    for (sha, name) in &m.files {
        let stale: Vec<PathBuf> = targets(name).into_iter().filter(|t| fs::read(t).is_ok_and(|b| sha256_hex(&b) != *sha)).collect();
        if stale.is_empty() {
            continue; // not installed, or already this version
        }
        let tmp = work.join(name);
        // GitHub turns spaces in asset names into dots
        let got = fetch(&format!("{BASE}/download/{}/{}", m.version, name.replace(' ', ".")), &tmp).and_then(|_| {
            let b = fs::read(&tmp).map_err(|e| e.to_string())?;
            if sha256_hex(&b) == *sha { Ok(()) } else { Err(tf!("{name}: хеш не совпадает с манифестом", "{name}: hash does not match the manifest", ja: "{name}: ハッシュがマニフェストと一致しません", pt: "{name}: o hash não corresponde ao manifesto", zh: "{name}：哈希与清单不匹配")) }
        });
        match got.and_then(|_| swap_all(&tmp, &stale)) {
            Ok(()) => done.push(name.as_str()),
            Err(e) => errs.push(e),
        }
    }
    let msg = if done.is_empty() { tf!("{}: всё актуально", "{}: up to date", ja: "{}: 最新です", pt: "{}: atualizado", zh: "{}：已是最新", m.version) } else { tf!("{}: обновлено {}", "{}: updated {}", ja: "{}: {} を更新しました", pt: "{}: atualizado {}", zh: "{}：已更新 {}", m.version, done.join(", ")) };
    if errs.is_empty() {
        let _ = fs::write(&vfile, &m.version); // failed files are retried: the version is recorded only when all went through
        Ok(msg)
    } else { Err(format!("{msg}; {}", errs.join("; "))) }
}

/// After install / uninstall. This fork ships its own builds of the plugins, and the WinRust-update task would replace them
/// with the official releases (the signed manifest only knows those): the task is never created, and one left by an
/// official install is removed. `plugin-launcher autoupdate` still runs a manual check when asked for.
pub(crate) fn updater_sync() {
    let _ = run("schtasks", &["/delete", "/tn", TASK, "/f"]);
    if !PLUGINS.iter().any(|p| (p.installed)()) {
        let _ = run("reg", &["delete", ARP, "/f"]);
        let _ = fs::remove_dir_all(home()); // fails harmlessly if this very copy is running
    }
}

#[allow(dead_code)] // the official installer's way of turning automatic updates on, kept for merges from upstream
fn enable() -> Result<(), String> {
    let me = env::current_exe().map_err(|e| e.to_string())?;
    fs::create_dir_all(home()).map_err(|e| e.to_string())?;
    let same = me.canonicalize().ok().zip(self_exe().canonicalize().ok()).is_some_and(|(a, b)| a == b);
    if !same {
        put(&me, &self_exe())?;
    }
    let exe = self_exe().display().to_string();
    let uninstall = format!("\"{exe}\" rollback --yes");
    let vals = [("DisplayName", "REG_SZ", "WinRust"), ("DisplayVersion", "REG_SZ", env!("CARGO_PKG_VERSION")), ("Publisher", "REG_SZ", "Lemxy"), ("DisplayIcon", "REG_SZ", &exe), ("UninstallString", "REG_SZ", &uninstall), ("NoModify", "REG_DWORD", "1"), ("NoRepair", "REG_DWORD", "1")];
    for (name, kind, data) in vals {
        soft(t!("нет записи в «Установленных приложениях»", "no entry in Installed apps", ja: "インストール済みアプリに登録されませんでした", pt: "sem entrada em Aplicativos instalados", zh: "未添加到“已安装的应用”"), run("reg", &["add", ARP, "/v", name, "/t", kind, "/d", data, "/f"]));
    }
    // SYSTEM: writes Program Files without asking. At boot (after 5 min: the network is up) and daily at a random time.
    let task = format!(
        "$a=New-ScheduledTaskAction -Execute '{}' -Argument autoupdate; \
         $t=@((New-ScheduledTaskTrigger -AtStartup), (New-ScheduledTaskTrigger -Daily -At 12:00 -RandomDelay (New-TimeSpan -Hours 4))); $t[0].Delay='PT5M'; \
         Register-ScheduledTask -TaskName {TASK} -Force -Action $a -Trigger $t -Principal (New-ScheduledTaskPrincipal -UserId 'S-1-5-18' -LogonType ServiceAccount -RunLevel Highest) \
         -Settings (New-ScheduledTaskSettingsSet -StartWhenAvailable -RunOnlyIfNetworkAvailable -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries -ExecutionTimeLimit (New-TimeSpan -Minutes 30)) | Out-Null",
        self_exe().display()
    );
    run("powershell", &["-NoProfile", "-Command", &task])
}
