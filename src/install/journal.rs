//! A plugin's change journal on disk (format and tests: common/src/journal.rs). Lives in Program Files (admin-only): the elevated
//! uninstall replays it, so a file a user could plant or edit would be a way to write any HKLM value.
use crate::*;
pub(crate) use i18n::journal::{Entry, svc_mode};

pub(crate) struct Journal(PathBuf);

pub(crate) fn svc_key(name: &str) -> String {
    format!(r"HKLM\SYSTEM\CurrentControlSet\Services\{name}")
}

/// A value as the journal stores it: (type, data), DWORD data in decimal. None = no such value.
pub(crate) fn reg_get(key: &str, name: &str) -> Option<(String, String)> {
    let o = out("reg", &["query", key, "/v", name]).ok()?;
    let (ty, d) = i18n::reg::value(&o)?;
    let d = if ty == "REG_DWORD" { i18n::reg::dword(&o)?.to_string() } else { d.to_string() };
    Some((ty.to_string(), d))
}

impl Journal {
    /// Opens `plugin`'s journal. A missing one starts with `legacy()`: what a launcher from before the journal changed.
    pub(crate) fn open(plugin: &str, legacy: impl FnOnce() -> Vec<Entry>) -> Result<Journal, String> {
        let j = Journal(program_files().join(r"WinRust\journal").join(format!("{plugin}.txt")));
        match fs::metadata(&j.0) {
            Ok(_) => {
                j.entries()?;
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                fs::create_dir_all(j.0.parent().unwrap()).map_err(|e| e.to_string())?;
                let text: String = legacy().iter().filter_map(Entry::line).map(|l| l + "\n").collect();
                let mut f = fs::OpenOptions::new().write(true).create_new(true).open(&j.0)
                    .map_err(|e| format!("{}: {e}", j.0.display()))?;
                f.write_all(text.as_bytes()).and_then(|_| f.sync_all())
                    .map_err(|e| format!("{}: {e}", j.0.display()))?;
            }
            Err(e) => return Err(format!("{}: {e}", j.0.display())),
        }
        Ok(j)
    }

    fn entries(&self) -> Result<Vec<Entry>, String> {
        let path = self.0.display();
        let text = fs::read_to_string(&self.0).map_err(|e| format!("{path}: {e}"))?;
        i18n::journal::parse_all(&text)
            .map_err(|line| tf!("{path}: повреждена строка журнала {line}", "{path}: invalid journal record on line {line}", ja: "{path}: {line} 行目のジャーナル レコードが無効です", pt: "{path}: registro de diário inválido na linha {line}", zh: "{path}：第 {line} 行的日志记录无效"))
    }

    /// Writes `e` down before the change it describes; a value already in the journal keeps its original.
    fn record(&self, e: Entry) -> Result<(), String> {
        if self.entries()?.iter().any(|x| x.same(&e)) {
            return Ok(());
        }
        let line = e.line().ok_or(tf!("не могу сохранить старое значение: {e:?}", "cannot save the old value: {e:?}", ja: "古い値を保存できません: {e:?}", pt: "não foi possível salvar o valor antigo: {e:?}", zh: "无法保存旧值：{e:?}"))?;
        let mut f = fs::OpenOptions::new().append(true).open(&self.0).map_err(|e| format!("{}: {e}", self.0.display()))?;
        writeln!(f, "{line}").and_then(|_| f.sync_all()).map_err(|e| format!("{}: {e}", self.0.display()))
    }

    pub(crate) fn set_reg(&self, key: &str, name: &str, ty: &str, data: &str) -> Result<(), String> {
        self.record(Entry::Reg { key: key.into(), name: name.into(), old: reg_get(key, name) })?;
        run("reg", &["add", key, "/v", name, "/t", ty, "/d", data, "/f"])
    }

    pub(crate) fn set_svc(&self, name: &str, mode: &str) -> Result<(), String> {
        let k = svc_key(name);
        let old = svc_mode(reg_dword(&k, "Start"), reg_dword(&k, "DelayedAutostart")).ok_or(tf!("служба {name}: неизвестный тип запуска", "service {name}: unknown start type", ja: "サービス {name}: 開始の種類が不明です", pt: "serviço {name}: tipo de início desconhecido", zh: "服务 {name}：未知的启动类型"))?;
        self.record(Entry::Svc { name: name.into(), mode: old.into() })?;
        run("sc.exe", &["config", name, "start=", mode])
    }

    /// Puts everything back, newest first. The journal is deleted only when every entry was restored, so a retry can finish.
    pub(crate) fn undo(&self) -> Result<(), String> {
        let errs: Vec<String> = self.entries()?.iter().rev().filter_map(|e| restore(e).err()).collect();
        if !errs.is_empty() {
            return Err(errs.join("; "));
        }
        let _ = fs::remove_file(&self.0);
        let _ = fs::remove_dir(self.0.parent().unwrap()); // only when empty
        Ok(())
    }
}

fn restore(e: &Entry) -> Result<(), String> {
    match e {
        // the change itself may have failed: a value that is not there is already restored
        Entry::Reg { key, name, old: None } => run("reg", &["delete", key, "/v", name, "/f"]).or_else(|e| if reg_get(key, name).is_none() { Ok(()) } else { Err(e) }),
        Entry::Reg { key, name, old: Some((ty, d)) } => run("reg", &["add", key, "/v", name, "/t", ty, "/d", d, "/f"]),
        // stripped builds may have lost the service since: nothing to restore
        Entry::Svc { name, mode } => run("sc.exe", &["config", name, "start=", mode]).or_else(|e| if run("sc.exe", &["query", name]).is_err() { Ok(()) } else { Err(e) }),
    }
}
