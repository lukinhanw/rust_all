//! Эвристики «подозрительных» процессов. Это подсказки для внимания, а не антивирус:
//! совпадение с правилом не значит, что программа вредоносная (и наоборот).
use std::{
    collections::HashMap,
    ptr::{null, null_mut},
    sync::{mpsc, Arc, Mutex},
};

#[derive(Clone, Debug, PartialEq)]
pub struct Reason {
    pub sev: u8, // 1 — низкая, 2 — средняя, 3 — высокая
    pub text: String,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Sig {
    Unknown,  // ещё проверяется или проверить нельзя
    Signed,   // действительная цифровая подпись
    Unsigned, // подписи нет
    Invalid,  // подпись есть, но недействительна или не вызывает доверия
}

pub fn label(sev: u8) -> &'static str {
    match sev {
        3 => t!("⚠ Высокая", "⚠ High", ja: "⚠ 高", pt: "⚠ Alta", zh: "⚠ 高"),
        2 => t!("▲ Средняя", "▲ Medium", ja: "▲ 中", pt: "▲ Média", zh: "▲ 中"),
        1 => t!("● Низкая", "● Low", ja: "● 低", pt: "● Baixa", zh: "● 低"),
        _ => "",
    }
}

pub fn sev_word(sev: u8) -> &'static str {
    match sev {
        3 => t!("Высокая", "High", ja: "高", pt: "Alta", zh: "高"),
        2 => t!("Средняя", "Medium", ja: "中", pt: "Média", zh: "中"),
        _ => t!("Низкая", "Low", ja: "低", pt: "Baixa", zh: "低"),
    }
}

/// Системные процессы, под которые любят маскироваться вредоносные программы.
const SYSTEM_NAMES: [&str; 12] = [
    "svchost.exe", "lsass.exe", "csrss.exe", "winlogon.exe", "services.exe", "smss.exe", "wininit.exe", "spoolsv.exe", "taskhostw.exe",
    "dwm.exe", "explorer.exe", "conhost.exe",
];

/// Папки, откуда легитимные программы обычно не работают постоянно.
static RISKY_DIRS: std::sync::LazyLock<[(&'static str, &'static str); 4]> = std::sync::LazyLock::new(|| [
    (r"\temp\", t!("временной папки", "a temporary folder", ja: "一時フォルダー", pt: "uma pasta temporária", zh: "临时文件夹")),
    (r"\downloads\", t!("папки «Загрузки»", "the Downloads folder", ja: "ダウンロード フォルダー", pt: "a pasta Downloads", zh: "下载文件夹")),
    (r"\$recycle.bin\", t!("корзины", "the Recycle Bin", ja: "ごみ箱", pt: "a Lixeira", zh: "回收站")),
    (r"\users\public\", t!("общей папки Public", "the shared Public folder", ja: "共有の「パブリック」フォルダー", pt: "a pasta Público compartilhada", zh: "共享的公用文件夹")),
]);

pub struct Facts<'a> {
    pub name: &'a str,
    pub exe: Option<&'a str>,
    pub parent: Option<&'a str>,
    pub exists: bool,
    pub sig: Sig,
    pub windir: &'a str,
}

fn short(p: &str) -> String {
    let n = p.chars().count();
    if n <= 70 { p.to_string() } else { format!("…{}", p.chars().skip(n - 68).collect::<String>()) }
}

pub fn assess(f: &Facts) -> Vec<Reason> {
    let mut out = vec![];
    let mut add = |sev: u8, text: String| out.push(Reason { sev, text });
    let name = f.name.to_lowercase();
    let parent = f.parent.map(str::to_lowercase);

    if let Some(exe) = f.exe {
        let e = exe.to_lowercase().replace('/', "\\");
        let in_windows = e.starts_with(&f.windir.to_lowercase());
        if SYSTEM_NAMES.contains(&name.as_str()) && !in_windows {
            add(3, tf!("Имя совпадает с системным процессом «{}», но файл лежит не в папке Windows: {}", "The name matches the system process '{}' but the file is not in the Windows folder: {}", ja: "名前がシステム プロセス '{}' と一致しますが、ファイルが Windows フォルダーにありません: {}", pt: "O nome coincide com o do processo do sistema '{}', mas o arquivo não está na pasta do Windows: {}", zh: "名称与系统进程「{}」相同，但文件不在 Windows 文件夹中：{}", f.name, short(exe)));
        }
        if !f.exists {
            add(3, t!("Файла процесса нет на диске — его удалили после запуска (так ведёт себя вредоносный код, а иногда и установщики)", "The process file is gone from disk - it was deleted after launch (malware does this, and sometimes installers)", ja: "プロセスのファイルがディスクから消えています - 起動後に削除されました (マルウェアが行う手口で、インストーラーでも起こることがあります)", pt: "O arquivo do processo sumiu do disco - foi excluído após a execução (malware faz isso, e às vezes instaladores também)", zh: "该进程文件已从磁盘上消失——它在启动后被删除了（恶意软件常这样做，安装程序有时也会）").into());
        }
        let risky = RISKY_DIRS.iter().find(|(d, _)| e.contains(d));
        if let Some((_, what)) = risky {
            add(2, tf!("Запущен из {what}: {} — постоянные программы обычно стоят в Program Files или AppData\\Local\\Programs", "Started from {what}: {} - permanent programs usually live in Program Files or AppData\\Local\\Programs", ja: "{what}から起動されました: {} - 常駐するプログラムは通常 Program Files または AppData\\Local\\Programs にあります", pt: "Iniciado de {what}: {} - programas permanentes costumam ficar em Program Files ou AppData\\Local\\Programs", zh: "从{what}启动：{}——常驻程序通常位于 Program Files 或 AppData\\Local\\Programs", short(exe)));
        }
        if !in_windows {
            match f.sig {
                Sig::Invalid => add(2, t!("Цифровая подпись файла недействительна или не вызывает доверия", "The file's digital signature is invalid or not trusted", ja: "ファイルのデジタル署名が無効、または信頼されていません", pt: "A assinatura digital do arquivo é inválida ou não é confiável", zh: "该文件的数字签名无效或不受信任").into()),
                Sig::Unsigned if risky.is_some() => add(2, t!("Файл не подписан, а запущен из временной или пользовательской папки", "The file is unsigned and runs from a temporary or user folder", ja: "ファイルは署名されておらず、一時フォルダーまたはユーザー フォルダーから実行されています", pt: "O arquivo não é assinado e é executado de uma pasta temporária ou de usuário", zh: "该文件未签名，且从临时文件夹或用户文件夹运行").into()),
                Sig::Unsigned => add(1, t!("Файл не имеет цифровой подписи (издатель не подтверждён) — для самописных и портативных программ это нормально", "The file has no digital signature (publisher not verified) - normal for home-made and portable programs", ja: "ファイルにデジタル署名がありません (発行元が未確認) - 自作やポータブル プログラムでは普通のことです", pt: "O arquivo não tem assinatura digital (editor não verificado) - normal para programas caseiros e portáteis", zh: "该文件没有数字签名（发布者未验证）——自制和便携程序属于正常情况").into()),
                _ => {}
            }
        }
    }
    match (name.as_str(), parent.as_deref()) {
        ("svchost.exe", Some(p)) if p != "services.exe" => add(3, tf!("svchost.exe запущен не службой services.exe, а процессом «{}»", "svchost.exe was started not by services.exe but by the process '{}'", ja: "svchost.exe は services.exe ではなくプロセス '{}' によって起動されました", pt: "O svchost.exe foi iniciado não pelo services.exe, mas pelo processo '{}'", zh: "svchost.exe 不是由 services.exe 启动的，而是由进程「{}」启动", f.parent.unwrap_or(""))),
        ("lsass.exe", Some(p)) if p != "wininit.exe" => add(3, tf!("lsass.exe запущен не wininit.exe, а процессом «{}»", "lsass.exe was started not by wininit.exe but by the process '{}'", ja: "lsass.exe は wininit.exe ではなくプロセス '{}' によって起動されました", pt: "O lsass.exe foi iniciado não pelo wininit.exe, mas pelo processo '{}'", zh: "lsass.exe 不是由 wininit.exe 启动的，而是由进程「{}」启动", f.parent.unwrap_or(""))),
        _ => {}
    }
    out.sort_by_key(|r| std::cmp::Reverse(r.sev));
    out
}

/// Файл действительно отсутствует? Только ошибка «не найден»: отказ в доступе, занятость и прочие сбои — не повод считать файл удалённым.
pub fn file_missing(path: &str) -> bool {
    !path.is_empty() && matches!(std::fs::metadata(path), Err(e) if e.kind() == std::io::ErrorKind::NotFound)
}

/// Проверка цифровой подписи файла (WinVerifyTrust, без обращения к сети).
pub fn verify(path: &str) -> Sig {
    use windows_sys::Win32::Security::WinTrust::*;
    let wide: Vec<u16> = path.encode_utf16().chain(Some(0)).collect();
    unsafe {
        let mut file = WINTRUST_FILE_INFO { cbStruct: std::mem::size_of::<WINTRUST_FILE_INFO>() as u32, pcwszFilePath: wide.as_ptr(), hFile: null_mut(), pgKnownSubject: null_mut() };
        let mut data: WINTRUST_DATA = std::mem::zeroed();
        data.cbStruct = std::mem::size_of::<WINTRUST_DATA>() as u32;
        data.dwUIChoice = WTD_UI_NONE;
        data.fdwRevocationChecks = WTD_REVOKE_NONE;
        data.dwUnionChoice = WTD_CHOICE_FILE;
        data.Anonymous.pFile = &mut file;
        data.dwStateAction = WTD_STATEACTION_VERIFY;
        data.dwProvFlags = WTD_CACHE_ONLY_URL_RETRIEVAL | WTD_REVOCATION_CHECK_NONE;
        let mut action = WINTRUST_ACTION_GENERIC_VERIFY_V2;
        let no_ui = -1isize as *mut core::ffi::c_void; // INVALID_HANDLE_VALUE: без окон
        let rc = WinVerifyTrust(no_ui, &mut action, &mut data as *mut _ as *mut core::ffi::c_void);
        data.dwStateAction = WTD_STATEACTION_CLOSE;
        WinVerifyTrust(no_ui, &mut action, &mut data as *mut _ as *mut core::ffi::c_void);
        let _ = null::<u8>();
        match rc as u32 {
            0 => Sig::Signed,
            0x800B0100 => Sig::Unsigned,         // TRUST_E_NOSIGNATURE
            0x800B0003 | 0x800B0001 => Sig::Unknown, // формат файла не поддерживает подпись / нет провайдера
            _ => Sig::Invalid,
        }
    }
}

/// Кэш проверок подписи: проверка идёт в фоновом потоке, чтобы не тормозить интерфейс.
pub struct SigCache {
    map: Arc<Mutex<HashMap<String, Sig>>>,
    tx: mpsc::Sender<String>,
}

impl SigCache {
    pub fn new() -> Self {
        let map = Arc::new(Mutex::new(HashMap::new()));
        let (tx, rx) = mpsc::channel::<String>();
        let m = map.clone();
        std::thread::spawn(move || {
            for p in rx {
                let s = verify(&p);
                if let Ok(mut g) = m.lock() {
                    g.insert(p, s);
                }
            }
        });
        Self { map, tx }
    }

    /// Сразу возвращает известный результат; для нового файла ставит проверку в очередь и отдаёт Unknown.
    pub fn get(&self, path: &str) -> Sig {
        let Ok(mut g) = self.map.lock() else { return Sig::Unknown };
        if let Some(s) = g.get(path) {
            return *s;
        }
        g.insert(path.to_string(), Sig::Unknown);
        let _ = self.tx.send(path.to_string());
        Sig::Unknown
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts<'a>(name: &'a str, exe: &'a str, parent: Option<&'a str>, exists: bool, sig: Sig) -> Facts<'a> {
        Facts { name, exe: Some(exe), parent, exists, sig, windir: r"C:\Windows" }
    }
    fn sev(r: &[Reason]) -> u8 {
        r.iter().map(|x| x.sev).max().unwrap_or(0)
    }

    #[test]
    fn normal_programs_are_not_flagged() {
        let r = assess(&facts("chrome.exe", r"C:\Program Files\Google\Chrome\Application\chrome.exe", Some("explorer.exe"), true, Sig::Signed));
        assert!(r.is_empty());
        let r = assess(&facts("svchost.exe", r"C:\Windows\System32\svchost.exe", Some("services.exe"), true, Sig::Unknown));
        assert!(r.is_empty());
    }

    #[test]
    fn fake_system_process_is_high() {
        let r = assess(&facts("svchost.exe", r"C:\Users\me\AppData\Local\Temp\svchost.exe", Some("services.exe"), true, Sig::Unsigned));
        assert_eq!(sev(&r), 3);
        assert!(r.iter().any(|x| x.text.contains("совпадает с системным")));
        assert!(r.iter().any(|x| x.text.contains("временной папки")));
    }

    #[test]
    fn wrong_parent_and_deleted_file_are_high() {
        assert_eq!(sev(&assess(&facts("svchost.exe", r"C:\Windows\System32\svchost.exe", Some("cmd.exe"), true, Sig::Unknown))), 3);
        assert_eq!(sev(&assess(&facts("lsass.exe", r"C:\Windows\System32\lsass.exe", Some("services.exe"), true, Sig::Unknown))), 3);
        assert_eq!(sev(&assess(&facts("a.exe", r"C:\Tools\a.exe", None, false, Sig::Signed))), 3);
    }

    #[test]
    fn downloads_and_signature_levels() {
        assert_eq!(sev(&assess(&facts("x.exe", r"C:\Users\me\Downloads\x.exe", None, true, Sig::Signed))), 2);
        assert_eq!(sev(&assess(&facts("x.exe", r"C:\Users\me\Downloads\x.exe", None, true, Sig::Unsigned))), 2);
        assert_eq!(sev(&assess(&facts("tool.exe", r"C:\Tools\tool.exe", None, true, Sig::Unsigned))), 1);
        assert_eq!(sev(&assess(&facts("tool.exe", r"C:\Tools\tool.exe", None, true, Sig::Invalid))), 2);
        // системные файлы из Windows подпись не проверяем (каталожные подписи) — не флагуем
        assert!(assess(&facts("notepad.exe", r"C:\Windows\notepad.exe", None, true, Sig::Unsigned)).is_empty());
    }

    #[test]
    fn missing_means_not_found_only() {
        assert!(file_missing(r"C:\no\such\file.exe"));
        assert!(!file_missing(r"C:\Windows\System32\cmd.exe"));
        assert!(!file_missing(r"C:\Windows\System32\..\System32\cmd.exe")); // путь с «..» (как у Git bash)
        assert!(!file_missing("")); // пустой путь — «неизвестно», а не «удалён»
    }

    #[test]
    fn real_signature_check_runs() {
        // cmd.exe лежит в каталоге Windows (подпись каталожная) — главное, что вызов не падает и даёт определённый ответ
        let s = verify(r"C:\Windows\System32\cmd.exe");
        assert!(matches!(s, Sig::Signed | Sig::Unsigned | Sig::Invalid | Sig::Unknown));
        // несуществующий файл не должен считаться подписанным
        assert_ne!(verify(r"C:\no\such\file.exe"), Sig::Signed);
    }
}
