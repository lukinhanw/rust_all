//! rust-search: NTFS search service + Alt+Space launcher; disables Windows Search (reversibly).
use crate::*;

/// Where launchers before the change journal kept the originals; turned into the journal's first lines, then deleted.
pub(crate) const STATE: &str = r"HKLM\SOFTWARE\plugin-launcher\rust-search";
pub(crate) const SB_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Search";

pub(crate) const SH_KEY: &str = r"HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion\Image File Execution Options\SearchHost.exe";

pub(crate) fn prog_dir() -> PathBuf {
    program_files().join("rs-search")
}

/// What this plugin changed in the system (journal.rs). An install by an older launcher is read from STATE: the Windows
/// defaults where it saved nothing.
pub(crate) fn journal() -> Result<Journal, String> {
    let j = Journal::open("rust-search", || {
        if run("reg", &["query", STATE]).is_err() && !prog_dir().join("rs.exe").is_file() {
            return vec![]; // never installed
        }
        let ws = svc_mode(reg_dword(STATE, "WSearchStart"), reg_dword(STATE, "WSearchDelayed")).unwrap_or("delayed-auto");
        let sb = reg_dword(STATE, "SearchboxMode").unwrap_or(2);
        vec![
            Entry::Svc { name: "WSearch".into(), mode: ws.into() },
            Entry::Reg { key: SH_KEY.into(), name: "Debugger".into(), old: None },
            Entry::Reg { key: SB_KEY.into(), name: "SearchboxTaskbarMode".into(), old: Some(("REG_DWORD".into(), sb.to_string())) },
        ]
    })?;
    let _ = run("reg", &["delete", STATE, "/f"]);
    Ok(j)
}

/// Stripped Windows builds (LTSC ISOs run through a debloater) have no WSearch service: nothing to switch off, not an error.
pub(crate) fn has_wsearch() -> bool {
    run("sc.exe", &["query", "WSearch"]).is_ok()
}

pub(crate) fn install() -> Result<(), String> {
    let j = journal()?; // before anything changes: a failed install is rolled back from it
    with_payload(&["rs.exe", "rs-launcher.exe"], |src| {
        run(src.join("rs.exe").to_str().unwrap(), &["--install"])?; // copies itself to Program Files, registers boot tasks
        let dst = prog_dir().join("rs-launcher.exe"); // must live somewhere stable: --autostart registers its own path
        let _ = run("taskkill", &["/f", "/im", "rs-launcher.exe"]);
        put(&src.join("rs-launcher.exe"), &dst)?;
        if logon_task("rs-launcher", &dst) {
            let _ = run("reg", &["delete", r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run", "/v", "rs-launcher", "/f"]);
        } else {
            run(dst.to_str().unwrap(), &["--autostart"])?; // fallback: Run key
        }
        // with the desktop user's normal token (launch::program): started from here it would inherit admin rights, and
        // every file opened from search would run elevated too.
        let _ = winrust_common::launch::program(&dst.to_string_lossy());
        if has_wsearch() {
            j.set_svc("WSearch", "disabled")?;
            let _ = run("sc.exe", &["stop", "WSearch"]);
        } else {
            WARN.lock().unwrap().push(t!("службы Windows Search (WSearch) нет в этой системе: шаг пропущен", "the Windows Search service (WSearch) does not exist on this system: step skipped", ja: "Windows Search サービス (WSearch) がこのシステムに存在しません: 手順をスキップしました", pt: "o serviço Pesquisa do Windows (WSearch) não existe neste sistema: etapa ignorada", zh: "此系统上不存在 Windows Search 服务（WSearch）：已跳过该步骤").into());
        }
        // SearchHost.exe (the Start/Win+S search UI) is not a service, so it gets the same IFEO trick as taskmgr: Windows runs `cmd /c exit` instead.
        j.set_reg(SH_KEY, "Debugger", "REG_SZ", "cmd.exe /c exit")?;
        let _ = run("taskkill", &["/f", "/im", "SearchHost.exe"]);
        j.set_reg(SB_KEY, "SearchboxTaskbarMode", "REG_DWORD", "0") // hide the taskbar search box
    })
}

pub(crate) fn uninstall() -> Result<(), String> {
    // Restore Windows Search before removing the launcher. An error leaves the plugin available for a retry.
    journal()?.undo()?;
    let _ = run("taskkill", &["/f", "/im", "rs-launcher.exe"]);
    let _ = run("reg", &["delete", r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run", "/v", "rs-launcher", "/f"]);
    let _ = run("schtasks", &["/delete", "/tn", "rs-launcher", "/f"]);
    let k = svc_key("WSearch");
    if svc_mode(reg_dword(&k, "Start"), reg_dword(&k, "DelayedAutostart")).is_some_and(|m| m.ends_with("auto")) {
        let _ = run("sc.exe", &["start", "WSearch"]);
    }
    // run the unpacked copy: an exe cannot delete itself, so the installed one would leave its folder behind
    let files = with_payload(&["rs.exe"], |src| run(src.join("rs.exe").to_str().unwrap(), &["--uninstall"]));
    let _ = fs::remove_dir_all(prog_dir());
    files
}
