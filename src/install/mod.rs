//! Install logic, shared by the window and the command line so both do exactly the same thing:
//! process/registry helpers, the embedded payload, and one file per plugin with its install/uninstall pair.
use crate::*;
mod explorer;
mod journal;
mod search;
mod session;
mod taskmgr;
mod tools;
mod topbar;
pub(crate) use {explorer::*, journal::*, search::*, session::*, taskmgr::*, tools::*, topbar::*};

pub(crate) fn out(prog: &str, args: &[&str]) -> Result<String, String> {
    // reg.exe: HKCU means the signed-in user's, not the elevated account's (see session.rs)
    let owned: Vec<String> = if prog == "reg" { i18n::reg::user_args(args) } else { args.iter().map(|a| a.to_string()).collect() };
    let o = Command::new(prog).args(&owned).creation_flags(0x0800_0000).output().map_err(|e| format!("{prog}: {e}"))?;
    if o.status.success() {
        Ok(String::from_utf8_lossy(&o.stdout).into_owned())
    } else {
        Err(format!("{prog} {}: {}{}", owned.join(" "), String::from_utf8_lossy(&o.stdout).trim(), String::from_utf8_lossy(&o.stderr).trim()))
    }
}

pub(crate) fn run(prog: &str, args: &[&str]) -> Result<(), String> {
    out(prog, args).map(|_| ())
}

// Non-fatal failures of one install: shown as a count in the status line, spelled out in install.log.
pub(crate) static WARN: Mutex<Vec<String>> = Mutex::new(Vec::new());

pub(crate) fn soft(what: &str, r: Result<(), String>) {
    if let Err(e) = r {
        WARN.lock().unwrap().push(format!("{what}: {e}"));
    }
}

pub(crate) fn program_data() -> PathBuf {
    PathBuf::from(env::var("ProgramData").unwrap_or_else(|_| r"C:\ProgramData".into()))
}

/// Start Menu shortcut: search and the bar's Start panel list Start Menu shortcuts only.
pub(crate) fn start_lnk(name: &str) -> PathBuf {
    program_data().join(format!(r"Microsoft\Windows\Start Menu\Programs\{name}.lnk"))
}

pub(crate) fn shortcut(lnk: &Path, target: &Path, args: &str) -> Result<(), String> {
    run("powershell", &["-NoProfile", "-Command", &format!("$s=(New-Object -ComObject WScript.Shell).CreateShortcut('{}'); $s.TargetPath='{}'; $s.Arguments='{args}'; $s.Save()", lnk.display(), target.display())])
}

pub(crate) fn log(s: &str) {
    let d = program_data().join("plugin-launcher");
    let _ = fs::create_dir_all(&d);
    if let Ok(mut f) = fs::OpenOptions::new().create(true).append(true).open(d.join("install.log")) {
        let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
        let _ = writeln!(f, "[{t}] {s}"); // shortcut: unix seconds, no date formatting; upgrade if the log gets read by humans a lot
    }
}

/// Copies over an exe that was just killed: taskkill returns before Windows (or an antivirus scan) releases the file, so retry ~6 s.
pub(crate) fn put(src: &Path, dst: &Path) -> Result<(), String> {
    let mut r = fs::copy(src, dst).map(|_| ());
    for _ in 0..30 {
        if !matches!(&r, Err(e) if e.raw_os_error() == Some(32) || e.raw_os_error() == Some(5)) {
            break;
        }
        thread::sleep(std::time::Duration::from_millis(200));
        r = fs::copy(src, dst).map(|_| ());
    }
    r.map_err(|e| format!("{}: {e}", dst.display()))
}

pub(crate) fn reg_dword(key: &str, name: &str) -> Option<u32> {
    i18n::reg::dword(&out("reg", &["query", key, "/v", name]).ok()?)
}

/// A REG_SZ (or any string-like) value as text.
pub(crate) fn reg_str(key: &str, name: &str) -> Option<String> {
    i18n::reg::value(&out("reg", &["query", key, "/v", name]).ok()?).map(|(_, d)| d.to_string())
}

include!(concat!(env!("OUT_DIR"), "/payload.rs")); // PAYLOAD: the exes listed in payload.txt, embedded by build.rs

/// Update without rebuilding the launcher: an exe in `payload\` next to this one (or two levels up, the cargo
/// target\release layout) wins over the embedded copy. Trust level = the launcher exe's own folder. Two levels up only from
/// target\release: from C:\Program Files\WinRust it would be C:\payload, which any user may create.
pub(crate) fn disk_payload() -> Option<PathBuf> {
    let exe = env::current_exe().ok()?;
    let d = exe.parent()?;
    [Some(d.join("payload")), d.ends_with(r"target\release").then(|| d.join(r"..\..\payload"))].into_iter().flatten().find(|p| p.is_dir())
}

/// Unpacks the `need`ed bundled plugins, runs `f` on them, deletes them. The folder is under Program Files (admin-only writable),
/// not %TEMP%: a user-level process must not be able to swap an exe that is about to be installed as a SYSTEM task.
pub(crate) fn with_payload<T>(need: &[&str], f: impl FnOnce(&Path) -> Result<T, String>) -> Result<T, String> {
    let d = program_files().join(format!("plugin-launcher-tmp-{}", std::process::id()));
    fs::create_dir_all(&d).map_err(|e| format!("{}: {e}", d.display()))?;
    let disk = disk_payload();
    let unpack = |n: &&str| -> Result<(), String> {
        let bytes = PAYLOAD.iter().find(|(m, _)| m == n).ok_or(tf!("нет {n} в сборке", "{n} is missing from the build", ja: "{n} がビルドに含まれていません", pt: "{n} não está na compilação", zh: "构建中缺少 {n}"))?.1;
        match disk.as_ref().map(|p| p.join(n)).filter(|p| p.is_file()) {
            Some(src) => {
                log(&tf!("{n}: из {}", "{n}: from {}", ja: "{n}: {} から", pt: "{n}: de {}", zh: "{n}：来自 {}", src.display()));
                fs::copy(src, d.join(n)).map(|_| ())
            }
            None => fs::write(d.join(n), bytes),
        }
        .map_err(|e| tf!("распаковка {n}: {e}", "unpacking {n}: {e}", ja: "{n} を展開中: {e}", pt: "descompactando {n}: {e}", zh: "正在解压 {n}：{e}"))
    };
    let r = need.iter().try_for_each(unpack).and_then(|_| f(&d));
    let _ = fs::remove_dir_all(&d);
    r
}

pub(crate) fn program_files() -> PathBuf {
    PathBuf::from(env::var("ProgramFiles").unwrap_or(r"C:\Program Files".into()))
}

// shortcut: Windows makes at most one restore point per 24 h and it needs System Protection on; failures are ignored.
// Checkpoint-Computer can hang for good on some machines (the shadow copy is taken, the cmdlet never returns), which
// froze the install at its first plugin: after 60 s it is abandoned and the install goes on.
pub(crate) fn checkpoint() -> Result<(), String> {
    let mut c = Command::new("powershell")
        .args(["-NoProfile", "-Command", "Checkpoint-Computer -Description 'Plugin Launcher' -RestorePointType MODIFY_SETTINGS"])
        .creation_flags(0x0800_0000)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("powershell: {e}"))?;
    let end = std::time::Instant::now() + std::time::Duration::from_secs(60);
    loop {
        match c.try_wait().map_err(|e| e.to_string())? {
            Some(s) if s.success() => return Ok(()),
            Some(s) => return Err(format!("Checkpoint-Computer: {s}")),
            None if std::time::Instant::now() >= end => {
                let _ = c.kill();
                let _ = c.wait();
                return Err("Checkpoint-Computer: no answer after 60 s".into());
            }
            None => std::thread::sleep(std::time::Duration::from_millis(250)),
        }
    }
}

// Shared by the window and the command line, so both do exactly the same thing.
pub(crate) fn install_plugin(i: usize) -> Result<String, String> {
    let p = &PLUGINS[i];
    let (inst, uninst) = p.actions.ok_or(t!("установка недоступна", "installation is not available", ja: "インストールは利用できません", pt: "a instalação não está disponível", zh: "安装不可用"))?;
    init().inspect_err(|e| log(&tf!("{}: ОШИБКА {e}", "{}: ERROR {e}", ja: "{}: エラー {e}", pt: "{}: ERRO {e}", zh: "{}：错误 {e}", p.name)))?; // before anything is written: no user, no install
    let was = (p.installed)(); // already installed = an update: a failure must not remove the working copy
    WARN.lock().unwrap().clear();
    soft(t!("точка восстановления не создана", "restore point was not created", ja: "復元ポイントを作成できませんでした", pt: "o ponto de restauração não foi criado", zh: "未创建还原点"), checkpoint());
    if let Err(e) = inst() {
        log(&tf!("{}: ОШИБКА {e}", "{}: ERROR {e}", ja: "{}: エラー {e}", pt: "{}: ERRO {e}", zh: "{}：错误 {e}", p.name));
        if was {
            return Err(e);
        }
        let rollback = match uninst() {
            Ok(()) => t!("автоматический откат завершён", "automatic rollback completed", ja: "自動ロールバックが完了しました", pt: "reversão automática concluída", zh: "自动回滚已完成").to_string(),
            Err(u) => tf!("ошибка автоматического отката: {u}", "automatic rollback failed: {u}", ja: "自動ロールバックに失敗しました: {u}", pt: "a reversão automática falhou: {u}", zh: "自动回滚失败：{u}"),
        };
        log(&format!("{}: {rollback}", p.name));
        return Err(format!("{e} ({rollback})"));
    }
    updater_sync(); // first install: the WinRust-update task appears (a failure is a warning)
    let w = WARN.lock().unwrap().clone();
    log(&format!("{}: {} {}", p.name, if was { t!("обновлён", "updated", ja: "更新済み", pt: "atualizado", zh: "已更新") } else { t!("установлен", "installed", ja: "インストール済み", pt: "instalado", zh: "已安装") }, w.join(" | ")));
    let note = if w.is_empty() { String::new() } else { tf!(" Предупреждений: {} (см. %ProgramData%\\plugin-launcher\\install.log).", " Warnings: {} (see %ProgramData%\\plugin-launcher\\install.log).", ja: " 警告: {} 件 (%ProgramData%\\plugin-launcher\\install.log を参照)。", pt: " Avisos: {} (veja %ProgramData%\\plugin-launcher\\install.log).", zh: " 警告：{}（见 %ProgramData%\\plugin-launcher\\install.log）。", w.len()) };
    Ok(format!("{} {}.{note}", p.name, if was { t!("обновлён", "updated", ja: "更新済み", pt: "atualizado", zh: "已更新") } else { t!("установлен", "installed", ja: "インストール済み", pt: "instalado", zh: "已安装") }))
}

pub(crate) fn uninstall_plugin(i: usize) -> Result<String, String> {
    let p = &PLUGINS[i];
    let (_, uninst) = p.actions.ok_or(t!("удаление недоступно", "uninstall is not available", ja: "アンインストールは利用できません", pt: "a desinstalação não está disponível", zh: "卸载不可用"))?;
    WARN.lock().unwrap().clear();
    init_soft();
    uninst().inspect_err(|e| log(&tf!("{}: ОШИБКА удаления {e}", "{}: uninstall ERROR {e}", ja: "{}: アンインストール エラー {e}", pt: "{}: ERRO ao desinstalar {e}", zh: "{}：卸载错误 {e}", p.name)))?;
    updater_sync();
    let w = WARN.lock().unwrap().clone();
    log(&format!("{}: {} {}", p.name, t!("удалён", "removed", ja: "削除済み", pt: "removido", zh: "已移除"), w.join(" | ")));
    let note = if w.is_empty() { String::new() } else { tf!(" Предупреждений: {} (см. %ProgramData%\\plugin-launcher\\install.log).", " Warnings: {} (see %ProgramData%\\plugin-launcher\\install.log).", ja: " 警告: {} 件 (%ProgramData%\\plugin-launcher\\install.log を参照)。", pt: " Avisos: {} (veja %ProgramData%\\plugin-launcher\\install.log).", zh: " 警告：{}（见 %ProgramData%\\plugin-launcher\\install.log）。", w.len()) };
    Ok(tf!("{} удалён.{note}", "{} removed.{note}", ja: "{} を削除しました。{note}", pt: "{} removido.{note}", zh: "{} 已移除。{note}", p.name))
}

pub(crate) fn rollback() -> Result<String, String> {
    WARN.lock().unwrap().clear();
    init_soft();
    // reverse install order: rust-explorer (which switched Explorer off) goes before topbar, else the session is left with no panel and no shell
    let errs: Vec<String> = PLUGINS.iter().rev().filter(|p| (p.installed)()).filter_map(|p| p.actions.and_then(|a| a.1().err().map(|e| format!("{}: {e}", p.name)))).collect();
    updater_sync(); // nothing left: the update task and the installer copy go too
    let warnings = WARN.lock().unwrap().join(" | ");
    let note = if warnings.is_empty() { "" } else {
        log(&format!("rollback: {warnings}"));
        t!(" Есть предупреждения (см. %ProgramData%\\plugin-launcher\\install.log).", " See warnings in %ProgramData%\\plugin-launcher\\install.log.", ja: " 警告は %ProgramData%\\plugin-launcher\\install.log を参照してください。", pt: " Veja os avisos em %ProgramData%\\plugin-launcher\\install.log.", zh: " 警告请见 %ProgramData%\\plugin-launcher\\install.log。")
    };
    if errs.is_empty() {
        Ok(tf!("Все плагины удалены.{note}", "All plugins removed.{note}", ja: "すべてのプラグインを削除しました。{note}", pt: "Todos os plugins foram removidos.{note}", zh: "所有插件已移除。{note}"))
    } else {
        Err(format!("{}{note}", errs.join("; ")))
    }
}
