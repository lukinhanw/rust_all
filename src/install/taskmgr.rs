//! rtaskmgr-win32: replaces Task Manager through an IFEO Debugger; ships rs-du (disk cleaner) next to it.
use crate::*;

pub(crate) const TM_KEY: &str = r"HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion\Image File Execution Options\taskmgr.exe";

/// Another replacement's Debugger (Process Explorer's "Replace Task Manager" writes the same value), saved once, put back on uninstall.
pub(crate) const TM_STATE: &str = r"HKLM\SOFTWARE\plugin-launcher\rtaskmgr-win32";

/// Ours only: a Debugger set by another program is not this plugin.
pub(crate) fn tm_installed() -> bool {
    reg_str(TM_KEY, "Debugger").is_some_and(|d| d.to_lowercase().contains("rtaskmgr"))
}

pub(crate) fn tm_dir() -> PathBuf {
    program_files().join("rtaskmgr")
}

// Image File Execution Options "Debugger": Windows starts our exe instead of taskmgr.exe (the system file stays untouched, so this is fully reversible).
pub(crate) fn tm_install() -> Result<(), String> {
    if let Some(old) = reg_str(TM_KEY, "Debugger").filter(|_| !tm_installed() && reg_str(TM_STATE, "Debugger").is_none()) {
        run("reg", &["add", TM_STATE, "/v", "Debugger", "/t", "REG_SZ", "/d", &old, "/f"])?;
    }
    with_payload(&["Task manager.exe", "rs-du.exe", "debloat.exe"], |d| {
        let dst = tm_dir().join("rtaskmgr-win32.exe");
        let _ = run("taskkill", &["/f", "/im", "rtaskmgr-win32.exe"]); // an update: a running exe cannot be overwritten
        fs::create_dir_all(tm_dir()).map_err(|e| e.to_string())?;
        put(&d.join("Task manager.exe"), &dst)?;
        put(&d.join("rs-du.exe"), &tm_dir().join("rs-du.exe"))?;
        put(&d.join("debloat.exe"), &tm_dir().join("debloat.exe"))?; // the Speed page's optional groups (updaters, Defender, NTFS, hosts)
        // the search (and the bar's Start panel) lists Start Menu shortcuts only, so without one the task manager is invisible to them
        soft(t!("ярлык в меню Пуск не создан, поиск не увидит диспетчер", "Start menu shortcut was not created, search will not find the task manager", ja: "スタート メニューのショートカットを作成できませんでした。検索でタスク マネージャーが見つかりません", pt: "O atalho do menu Iniciar não foi criado; a pesquisa não encontrará o gerenciador de tarefas", zh: "开始菜单快捷方式未创建，搜索将找不到任务管理器"), shortcut(&start_lnk("Task manager"), &dst, ""));
        // elevated start without a UAC prompt (the exe re-launches itself through this task), so it can end protected processes
        let task = format!("{} Register-ScheduledTask -TaskName rtaskmgr -Force -Action (New-ScheduledTaskAction -Execute '{}') -Principal (New-ScheduledTaskPrincipal -UserId $u -LogonType Interactive -RunLevel Highest) -Settings (New-ScheduledTaskSettingsSet -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries -ExecutionTimeLimit ([TimeSpan]::Zero) -MultipleInstances Parallel) | Out-Null", task_user(), dst.display());
        soft(t!("задача rtaskmgr не создана, запуск от администратора будет через UAC", "rtaskmgr task was not created, launching as administrator will go through UAC", ja: "rtaskmgr タスクを作成できませんでした。管理者として起動すると UAC が表示されます", pt: "a tarefa rtaskmgr não foi criada; abrir como administrador passará pelo UAC", zh: "rtaskmgr 计划任务未创建，以管理员身份启动时会经过 UAC"), run("powershell", &["-NoProfile", "-Command", &task]));
        run("reg", &["add", TM_KEY, "/v", "Debugger", "/t", "REG_SZ", "/d", &format!("\"{}\"", dst.display()), "/f"])
    })
}

pub(crate) fn tm_uninstall() -> Result<(), String> {
    // first what the Speed page switched on (profile, shell blocks, debloat groups): it needs the exe and debloat.exe next to it.
    // On failure everything stays, so uninstalling again retries.
    let exe = tm_dir().join("rtaskmgr-win32.exe");
    if exe.is_file() {
        let _ = run("taskkill", &["/f", "/im", "rtaskmgr-win32.exe"]);
        out(&exe.display().to_string(), &["--undo-speed"])?;
    }
    let _ = fs::remove_file(tm_dir().join("debloat.exe"));
    let _ = fs::remove_file(start_lnk("Task manager"));
    let _ = run("schtasks", &["/delete", "/tn", "rtaskmgr", "/f"]);
    // registry first: with the exe gone, taskmgr.exe would not start at all. Only our value: the key may hold
    // other IFEO settings (e.g. mitigation policies) that are not ours to remove. The replacement found at install comes back.
    match reg_str(TM_STATE, "Debugger") {
        Some(old) => run("reg", &["add", TM_KEY, "/v", "Debugger", "/t", "REG_SZ", "/d", &old, "/f"])?,
        None if tm_installed() => run("reg", &["delete", TM_KEY, "/v", "Debugger", "/f"])?,
        None => {}
    }
    let _ = run("reg", &["delete", TM_STATE, "/f"]);
    let _ = run("taskkill", &["/f", "/im", "rtaskmgr-win32.exe"]);
    let _ = fs::remove_file(tm_dir().join("rtaskmgr-win32.exe"));
    cleaner_release(bar_dir().join("topbar.exe").is_file());
    Ok(())
}

/// rs-du (disk cleaner) lives in the task manager folder and is shared by rtaskmgr-win32 and the topbar menu:
/// it goes away only when the other plugin is gone too.
pub(crate) fn cleaner_release(other_installed: bool) {
    if !other_installed {
        let _ = fs::remove_file(tm_dir().join("rs-du.exe"));
    }
    let _ = fs::remove_dir(tm_dir()); // only succeeds when empty
}
