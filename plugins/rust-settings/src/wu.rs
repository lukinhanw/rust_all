//! Windows Update through the Windows Update Agent COM API (documented): look for updates, download and install them.
//! Searching and installing take minutes, so they run in a thread; the window shows `state()`. Installing needs administrator
//! rights (Windows says "access denied" otherwise, which is reported as such).
use std::sync::Mutex;
use windows::{
    Win32::System::{
        Com::{CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx},
        UpdateAgent::*,
    },
    core::BSTR,
};

#[derive(Clone, Default)]
pub struct State {
    pub busy: Option<String>,   // what is going on right now
    pub found: Option<Vec<String>>, // titles of the updates waiting; None: not looked yet
    pub note: String,           // the result of the last action
}

static STATE: Mutex<State> = Mutex::new(State { busy: None, found: None, note: String::new() });

pub fn state() -> State {
    STATE.lock().map(|s| s.clone()).unwrap_or_default()
}

fn with<T>(f: impl FnOnce(&mut State) -> T) {
    if let Ok(mut s) = STATE.lock() {
        f(&mut s);
    }
}

fn session() -> windows::core::Result<IUpdateSession> {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        CoCreateInstance(&UpdateSession, None, CLSCTX_INPROC_SERVER)
    }
}

fn search(s: &IUpdateSession) -> windows::core::Result<IUpdateCollection> {
    // no Type filter: drivers and firmware too, as Windows Update itself offers them (#100)
    unsafe { s.CreateUpdateSearcher()?.Search(&BSTR::from("IsInstalled=0 and IsHidden=0"))?.Updates() }
}

fn titles(c: &IUpdateCollection) -> Vec<String> {
    unsafe { (0..c.Count().unwrap_or(0)).filter_map(|i| c.get_Item(i).ok()).filter_map(|u| u.Title().ok()).map(|t| t.to_string()).collect() }
}

fn err_text(e: &windows::core::Error) -> String {
    if e.code().0 as u32 == 0x8007_0005 {
        t!("Нужны права администратора", "Administrator rights are needed", ja: "管理者権限が必要です", pt: "São necessários direitos de administrador", zh: "需要管理员权限").into()
    } else {
        e.message()
    }
}

fn start(what: &str, done: impl FnOnce() + Send + 'static, job: impl FnOnce() -> String + Send + 'static) {
    let mut busy = true;
    with(|s| {
        busy = s.busy.is_some();
        if !busy {
            s.busy = Some(what.to_string());
        }
    });
    if busy {
        return;
    }
    std::thread::spawn(move || {
        let note = job();
        with(|s| (s.busy, s.note) = (None, note));
        done();
    });
}

/// Looks for updates.
pub fn check(done: impl FnOnce() + Send + 'static) {
    start(t!("Поиск обновлений…", "Checking for updates…", ja: "更新プログラムを確認しています…", pt: "Verificando atualizações…", zh: "正在检查更新…"), done, || match session().and_then(|s| search(&s)) {
        Ok(c) => {
            let t = titles(&c);
            let note = if t.is_empty() { t!("Обновлений нет: установлены все", "You're up to date", ja: "最新の状態です", pt: "Você está atualizado", zh: "已安装所有更新").into() } else { tf!("Найдено обновлений: {}", "Updates found: {}", ja: "見つかった更新プログラム: {}", pt: "Atualizações encontradas: {}", zh: "找到更新：{}", t.len()) };
            with(|s| s.found = Some(t));
            note
        }
        Err(e) => tf!("Не удалось проверить: {}", "Could not check: {}", ja: "確認できませんでした: {}", pt: "Não foi possível verificar: {}", zh: "无法检查：{}", err_text(&e)),
    });
}

/// Downloads and installs every update found by `check`.
pub fn install(done: impl FnOnce() + Send + 'static) {
    start(t!("Загрузка и установка обновлений…", "Downloading and installing updates…", ja: "更新プログラムをダウンロードしてインストールしています…", pt: "Baixando e instalando atualizações…", zh: "正在下载并安装更新…"), done, || {
        let run = || -> windows::core::Result<String> {
            unsafe {
                let s = session()?;
                let found = search(&s)?;
                if found.Count()? == 0 {
                    return Ok(t!("Обновлений нет: установлены все", "You're up to date", ja: "最新の状態です", pt: "Você está atualizado", zh: "已安装所有更新").into());
                }
                for i in 0..found.Count()? {
                    let u = found.get_Item(i)?;
                    if !u.EulaAccepted()?.as_bool() {
                        u.AcceptEula()?;
                    }
                }
                let d = s.CreateUpdateDownloader()?;
                d.SetUpdates(&found)?;
                d.Download()?;
                let inst = s.CreateUpdateInstaller()?;
                inst.SetUpdates(&found)?;
                let r = inst.Install()?;
                Ok(if r.RebootRequired()?.as_bool() { t!("Установлено. Нужна перезагрузка", "Installed. Restart to finish", ja: "インストールしました。完了するには再起動してください", pt: "Instalado. Reinicie para concluir", zh: "安装完成，需要重启").into() } else { t!("Обновления установлены", "Updates installed", ja: "更新プログラムをインストールしました", pt: "Atualizações instaladas", zh: "更新已安装").into() })
            }
        };
        match run() {
            Ok(m) => {
                with(|s| s.found = Some(vec![]));
                m
            }
            Err(e) => tf!("Не удалось установить: {}", "Could not install: {}", ja: "インストールできませんでした: {}", pt: "Não foi possível instalar: {}", zh: "无法安装：{}", err_text(&e)),
        }
    });
}
