//! Страница «Ускорение»: профили «Игра»/«Работа», кэш памяти, лишние процессы оболочки и группы debloat
//! (апдейтеры, исключения Defender, NTFS, блокировка рекламы в hosts). Всё обратимо; долгие действия идут в фоновом потоке,
//! результат забирает такт окна (`take_done`).
use crate::sys;
use std::{
    fs,
    os::windows::process::CommandExt,
    path::PathBuf,
    process::Command,
    ptr::null_mut,
    sync::Mutex,
};
use winapi::shared::minwindef::HKEY;
use winapi::um::{winnt::{KEY_READ, REG_SZ}, winreg::*};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Id {
    Game,
    Work,
    MemNow,
    MemAuto,
    Shell,
    Updaters,
    Defender,
    Ntfs,
    Dns,
}

pub const ALL: [Id; 9] = [Id::Game, Id::Work, Id::MemNow, Id::MemAuto, Id::Shell, Id::Updaters, Id::Defender, Id::Ntfs, Id::Dns];

impl Id {
    pub fn key(self) -> &'static str {
        match self {
            Id::Game => "game",
            Id::Work => "work",
            Id::MemNow => "memnow",
            Id::MemAuto => "memauto",
            Id::Shell => "shell",
            Id::Updaters => "updaters",
            Id::Defender => "defender",
            Id::Ntfs => "ntfs",
            Id::Dns => "dns",
        }
    }
    pub fn from_key(k: &str) -> Option<Id> {
        ALL.iter().copied().find(|i| i.key() == k)
    }
    pub fn title(self) -> &'static str {
        match self {
            Id::Game => t!("Профиль «Игра»", "'Game' profile", ja: "「ゲーム」プロファイル", pt: "Perfil 'Jogo'", zh: "「游戏」配置"),
            Id::Work => t!("Профиль «Работа»", "'Work' profile", ja: "「作業」プロファイル", pt: "Perfil 'Trabalho'", zh: "「工作」配置"),
            Id::MemNow => t!("Очистить кэш памяти сейчас", "Clear memory cache now", ja: "メモリ キャッシュを今すぐクリア", pt: "Limpar o cache de memória agora", zh: "立即清理内存缓存"),
            Id::MemAuto => t!("Очищать кэш памяти при нехватке", "Clear memory cache when it runs short", ja: "メモリ不足時にメモリ キャッシュをクリア", pt: "Limpar o cache de memória quando ele ficar escasso", zh: "内存不足时清理内存缓存"),
            Id::Shell => t!("Не запускать лишние процессы оболочки", "Block unneeded shell processes", ja: "不要なシェル プロセスをブロック", pt: "Bloquear processos de shell desnecessários", zh: "阻止不需要的外壳进程"),
            Id::Updaters => t!("Один обновлятор через winget", "One updater through winget", ja: "winget による一括アップデーター", pt: "Um único atualizador via winget", zh: "通过 winget 统一更新"),
            Id::Defender => t!("Исключения Defender для сборок и игр", "Defender exclusions for builds and games", ja: "ビルドとゲーム用の Defender 除外", pt: "Exclusões do Defender para compilações e jogos", zh: "为编译和游戏添加 Defender 排除项"),
            Id::Ntfs => t!("Быстрее NTFS", "Faster NTFS", ja: "高速 NTFS", pt: "NTFS mais rápido", zh: "加速 NTFS"),
            Id::Dns => t!("Блокировать рекламу (hosts)", "Block ads (hosts)", ja: "広告をブロック (hosts)", pt: "Bloquear anúncios (hosts)", zh: "拦截广告（hosts）"),
        }
    }
    pub fn desc(self) -> &'static str {
        match self {
            Id::Game => t!("Останавливает Windows Update, BITS, оптимизацию доставки, печать, телеметрию; закрывает OneDrive, Teams, Dropbox, Google Drive, Phone Link, виджеты. «Выключить» всё запускает обратно.", "Stops Windows Update, BITS, Delivery Optimization, printing, telemetry; closes OneDrive, Teams, Dropbox, Google Drive, Phone Link, widgets. 'Disable' starts everything again.", ja: "Windows Update、BITS、配信の最適化、印刷、テレメトリを停止し、OneDrive、Teams、Dropbox、Google Drive、スマートフォン連携、ウィジェットを終了します。「無効にする」とすべて再開します。", pt: "Para o Windows Update, o BITS, a Otimização de Entrega, a impressão e a telemetria; fecha OneDrive, Teams, Dropbox, Google Drive, Phone Link e widgets. 'Desativar' inicia tudo de novo.", zh: "停止 Windows Update、BITS、传递优化、打印、遥测；关闭 OneDrive、Teams、Dropbox、Google Drive、手机连接、小组件。「禁用」会重新启动所有项目。"),
            Id::Work => t!("Закрывает Steam, Epic, Battle.net, EA, Ubisoft, GOG, Overwolf, Xbox и Game Bar, останавливает их службы. «Выключить» запускает лаунчеры обратно.", "Closes Steam, Epic, Battle.net, EA, Ubisoft, GOG, Overwolf, Xbox and Game Bar, stops their services. 'Disable' starts the launchers again.", ja: "Steam、Epic、Battle.net、EA、Ubisoft、GOG、Overwolf、Xbox、Game Bar を終了し、それらのサービスを停止します。「無効にする」とランチャーを再び起動します。", pt: "Fecha Steam, Epic, Battle.net, EA, Ubisoft, GOG, Overwolf, Xbox e Game Bar e para os serviços deles. 'Desativar' inicia os lançadores de novo.", zh: "关闭 Steam、Epic、Battle.net、EA、Ubisoft、GOG、Overwolf、Xbox 和 Game Bar，并停止其服务。「禁用」会重新启动这些启动器。"),
            Id::MemNow => t!("Сбрасывает список standby (кэш файлов, который никто не держит), как ISLC. Помогает от рывков в играх, когда свободной памяти нет.", "Drops the standby list (cached file data nobody holds), like ISLC. Helps against stutter in games when no memory is free.", ja: "ISLC のように、スタンバイ リスト (誰も使っていないキャッシュ済みのファイル データ) を破棄します。空きメモリがないときのゲームのカクつきに効果があります。", pt: "Descarta a lista de espera (dados de arquivos em cache que ninguém usa), como o ISLC. Ajuda contra engasgos em jogos quando não há memória livre.", zh: "清空备用列表（无人持有的缓存文件数据），类似 ISLC。在没有可用内存时有助于缓解游戏卡顿。"),
            Id::MemAuto => t!("Панель topbar каждые 10 с проверяет память и сбрасывает кэш, если свободно меньше 5% (не меньше 512 МБ), а кэш больше 1 ГБ; не чаще раза в минуту.", "The topbar checks memory every 10 s and drops the cache when less than 5% (at least 512 MB) is free and the cache holds over 1 GB; at most once a minute.", ja: "topbar は 10 秒ごとにメモリを確認し、空きが 5% (最低 512 MB) 未満でキャッシュが 1 GB を超えているときにキャッシュを破棄します。最大で 1 分に 1 回です。", pt: "A topbar verifica a memória a cada 10 s e descarta o cache quando há menos de 5% livre (no mínimo 512 MB) e o cache passa de 1 GB; no máximo uma vez por minuto.", zh: "topbar 每 10 秒检查一次内存，当可用内存低于 5%（至少 512 MB）且缓存超过 1 GB 时清理缓存；每分钟最多一次。"),
            Id::Shell => t!("Меню «Пуск» Windows, виджеты и Связь с телефоном не запускаются (их заменяет topbar). Уведомления и ввод текста не трогаются.", "The Windows Start menu, widgets and Phone Link do not start (topbar replaces them). Notifications and text input are not touched.", ja: "Windows のスタート メニュー、ウィジェット、スマートフォン連携は起動しません (topbar が代わりを務めます)。通知とテキスト入力には影響しません。", pt: "O menu Iniciar do Windows, os widgets e o Phone Link não iniciam (a topbar os substitui). Notificações e entrada de texto não são afetadas.", zh: "Windows 开始菜单、小组件和手机连接不会启动（由 topbar 取代）。通知和文本输入不受影响。"),
            Id::Updaters => t!("Отключает службы и задачи обновления Adobe, Mozilla, Dropbox, CCleaner, Opera, Apple, Zoom, OneDrive; раз в неделю winget обновляет все программы. Браузеры обновляются сами, как раньше.", "Disables updater services and tasks of Adobe, Mozilla, Dropbox, CCleaner, Opera, Apple, Zoom, OneDrive; winget updates every app once a week. Browsers keep updating themselves.", ja: "Adobe、Mozilla、Dropbox、CCleaner、Opera、Apple、Zoom、OneDrive のアップデーター サービスとタスクを無効にし、winget がすべてのアプリを週に 1 回更新します。ブラウザーは引き続き自動更新されます。", pt: "Desativa os serviços e tarefas de atualização de Adobe, Mozilla, Dropbox, CCleaner, Opera, Apple, Zoom e OneDrive; o winget atualiza todos os apps uma vez por semana. Os navegadores continuam se atualizando sozinhos.", zh: "禁用 Adobe、Mozilla、Dropbox、CCleaner、Opera、Apple、Zoom、OneDrive 的更新服务和任务；winget 每周更新一次所有应用。浏览器会自行更新。"),
            Id::Defender => t!("Defender перестаёт проверять кэши cargo, rustup, npm, pip, NuGet, Gradle и библиотеки Steam/Epic, а также cargo.exe и rustc.exe. Сборка и загрузка игр быстрее, но эти папки не сканируются.", "Defender stops scanning the caches of cargo, rustup, npm, pip, NuGet, Gradle and the Steam/Epic libraries, and cargo.exe and rustc.exe. Builds and game loading get faster, but those folders are not scanned.", ja: "Defender は cargo、rustup、npm、pip、NuGet、Gradle、Steam/Epic ライブラリのキャッシュと、cargo.exe および rustc.exe のスキャンを停止します。ビルドとゲームの読み込みが速くなりますが、これらのフォルダーはスキャンされません。", pt: "O Defender deixa de verificar os caches de cargo, rustup, npm, pip, NuGet, Gradle e as bibliotecas do Steam/Epic, além de cargo.exe e rustc.exe. Compilações e carregamento de jogos ficam mais rápidos, mas essas pastas não são verificadas.", zh: "Defender 将不再扫描 cargo、rustup、npm、pip、NuGet、Gradle 的缓存以及 Steam/Epic 库，还有 cargo.exe 和 rustc.exe。编译和游戏加载会更快，但这些文件夹不会被扫描。"),
            Id::Ntfs => t!("NTFS не пишет время последнего доступа при каждом чтении и не создаёт короткие имена 8.3. Действует после перезагрузки.", "NTFS stops writing the last-access time on every read and creating 8.3 short names. Takes effect after a reboot.", ja: "NTFS は読み取りのたびに最終アクセス日時を書き込むことと、8.3 形式の短い名前の作成をやめます。再起動後に有効になります。", pt: "O NTFS para de gravar o horário do último acesso a cada leitura e de criar nomes curtos 8.3. Vale após reiniciar.", zh: "NTFS 不再在每次读取时写入最后访问时间，也不再创建 8.3 短名称。重启后生效。"),
            Id::Dns => t!("Около 170 рекламных и следящих доменов ведут в никуда через файл hosts (домены Microsoft не трогаются).", "About 170 ad and tracker domains lead nowhere through the hosts file (Microsoft domains are left alone).", ja: "約 170 の広告・トラッカーのドメインを hosts ファイルで無効化します (Microsoft のドメインは対象外)。", pt: "Cerca de 170 domínios de anúncios e rastreadores não levam a lugar nenhum pelo arquivo hosts (domínios da Microsoft são preservados).", zh: "hosts 文件让约 170 个广告和跟踪域名无法访问（Microsoft 域名保持原样）。"),
        }
    }
    fn debloat_group(self) -> Option<&'static str> {
        matches!(self, Id::Updaters | Id::Defender | Id::Ntfs | Id::Dns).then(|| self.key())
    }
}

#[derive(Clone, PartialEq, Debug)]
pub enum State {
    On,
    Off,
    Busy,
    Info(String),        // разовое действие: что сейчас
    Unavailable(String), // почему нельзя
}

static BUSY: Mutex<Vec<Id>> = Mutex::new(vec![]);
static DONE: Mutex<Vec<String>> = Mutex::new(vec![]);

pub fn take_done() -> Vec<String> {
    std::mem::take(&mut *DONE.lock().unwrap())
}

fn program_files() -> PathBuf {
    PathBuf::from(std::env::var_os("ProgramFiles").unwrap_or_else(|| r"C:\Program Files".into()))
}

fn program_data() -> PathBuf {
    PathBuf::from(std::env::var_os("ProgramData").unwrap_or_else(|| r"C:\ProgramData".into()))
}

fn topbar_installed() -> bool {
    program_files().join(r"topbar\topbar.exe").is_file()
}

/// debloat.exe: установленный плагин или копия рядом с диспетчером (её кладёт установщик).
fn debloat_exe() -> Option<PathBuf> {
    let near = std::env::current_exe().ok().and_then(|e| Some(e.parent()?.join("debloat.exe")));
    [Some(program_files().join(r"debloat\debloat.exe")), near].into_iter().flatten().find(|p| p.is_file())
}

fn debloat_applied(group: &str) -> bool {
    program_data().join("winrust-debloat").join(format!("undo-{group}.txt")).is_file()
}

pub fn state(id: Id, is_admin: bool) -> State {
    if BUSY.lock().unwrap().contains(&id) {
        return State::Busy;
    }
    if !is_admin {
        return State::Unavailable(t!("нужен администратор", "administrator needed", ja: "管理者権限が必要です", pt: "requer administrador", zh: "需要管理员权限").into());
    }
    let on = |b: bool| if b { State::On } else { State::Off };
    match id {
        Id::Game | Id::Work => on(active_profile() == Some(id)),
        Id::MemNow => match winrust_common::mem::lists() {
            Some(l) => State::Info(tf!("свободно {}, кэш {}", "free {}, cache {}", ja: "空き {}、キャッシュ {}", pt: "livre {}, cache {}", zh: "可用 {}，缓存 {}", crate::ui::fmt_mem(l.free), crate::ui::fmt_mem(l.standby))),
            None => State::Unavailable(t!("нет права SeProfileSingleProcess", "no SeProfileSingleProcess right", ja: "SeProfileSingleProcess 権限がありません", pt: "sem o direito SeProfileSingleProcess", zh: "缺少 SeProfileSingleProcess 权限").into()),
        },
        Id::MemAuto if !topbar_installed() => State::Unavailable(t!("нужна панель topbar", "needs the topbar", ja: "topbar が必要です", pt: "requer a topbar", zh: "需要 topbar").into()),
        Id::MemAuto => on(winrust_common::mem::auto_on()),
        Id::Shell if !topbar_installed() => State::Unavailable(t!("нужна панель topbar (без неё не будет меню «Пуск»)", "needs the topbar (no Start menu without it)", ja: "topbar が必要です (なければスタート メニューがありません)", pt: "requer a topbar (sem ela não há menu Iniciar)", zh: "需要 topbar（没有它就没有开始菜单）").into()),
        Id::Shell => on(shell_blocked()),
        _ => match (id.debloat_group(), debloat_exe()) {
            (Some(g), Some(_)) => on(debloat_applied(g)),
            _ => State::Unavailable(t!("нет debloat.exe (переустановите диспетчер задач)", "no debloat.exe (reinstall the task manager)", ja: "debloat.exe がありません (タスク マネージャーを再インストールしてください)", pt: "sem debloat.exe (reinstale o gerenciador de tarefas)", zh: "找不到 debloat.exe（请重新安装任务管理器）").into()),
        },
    }
}

/// Запускает включение/выключение в фоне. Err — сразу понятно, что нельзя (текст для подсказки).
pub fn start(id: Id, on: bool, donors: Vec<u32>) -> Result<(), String> {
    match state(id, true) {
        State::Busy => return Err(t!("уже выполняется", "already running", ja: "すでに実行中", pt: "já em execução", zh: "已在运行").into()),
        State::Unavailable(why) => return Err(why),
        _ => {}
    }
    if id == Id::MemNow && !on {
        return Err(t!("это разовое действие: выключать нечего", "a one-time action: nothing to disable", ja: "一度きりの操作です: 無効にするものはありません", pt: "uma ação única: nada a desativar", zh: "一次性操作：没有可禁用的项目").into());
    }
    BUSY.lock().unwrap().push(id);
    std::thread::spawn(move || {
        let r = run(id, on, &donors);
        BUSY.lock().unwrap().retain(|&x| x != id);
        let head = id.title();
        DONE.lock().unwrap().push(match r {
            Ok(msg) => format!("{head}: {msg}"),
            Err(e) => tf!("{head}: ошибка — {e}", "{head}: error - {e}", ja: "{head}: エラー - {e}", pt: "{head}: erro - {e}", zh: "{head}：错误——{e}"),
        });
    });
    Ok(())
}

fn run(id: Id, on: bool, donors: &[u32]) -> Result<String, String> {
    match id {
        Id::Game | Id::Work if on => profile_on(id, donors),
        Id::Game | Id::Work => profile_off(donors),
        Id::MemNow => {
            let before = winrust_common::mem::lists();
            winrust_common::mem::purge_standby().map_err(|st| tf!("NTSTATUS 0x{:08X}", "NTSTATUS 0x{:08X}", ja: "NTSTATUS 0x{:08X}", pt: "NTSTATUS 0x{:08X}", zh: "NTSTATUS 0x{:08X}", st as u32))?;
            let freed = match (before, winrust_common::mem::lists()) {
                (Some(a), Some(b)) => b.free.saturating_sub(a.free),
                _ => 0,
            };
            Ok(tf!("освобождено {}", "{} freed", ja: "{} 解放", pt: "{} liberados", zh: "已释放 {}", crate::ui::fmt_mem(freed)))
        }
        Id::MemAuto => {
            let f = winrust_common::mem::auto_file().ok_or("ProgramData?")?;
            if on {
                fs::create_dir_all(f.parent().unwrap()).and_then(|_| fs::write(&f, "1")).map_err(|e| e.to_string())?;
                Ok(t!("включено (работает, пока запущена панель)", "on (works while the bar runs)", ja: "オン (バーの実行中に動作)", pt: "ligado (funciona enquanto a barra estiver em execução)", zh: "开（仅在 topbar 运行时有效）").into())
            } else {
                let _ = fs::remove_file(&f);
                Ok(t!("выключено", "off", ja: "オフ", pt: "desligado", zh: "关").into())
            }
        }
        Id::Shell => shell_set(on),
        _ => {
            let exe = debloat_exe().ok_or("debloat.exe")?;
            let g = id.debloat_group().unwrap();
            let o = Command::new(&exe).args([if on { "--apply" } else { "--undo" }, g]).creation_flags(0x0800_0000).output().map_err(|e| e.to_string())?;
            if o.status.success() {
                Ok(if on { t!("включено", "on", ja: "オン", pt: "ligado", zh: "开") } else { t!("возвращено как было", "restored", ja: "復元しました", pt: "restaurado", zh: "已恢复") }.into())
            } else {
                let err = String::from_utf8_lossy(&o.stderr);
                Err(err.lines().last().unwrap_or("debloat").to_string())
            }
        }
    }
}

/// Возвращает всё включённое здесь (удаление диспетчера). Синхронно; список ошибок.
pub fn undo_all() -> Vec<String> {
    let mut errs = vec![];
    for id in ALL {
        let applied = match id {
            Id::Game => active_profile().is_some(),
            Id::Work | Id::MemNow => false, // один файл профиля на оба; очистка — разовое действие
            Id::MemAuto => winrust_common::mem::auto_on(),
            Id::Shell => SHELL_EXES.iter().any(|e| debugger(e).is_some_and(|d| ours(&d))),
            _ => id.debloat_group().is_some_and(debloat_applied),
        };
        if applied && let Err(e) = run(id, false, &[]) {
            errs.push(format!("{}: {e}", id.title()));
        }
    }
    errs
}

// ---------- профили ----------

/// (имя процесса, аргументы для запуска обратно; None — не запускать: помощник или пакетное приложение, оно встанет само)
type ProcSpec = (&'static str, Option<&'static str>);

const GAME_SVCS: &[&str] = &["wuauserv", "BITS", "DoSvc", "UsoSvc", "Spooler", "WSearch", "DiagTrack", "MapsBroker", "WerSvc"];
const GAME_PROCS: &[ProcSpec] = &[
    ("onedrive.exe", Some("/background")),
    ("teams.exe", None),
    ("ms-teams.exe", None),
    ("dropbox.exe", Some("/systemstartup")),
    ("googledrivefs.exe", Some("")),
    ("phoneexperiencehost.exe", None),
    ("widgets.exe", None),
    ("adobecollabsync.exe", None),
    ("creative cloud.exe", None),
];
const WORK_SVCS: &[&str] = &["Steam Client Service", "XblAuthManager", "XblGameSave", "XboxNetApiSvc", "EABackgroundService", "EpicOnlineServices"];
const WORK_PROCS: &[ProcSpec] = &[
    ("steam.exe", Some("-silent")),
    ("steamwebhelper.exe", None),
    ("epicgameslauncher.exe", Some("-silent")),
    ("epicwebhelper.exe", None),
    ("battle.net.exe", Some("")),
    ("eadesktop.exe", Some("")),
    ("upc.exe", Some("")),
    ("galaxyclient.exe", Some("")),
    ("overwolf.exe", None),
    ("xboxpcapp.exe", None),
    ("gamebar.exe", None),
    ("gamebarftserver.exe", None),
];

fn profile_file() -> PathBuf {
    program_data().join(r"WinRust\profile.txt")
}

/// Файл профиля: первая строка game/work, дальше `svc|<имя>` (остановлено нами) и `run|<команда>` (закрыто нами).
pub fn parse_profile(text: &str) -> Option<(Id, Vec<String>, Vec<String>)> {
    let mut lines = text.lines();
    let id = match lines.next()?.trim() {
        "game" => Id::Game,
        "work" => Id::Work,
        _ => return None,
    };
    let (mut svcs, mut runs) = (vec![], vec![]);
    for l in lines {
        if let Some(s) = l.strip_prefix("svc|") {
            svcs.push(s.to_string());
        } else if let Some(r) = l.strip_prefix("run|") {
            runs.push(r.to_string());
        }
    }
    Some((id, svcs, runs))
}

fn active_profile() -> Option<Id> {
    fs::read_to_string(profile_file()).ok().and_then(|t| parse_profile(&t)).map(|p| p.0)
}

/// Все процессы: (PID, имя в нижнем регистре, путь к exe).
fn processes() -> Vec<(u32, String, Option<String>)> {
    use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};
    let Ok(mut s) = System::new() else { return vec![] };
    s.refresh_processes_specifics(ProcessesToUpdate::All, true, ProcessRefreshKind::nothing().with_exe(UpdateKind::Always));
    s.processes().iter().map(|(pid, p)| (pid.as_u32(), p.name().to_string_lossy().to_lowercase(), p.exe().map(|e| e.display().to_string()))).collect()
}

fn profile_on(id: Id, donors: &[u32]) -> Result<String, String> {
    if active_profile().is_some() {
        profile_off(donors)?;
    }
    let (svcs, procs) = if id == Id::Game { (GAME_SVCS, GAME_PROCS) } else { (WORK_SVCS, WORK_PROCS) };
    let mut text = format!("{}\n", id.key());
    let running: Vec<String> = sys::services().into_iter().filter(|s| s.running).map(|s| s.name).collect();
    let mut stopped = 0;
    for s in svcs {
        if running.iter().any(|r| r.eq_ignore_ascii_case(s)) && sys::service_ctl(s, false).is_ok() {
            text += &format!("svc|{s}\n");
            stopped += 1;
        }
    }
    let mut closed = 0;
    let mut runs: Vec<String> = vec![];
    for (pid, name, exe) in processes() {
        let Some((_, again)) = procs.iter().find(|p| p.0 == name) else { continue };
        if !sys::kill(pid) {
            continue;
        }
        closed += 1;
        if let (Some(args), Some(exe)) = (again, exe) {
            let cmd = if args.is_empty() { format!("\"{exe}\"") } else { format!("\"{exe}\" {args}") };
            if !runs.contains(&cmd) {
                runs.push(cmd);
            }
        }
    }
    for r in &runs {
        text += &format!("run|{r}\n");
    }
    let f = profile_file();
    fs::create_dir_all(f.parent().unwrap()).and_then(|_| fs::write(&f, text)).map_err(|e| e.to_string())?;
    Ok(tf!("остановлено служб: {stopped}, закрыто процессов: {closed}", "{stopped} services stopped, {closed} processes closed", ja: "{stopped} 個のサービスを停止、{closed} 個のプロセスを終了しました", pt: "{stopped} serviços parados, {closed} processos fechados", zh: "已停止 {stopped} 个服务，关闭 {closed} 个进程"))
}

fn profile_off(donors: &[u32]) -> Result<String, String> {
    let f = profile_file();
    let Some((_, svcs, runs)) = fs::read_to_string(&f).ok().and_then(|t| parse_profile(&t)) else {
        return Ok(t!("профиль не был включён", "no profile was on", ja: "オンのプロファイルはありませんでした", pt: "nenhum perfil estava ligado", zh: "当时没有启用任何配置").into());
    };
    let started = svcs.iter().filter(|s| sys::service_ctl(s, true).is_ok()).count();
    let mut launched = 0;
    for r in &runs {
        if sys::run_unelevated(donors, r) {
            launched += 1;
        } else if let Some(exe) = r.strip_prefix('"').and_then(|x| x.split('"').next()) {
            // без донора: программа запускается с обычными правами, но без аргументов
            launched += winrust_common::launch::program(exe) as usize;
        }
    }
    fs::remove_file(&f).map_err(|e| e.to_string())?;
    Ok(tf!("запущено служб: {started}, программ: {launched}", "{started} services and {launched} programs started again", ja: "{started} 個のサービスと {launched} 個のプログラムを再び起動しました", pt: "{started} serviços e {launched} programas iniciados de novo", zh: "已重新启动 {started} 个服务和 {launched} 个程序"))
}

// ---------- процессы оболочки ----------

const SHELL_EXES: [&str; 4] = ["StartMenuExperienceHost.exe", "Widgets.exe", "WidgetService.exe", "PhoneExperienceHost.exe"];
const IFEO: &str = r"SOFTWARE\Microsoft\Windows NT\CurrentVersion\Image File Execution Options";

/// Вместо процесса Windows запускает systray.exe: он ничего не показывает и сразу завершается.
fn noop_debugger() -> String {
    format!(r"{}\System32\systray.exe", std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into()))
}

fn w(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

fn debugger(exe: &str) -> Option<String> {
    let mut k: HKEY = null_mut();
    if unsafe { RegOpenKeyExW(HKEY_LOCAL_MACHINE, w(&format!(r"{IFEO}\{exe}")).as_ptr(), 0, KEY_READ, &mut k) } != 0 {
        return None;
    }
    let mut buf = [0u16; 1024];
    let mut len = (buf.len() * 2) as u32;
    let mut ty = 0u32;
    let rc = unsafe { RegQueryValueExW(k, w("Debugger").as_ptr(), null_mut(), &mut ty, buf.as_mut_ptr() as *mut u8, &mut len) };
    unsafe { RegCloseKey(k) };
    (rc == 0 && ty == REG_SZ).then(|| String::from_utf16_lossy(&buf[..len as usize / 2]).trim_end_matches('\0').to_string())
}

fn ours(v: &str) -> bool {
    v.to_lowercase().ends_with(r"\system32\systray.exe")
}

fn shell_blocked() -> bool {
    SHELL_EXES.iter().all(|e| debugger(e).is_some_and(|d| ours(&d)))
}

fn shell_set(on: bool) -> Result<String, String> {
    let mut n = 0;
    for exe in SHELL_EXES {
        let key = w(&format!(r"{IFEO}\{exe}"));
        let cur = debugger(exe);
        let rc = if on {
            if cur.as_deref().is_some_and(|d| !ours(d)) {
                continue; // чужой Debugger (отладчик, другая программа): не наш
            }
            let v = w(&noop_debugger());
            let rc = unsafe { RegSetKeyValueW(HKEY_LOCAL_MACHINE, key.as_ptr(), w("Debugger").as_ptr(), REG_SZ, v.as_ptr() as *const _, (v.len() * 2) as u32) };
            let _ = Command::new("taskkill.exe").args(["/f", "/im", exe]).creation_flags(0x0800_0000).output();
            rc
        } else if cur.as_deref().is_some_and(ours) {
            unsafe { RegDeleteKeyValueW(HKEY_LOCAL_MACHINE, key.as_ptr(), w("Debugger").as_ptr()) }
        } else {
            continue;
        };
        if rc != 0 {
            return Err(tf!("{exe}: ошибка реестра {rc}", "{exe}: registry error {rc}", ja: "{exe}: レジストリ エラー {rc}", pt: "{exe}: erro de registro {rc}", zh: "{exe}：注册表错误 {rc}"));
        }
        n += 1;
    }
    Ok(if on { tf!("заблокировано процессов: {n}", "{n} processes blocked", ja: "{n} 個のプロセスをブロックしました", pt: "{n} processos bloqueados", zh: "已阻止 {n} 个进程") } else { tf!("разблокировано процессов: {n}", "{n} processes unblocked", ja: "{n} 個のプロセスのブロックを解除しました", pt: "{n} processos desbloqueados", zh: "已解除阻止 {n} 个进程") })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_file_round_trip() {
        let t = "game\nsvc|wuauserv\nrun|\"C:\\Users\\a\\OneDrive.exe\" /background\nsvc|BITS\n";
        let (id, svcs, runs) = parse_profile(t).unwrap();
        assert_eq!(id, Id::Game);
        assert_eq!(svcs, ["wuauserv", "BITS"]);
        assert_eq!(runs, ["\"C:\\Users\\a\\OneDrive.exe\" /background"]);
        assert_eq!(parse_profile("junk\n"), None);
        assert_eq!(parse_profile(""), None);
    }

    #[test]
    fn keys_round_trip_and_lists_are_lower_case() {
        for id in ALL {
            assert_eq!(Id::from_key(id.key()), Some(id));
            assert!(!id.title().is_empty() && !id.desc().is_empty());
        }
        for (p, _) in GAME_PROCS.iter().chain(WORK_PROCS) {
            assert_eq!(*p, p.to_lowercase(), "имена процессов сравниваются в нижнем регистре");
        }
        assert!(ours(r"C:\WINDOWS\System32\systray.exe") && !ours("cmd.exe /c exit"));
    }

    #[test]
    fn needs_admin_without_it() {
        assert!(matches!(state(Id::Game, false), State::Unavailable(_)));
    }
}
