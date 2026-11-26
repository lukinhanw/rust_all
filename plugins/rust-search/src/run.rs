//! The launcher's Run mode (Win+R through the topbar): what to start for a typed line, the history of what was run,
//! and suggestions while typing (history first, then the entries of a typed folder).
use std::path::PathBuf;

const HISTORY_MAX: usize = 50;

/// Splits a typed line into (file, arguments) the way the Windows Run box does: a quoted first part is the file;
/// an existing path, a URL or a `shell:` / `ms-settings:` address is taken whole (paths may contain spaces);
/// otherwise the first word is the program and the rest its arguments.
pub fn split_cmd(line: &str, exists: impl Fn(&str) -> bool) -> (String, String) {
    let line = line.trim();
    if let Some(rest) = line.strip_prefix('"') {
        let (file, args) = rest.split_once('"').unwrap_or((rest, ""));
        return (file.to_string(), args.trim().to_string());
    }
    let scheme = line.split_once(':').is_some_and(|(s, _)| s.len() > 1 && s.chars().all(|c| c.is_ascii_alphanumeric() || "-+.".contains(c)));
    if scheme || exists(line) {
        return (line.to_string(), String::new());
    }
    let (file, args) = line.split_once(' ').unwrap_or((line, ""));
    (file.to_string(), args.trim().to_string())
}

/// Expands %VARIABLES% (unknown ones stay as typed).
pub fn expand(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(a) = rest.find('%') {
        out.push_str(&rest[..a]);
        let after = &rest[a + 1..];
        match after.find('%') {
            Some(b) if b > 0 && std::env::var(&after[..b]).is_ok() => {
                out.push_str(&std::env::var(&after[..b]).unwrap());
                rest = &after[b + 1..];
            }
            _ => {
                out.push('%');
                rest = after;
            }
        }
    }
    out + rest
}

fn history_file() -> Option<PathBuf> {
    // per user (it is what this person typed), not the shared ProgramData
    std::env::var_os("LOCALAPPDATA").map(|p| PathBuf::from(p).join("WinRust").join("run-history.txt"))
}

pub fn history() -> Vec<String> {
    history_file().and_then(|f| std::fs::read_to_string(f).ok()).unwrap_or_default().lines().filter(|l| !l.trim().is_empty()).map(String::from).collect()
}

/// `line` moved to the top of `list` (no duplicates, any case), at most HISTORY_MAX kept.
pub fn remember_in(list: &mut Vec<String>, line: &str) {
    let line = line.trim();
    list.retain(|l| !l.eq_ignore_ascii_case(line));
    list.insert(0, line.to_string());
    list.truncate(HISTORY_MAX);
}

pub fn remember(line: &str) {
    let mut h = history();
    remember_in(&mut h, line);
    if let Some(f) = history_file() {
        let _ = std::fs::create_dir_all(f.parent().unwrap());
        let _ = std::fs::write(f, h.join("\n"));
    }
}

/// Commands people type into Run, with what they open (Russian, English, Japanese, Portuguese, Chinese). The first POPULAR ones fill an empty
/// history. A description match finds them too: "реестр" or "registry" → regedit.
pub const KNOWN: &[(&str, &str, &str, &str, &str, &str)] = &[
    ("cmd", "Командная строка", "Command Prompt", "コマンド プロンプト", "Prompt de Comando", "命令提示符"),
    ("powershell", "Windows PowerShell", "Windows PowerShell", "Windows PowerShell", "Windows PowerShell", "Windows PowerShell"),
    ("regedit", "Редактор реестра", "Registry Editor", "レジストリ エディター", "Editor do Registro", "注册表编辑器"),
    ("taskmgr", "Диспетчер задач", "Task Manager", "タスク マネージャー", "Gerenciador de Tarefas", "任务管理器"),
    ("control", "Панель управления", "Control Panel", "コントロール パネル", "Painel de Controle", "控制面板"),
    ("msconfig", "Конфигурация системы, загрузка", "System Configuration, boot", "システム構成、ブート", "Configuração do Sistema, inicialização", "系统配置、启动"),
    ("services.msc", "Службы", "Services", "サービス", "Serviços", "服务"),
    ("devmgmt.msc", "Диспетчер устройств", "Device Manager", "デバイス マネージャー", "Gerenciador de Dispositivos", "设备管理器"),
    ("wt", "Терминал Windows", "Windows Terminal", "Windows ターミナル", "Terminal do Windows", "Windows 终端"),
    ("notepad", "Блокнот", "Notepad", "メモ帳", "Bloco de Notas", "记事本"),
    ("calc", "Калькулятор", "Calculator", "電卓", "Calculadora", "计算器"),
    ("mspaint", "Paint, рисование", "Paint", "ペイント", "Paint", "画图"),
    ("diskmgmt.msc", "Управление дисками, разделы", "Disk Management, partitions", "ディスクの管理、パーティション", "Gerenciamento de Disco, partições", "磁盘管理、分区"),
    ("compmgmt.msc", "Управление компьютером", "Computer Management", "コンピューターの管理", "Gerenciamento do Computador", "计算机管理"),
    ("eventvwr.msc", "Просмотр событий, журналы", "Event Viewer, logs", "イベント ビューアー、ログ", "Visualizador de Eventos, logs", "事件查看器、日志"),
    ("gpedit.msc", "Редактор локальной групповой политики", "Local Group Policy Editor", "ローカル グループ ポリシー エディター", "Editor de Política de Grupo Local", "本地组策略编辑器"),
    ("taskschd.msc", "Планировщик заданий", "Task Scheduler", "タスク スケジューラ", "Agendador de Tarefas", "任务计划程序"),
    ("lusrmgr.msc", "Локальные пользователи и группы", "Local Users and Groups", "ローカル ユーザーとグループ", "Usuários e Grupos Locais", "本地用户和组"),
    ("perfmon", "Системный монитор, производительность", "Performance Monitor", "パフォーマンス モニター", "Monitor de Desempenho", "性能监视器"),
    ("resmon", "Монитор ресурсов", "Resource Monitor", "リソース モニター", "Monitor de Recursos", "资源监视器"),
    ("appwiz.cpl", "Программы и компоненты, удаление программ", "Programs and Features, uninstall", "プログラムと機能、アンインストール", "Programas e Recursos, desinstalar", "程序和功能、卸载程序"),
    ("optionalfeatures", "Компоненты Windows", "Windows Features", "Windows の機能", "Recursos do Windows", "Windows 功能"),
    ("ncpa.cpl", "Сетевые подключения, адаптеры", "Network Connections, adapters", "ネットワーク接続、アダプター", "Conexões de Rede, adaptadores", "网络连接、适配器"),
    ("sysdm.cpl", "Свойства системы, имя компьютера, переменные среды", "System Properties, computer name, environment variables", "システムのプロパティ、コンピューター名、環境変数", "Propriedades do Sistema, nome do computador, variáveis de ambiente", "系统属性、计算机名、环境变量"),
    ("powercfg.cpl", "Электропитание", "Power Options", "電源オプション", "Opções de Energia", "电源选项"),
    ("mmsys.cpl", "Звук, устройства воспроизведения", "Sound, playback devices", "サウンド、再生デバイス", "Som, dispositivos de reprodução", "声音、播放设备"),
    ("main.cpl", "Мышь", "Mouse", "マウス", "Mouse", "鼠标"),
    ("desk.cpl", "Экран, разрешение", "Display, resolution", "ディスプレイ、解像度", "Vídeo, resolução", "显示、分辨率"),
    ("timedate.cpl", "Дата и время", "Date and Time", "日付と時刻", "Data e Hora", "日期和时间"),
    ("inetcpl.cpl", "Свойства браузера, прокси", "Internet Options, proxy", "インターネット オプション、プロキシ", "Opções da Internet, proxy", "Internet 选项、代理"),
    ("firewall.cpl", "Брандмауэр Защитника Windows", "Windows Defender Firewall", "Windows Defender ファイアウォール", "Firewall do Windows Defender", "Windows Defender 防火墙"),
    ("cleanmgr", "Очистка диска", "Disk Cleanup", "ディスク クリーンアップ", "Limpeza de Disco", "磁盘清理"),
    ("dfrgui", "Оптимизация дисков, дефрагментация", "Optimize Drives, defragment", "ドライブの最適化、デフラグ", "Otimizar Unidades, desfragmentar", "优化驱动器、碎片整理"),
    ("msinfo32", "Сведения о системе", "System Information", "システム情報", "Informações do Sistema", "系统信息"),
    ("dxdiag", "Диагностика DirectX, видеокарта", "DirectX Diagnostic Tool, graphics", "DirectX 診断ツール、グラフィックス", "Ferramenta de Diagnóstico do DirectX, placa de vídeo", "DirectX 诊断工具、显卡"),
    ("winver", "О программе Windows, версия", "About Windows, version", "Windows のバージョン情報", "Sobre o Windows, versão", "关于 Windows、版本"),
    ("mstsc", "Подключение к удалённому рабочему столу", "Remote Desktop Connection", "リモート デスクトップ接続", "Conexão de Área de Trabalho Remota", "远程桌面连接"),
    ("osk", "Экранная клавиатура", "On-Screen Keyboard", "スクリーン キーボード", "Teclado Virtual", "屏幕键盘"),
    ("magnify", "Экранная лупа", "Magnifier", "拡大鏡", "Lupa", "放大镜"),
    ("charmap", "Таблица символов", "Character Map", "文字コード表", "Mapa de Caracteres", "字符映射表"),
    ("snippingtool", "Ножницы, снимок экрана", "Snipping Tool, screenshot", "Snipping Tool、スクリーンショット", "Ferramenta de Recorte, captura de tela", "截图工具、屏幕截图"),
    ("mrt", "Средство удаления вредоносных программ", "Malicious Software Removal Tool", "悪意のあるソフトウェアの削除ツール", "Ferramenta de Remoção de Software Mal-Intencionado", "恶意软件删除工具"),
    ("%temp%", "Временные файлы", "Temporary files", "一時ファイル", "Arquivos temporários", "临时文件"),
    ("%appdata%", "Папка AppData (Roaming)", "AppData folder (Roaming)", "AppData フォルダー (Roaming)", "Pasta AppData (Roaming)", "AppData 文件夹 (Roaming)"),
    ("%localappdata%", "Папка AppData (Local)", "AppData folder (Local)", "AppData フォルダー (Local)", "Pasta AppData (Local)", "AppData 文件夹 (Local)"),
    ("shell:startup", "Папка автозагрузки", "Startup folder", "スタートアップ フォルダー", "Pasta Inicializar", "启动文件夹"),
    ("shell:downloads", "Загрузки", "Downloads", "ダウンロード", "Downloads", "下载"),
    ("shell:appsfolder", "Все приложения", "All apps", "すべてのアプリ", "Todos os aplicativos", "所有应用"),
    ("ms-settings:", "Параметры Windows", "Windows Settings", "Windows の設定", "Configurações do Windows", "Windows 设置"),
    ("ms-settings:windowsupdate", "Центр обновления Windows", "Windows Update", "Windows Update", "Windows Update", "Windows 更新"),
    ("ms-settings:display", "Параметры: экран", "Settings: display", "設定: ディスプレイ", "Configurações: vídeo", "设置：显示"),
    ("ms-settings:bluetooth", "Параметры: Bluetooth и устройства", "Settings: Bluetooth and devices", "設定: Bluetooth とデバイス", "Configurações: Bluetooth e dispositivos", "设置：蓝牙和设备"),
    ("ms-settings:network", "Параметры: сеть и Интернет", "Settings: network and internet", "設定: ネットワークとインターネット", "Configurações: rede e internet", "设置：网络和 Internet"),
    ("ms-settings:appsfeatures", "Параметры: установленные приложения", "Settings: installed apps", "設定: インストールされているアプリ", "Configurações: aplicativos instalados", "设置：已安装的应用"),
    ("ms-settings:personalization", "Параметры: персонализация", "Settings: personalization", "設定: 個人用設定", "Configurações: personalização", "设置：个性化"),
    ("ms-settings:sound", "Параметры: звук", "Settings: sound", "設定: サウンド", "Configurações: som", "设置：声音"),
];
const POPULAR: usize = 8;

/// The KNOWN entry for exactly this command, any case.
pub fn describe(line: &str) -> Option<usize> {
    KNOWN.iter().position(|k| k.0.eq_ignore_ascii_case(line.trim()))
}

/// Program names Run can start by name: the .exe files on PATH and the App Paths entries (chrome, winword...).
/// Sorted, no duplicates, without ".exe".
pub fn programs() -> Vec<String> {
    let mut v: Vec<String> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect::<Vec<_>>())
        .unwrap_or_default()
        .iter()
        .filter_map(|d| std::fs::read_dir(d).ok())
        .flatten()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .chain(app_paths())
        .filter_map(|n| n.to_lowercase().ends_with(".exe").then(|| n[..n.len() - 4].to_string()))
        .collect();
    v.sort_by_key(|s| s.to_lowercase());
    v.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
    v
}

fn app_paths() -> Vec<String> {
    use windows_sys::Win32::System::Registry::{HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, RegCloseKey, RegEnumKeyExW, RegOpenKeyExW};
    let sub: Vec<u16> = "Software\\Microsoft\\Windows\\CurrentVersion\\App Paths\0".encode_utf16().collect();
    let mut out = vec![];
    for root in [HKEY_LOCAL_MACHINE, HKEY_CURRENT_USER] {
        let mut k: HKEY = std::ptr::null_mut();
        if unsafe { RegOpenKeyExW(root, sub.as_ptr(), 0, KEY_READ, &mut k) } != 0 {
            continue;
        }
        for i in 0.. {
            let (mut buf, mut n) = ([0u16; 260], 260u32);
            if unsafe { RegEnumKeyExW(k, i, buf.as_mut_ptr(), &mut n, std::ptr::null(), std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut()) } != 0 {
                break;
            }
            out.push(String::from_utf16_lossy(&buf[..n as usize]));
        }
        unsafe { RegCloseKey(k) };
    }
    out
}

/// Where a suggestion row comes from (it decides the row's subtitle).
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Src {
    Typed,
    History,
    Folder,
    Known(usize), // index into KNOWN
    Program,      // from `programs()`
}

/// Rows for the typed text, best first: the text itself (Enter runs the selected row); history lines, known commands
/// and programs that start with it; history lines and known commands whose text or description contains it; the
/// entries of a typed folder (`C:\Us` → `C:\Users\`). Empty text: the recent history, topped up with common commands.
pub fn suggest(text: &str, history: &[String], programs: &[String]) -> Vec<(String, Src)> {
    let t = text.trim();
    let add = |s: &str, src: Src, out: &mut Vec<(String, Src)>| {
        if out.len() < 30 && !out.iter().any(|o| o.0.eq_ignore_ascii_case(s)) {
            out.push((s.to_string(), src));
        }
    };
    if t.is_empty() {
        let mut out = vec![];
        for h in history.iter().take(20) {
            add(h, Src::History, &mut out);
        }
        for (i, k) in KNOWN.iter().enumerate().take(POPULAR) {
            if out.len() < 7 {
                add(k.0, Src::Known(i), &mut out); // a new user sees what Run can do instead of a blank prompt
            }
        }
        return out;
    }
    let low = t.to_lowercase();
    let mut out = vec![(t.to_string(), Src::Typed)];
    for h in history.iter().filter(|h| h.to_lowercase().starts_with(&low)) {
        add(h, Src::History, &mut out);
    }
    for (i, k) in KNOWN.iter().enumerate().filter(|(_, k)| k.0.starts_with(&low)) {
        add(k.0, Src::Known(i), &mut out);
    }
    for p in programs.iter().filter(|p| p.to_lowercase().starts_with(&low)).take(8) {
        add(p, Src::Program, &mut out);
    }
    for h in history.iter().filter(|h| h.to_lowercase().contains(&low)) {
        add(h, Src::History, &mut out);
    }
    if low.chars().count() >= 3 {
        // by what it does: "реестр" → regedit (short text would match half the list)
        for (i, k) in KNOWN.iter().enumerate().filter(|(_, k)| k.0.contains(&low) || k.1.to_lowercase().contains(&low) || k.2.to_lowercase().contains(&low) || k.3.to_lowercase().contains(&low) || k.4.to_lowercase().contains(&low) || k.5.to_lowercase().contains(&low)) {
            add(k.0, Src::Known(i), &mut out);
        }
    }
    if let Some((dir, prefix)) = t.trim_start_matches('"').rsplit_once('\\') {
        let real = expand(&format!("{dir}\\"));
        if let Ok(rd) = std::fs::read_dir(&real) {
            let pl = prefix.to_lowercase();
            let mut v: Vec<(bool, String)> = rd
                .flatten()
                .filter_map(|e| {
                    let n = e.file_name().to_string_lossy().into_owned();
                    let d = e.file_type().is_ok_and(|t| t.is_dir());
                    n.to_lowercase().starts_with(&pl).then(|| (!d, format!("{dir}\\{n}{}", if d { "\\" } else { "" })))
                })
                .collect();
            v.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.to_lowercase().cmp(&b.1.to_lowercase()))); // folders first
            for (_, s) in v {
                add(&s, Src::Folder, &mut out);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_like_the_run_box() {
        let none = |_: &str| false;
        assert_eq!(split_cmd("notepad file.txt", none), ("notepad".into(), "file.txt".into()));
        assert_eq!(split_cmd("  cmd  ", none), ("cmd".into(), "".into()));
        assert_eq!(split_cmd(r#""C:\Program Files\a b.exe" -x -y"#, none), (r"C:\Program Files\a b.exe".into(), "-x -y".into()));
        assert_eq!(split_cmd("https://example.com/a b", none), ("https://example.com/a b".into(), "".into()));
        assert_eq!(split_cmd("ms-settings:display", none), ("ms-settings:display".into(), "".into()));
        assert_eq!(split_cmd("shell:startup", none), ("shell:startup".into(), "".into()));
        // a drive letter is not a scheme: C:\x y is a program and an argument unless the whole path exists
        assert_eq!(split_cmd(r"C:\tools\x.exe y", none), (r"C:\tools\x.exe".into(), "y".into()));
        assert_eq!(split_cmd(r"C:\Program Files", |p| p == r"C:\Program Files"), (r"C:\Program Files".into(), "".into()));
    }

    #[test]
    fn programs_come_from_path_without_exe() {
        let p = programs();
        assert!(p.iter().any(|n| n.eq_ignore_ascii_case("cmd")), "System32 is on PATH");
        assert!(p.iter().all(|n| !n.to_lowercase().ends_with(".exe")));
        assert!(p.windows(2).all(|w| w[0].to_lowercase() < w[1].to_lowercase()), "sorted, no duplicates");
    }

    #[test]
    fn expands_known_variables_only() {
        unsafe { std::env::set_var("RS_RUN_T", "X") };
        assert_eq!(expand("%RS_RUN_T%\\a"), "X\\a");
        assert_eq!(expand("100% %NO_SUCH_VAR_Q% %"), "100% %NO_SUCH_VAR_Q% %");
    }

    #[test]
    fn history_moves_to_top_without_duplicates() {
        let mut h = vec!["cmd".to_string(), "notepad".to_string()];
        remember_in(&mut h, " NOTEPAD ");
        assert_eq!(h, ["NOTEPAD", "cmd"]);
        for i in 0..60 {
            remember_in(&mut h, &i.to_string());
        }
        assert_eq!(h.len(), HISTORY_MAX);
        assert_eq!(h[0], "59");
    }

    #[test]
    fn suggests_text_history_then_folder() {
        assert_eq!(super::suggest(r"c:\no such dir 9q\x", &[], &[])[0].1, Src::Typed);
        let suggest = |t: &str, h: &[String]| super::suggest(t, h, &[]).into_iter().map(|r| r.0).collect::<Vec<_>>();
        let h = vec!["regedit".to_string(), "cmd /k ver".to_string(), "control".to_string()];
        // empty: the history first, topped up to 7 with common commands (no duplicates)
        assert_eq!(suggest("", &h), ["regedit", "cmd /k ver", "control", "cmd", "powershell", "taskmgr", "msconfig"]);
        // history before known commands, prefix matches before "contains"
        assert_eq!(suggest("co", &h), ["co", "control", "compmgmt.msc"]);
        assert_eq!(suggest("ed", &h)[..2], ["ed", "regedit"]);
        // by description, in any of the five languages
        assert_eq!(suggest("реестр", &[]), ["реестр", "regedit"]);
        assert_eq!(suggest("Registry", &[]), ["Registry", "regedit"]);
        assert_eq!(suggest("注册表", &[]), ["注册表", "regedit"]);
        // programs by prefix, marked as such
        assert_eq!(super::suggest("chr", &[], &["chrome".into(), "charmap".into()]), [("chr".into(), Src::Typed), ("chrome".into(), Src::Program)]);
        assert_eq!(describe(" REGEDIT "), Some(2));
        assert!(KNOWN.iter().all(|k| k.0 == k.0.to_lowercase()), "prefix matching compares against lowercase");
        let base = std::env::temp_dir().join("rs-run-test-5e2a");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("Beta")).unwrap();
        std::fs::write(base.join("bravo.txt"), "").unwrap();
        std::fs::write(base.join("alpha.txt"), "").unwrap();
        let typed = format!("{}\\b", base.display());
        assert_eq!(suggest(&typed, &[]), [typed.clone(), format!("{}\\Beta\\", base.display()), format!("{}\\bravo.txt", base.display())]);
        std::fs::remove_dir_all(&base).unwrap();
    }
}
