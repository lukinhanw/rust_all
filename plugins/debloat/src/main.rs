// debloat                      show what would change (nothing is modified)
// debloat --apply              apply (admin); the old value of every change is written to the undo file BEFORE the change
// debloat --undo               put every value and service back exactly as it was
// debloat <cmd> <group>        the same for one optional group, each with its own undo file (rtaskmgr's Speed page uses them):
//   updaters   third-party updater services and tasks off, one weekly `winget upgrade --all` task instead (browsers keep theirs)
//   defender   Defender exclusions for build caches and game libraries (opt-in: those folders are no longer scanned)
//   ntfs       NTFS stops updating last-access times and creating 8.3 short names (after a reboot)
//   dns        ad and tracker domains blocked in the hosts file
// debloat --status <group>     prints on/off; exit code 0 = applied (its undo file exists)
//
// Only reversible things. Not touched on purpose: Windows Update, Print Spooler, Xbox/Game services, SysMain, installed apps,
// browser updaters (a browser must patch itself quickly), Microsoft domains in hosts (Defender flags that as hijacking).
// Side effect worth knowing: Edge shows "managed by your organization" while its two policies below are set.
use std::{
    env,
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    process::Command,
};
use winrust_common::{t, tf};

// (service, what it is for). A function, not a `const`: `t!` needs the language at run time.
fn services() -> Vec<(&'static str, &'static str)> {
    vec![
        ("DiagTrack", t!("Подключённые пользовательские возможности и телеметрия", "Connected User Experiences and Telemetry", ja: "接続済みユーザー エクスペリエンスとテレメトリ", pt: "Experiências de Usuário Conectadas e Telemetria", zh: "已连接用户体验和遥测")),
        ("dmwappushservice", t!("Маршрутизация push-сообщений WAP (помощник телеметрии)", "WAP push message routing (telemetry helper)", ja: "WAP プッシュ メッセージのルーティング (テレメトリの補助)", pt: "Roteamento de mensagens push WAP (auxiliar de telemetria)", zh: "WAP 推送消息路由（遥测辅助服务）")),
        ("MapsBroker", t!("загруженные карты", "downloaded maps", ja: "ダウンロードした地図", pt: "mapas baixados", zh: "已下载的地图")),
        ("RetailDemo", t!("режим демонстрации в магазине", "retail demo mode", ja: "小売店デモ モード", pt: "modo de demonstração de varejo", zh: "零售演示模式")),
        ("WerSvc", t!("Отчёты об ошибках Windows", "Windows Error Reporting", ja: "Windows エラー報告", pt: "Relatório de Erros do Windows", zh: "Windows 错误报告")),
        ("Fax", t!("Факс", "Fax", ja: "FAX", pt: "Fax", zh: "传真")),
        ("DPS", t!("Служба политики диагностики (средства устранения неполадок)", "Diagnostic Policy Service (troubleshooters)", ja: "診断ポリシー サービス (トラブルシューティング)", pt: "Serviço de Política de Diagnóstico (solucionadores de problemas)", zh: "诊断策略服务（疑难解答）")),
        ("WdiServiceHost", t!("узел службы диагностики (нужна DPS)", "diagnostic service host (needs DPS)", ja: "診断サービス ホスト (DPS が必要)", pt: "host do serviço de diagnóstico (precisa do DPS)", zh: "诊断服务主机（需要 DPS）")),
        ("WdiSystemHost", t!("системный узел диагностики (нужна DPS)", "diagnostic system host (needs DPS)", ja: "診断システム ホスト (DPS が必要)", pt: "host do sistema de diagnóstico (precisa do DPS)", zh: "诊断系统主机（需要 DPS）")),
        ("whesvc", t!("Работоспособность и оптимизированные возможности Windows", "Windows Health and Optimized Experiences", ja: "Windows の正常性と最適化されたエクスペリエンス", pt: "Saúde e Experiências Otimizadas do Windows", zh: "Windows 运行状况和优化体验")),
        ("TrkWks", t!("распределённое отслеживание ссылок", "distributed link tracking", ja: "分散リンク トラッキング", pt: "rastreamento de links distribuído", zh: "分布式链接跟踪")),
        ("WSAIFabricSvc", t!("Windows AI Fabric (возможности Copilot+/Recall)", "Windows AI Fabric (Copilot+/Recall features)", ja: "Windows AI Fabric (Copilot+/Recall の機能)", pt: "Windows AI Fabric (recursos do Copilot+/Recall)", zh: "Windows AI Fabric（Copilot+/Recall 功能）")),
        // InventorySvc, lfsvc: rust-explorer's install already disables them with its own backup; PcaSvc stays on (older games)
    ]
}

// (key, value, DWORD data, what it does). A function, not a `const`: `t!` needs the language at run time.
fn regs() -> Vec<(&'static str, &'static str, u32, &'static str)> {
    vec![
        (r"HKLM\SOFTWARE\Policies\Microsoft\Windows\DataCollection", "AllowTelemetry", 0, t!("телеметрия на минимуме", "telemetry to the minimum", ja: "テレメトリを最小限に", pt: "telemetria no mínimo", zh: "遥测降到最低")),
        (r"HKLM\SOFTWARE\Policies\Microsoft\Windows\DataCollection", "DoNotShowFeedbackNotifications", 1, t!("без всплывающих запросов отзывов", "no feedback pop-ups", ja: "フィードバックのポップアップなし", pt: "sem pop-ups de comentários", zh: "不弹出反馈请求")),
        (r"HKLM\SOFTWARE\Policies\Microsoft\Windows\WindowsCopilot", "TurnOffWindowsCopilot", 1, t!("Copilot выключен", "Copilot off", ja: "Copilot を無効化", pt: "Copilot desligado", zh: "关闭 Copilot")),
        (r"HKLM\SOFTWARE\Policies\Microsoft\Windows\WindowsAI", "DisableAIDataAnalysis", 1, t!("Recall: без снимков экрана", "Recall: no snapshots of your screen", ja: "Recall: 画面のスナップショットを保存しない", pt: "Recall: sem capturas da tela", zh: "Recall：不截取屏幕快照")),
        (r"HKLM\SOFTWARE\Policies\Microsoft\Windows\CloudContent", "DisableWindowsConsumerFeatures", 1, t!("без предложенных и навязанных приложений", "no suggested/promoted apps", ja: "おすすめアプリやプロモーション アプリなし", pt: "sem apps sugeridos/promovidos", zh: "不推荐或预装推广应用")),
        (r"HKLM\SOFTWARE\Policies\Microsoft\Windows\CloudContent", "DisableCloudOptimizedContent", 1, t!("без контента из облака", "no cloud-driven content", ja: "クラウド配信のコンテンツなし", pt: "sem conteúdo da nuvem", zh: "不加载云端内容")),
        (r"HKLM\SOFTWARE\Policies\Microsoft\Windows\CloudContent", "DisableSoftLanding", 1, t!("без советов и подсказок Windows", "no Windows tips and tricks", ja: "Windows のヒントなし", pt: "sem dicas do Windows", zh: "不显示 Windows 提示和技巧")),
        (r"HKLM\SOFTWARE\Policies\Microsoft\Windows\AdvertisingInfo", "DisabledByGroupPolicy", 1, t!("без рекламного идентификатора", "no advertising ID", ja: "広告 ID なし", pt: "sem ID de publicidade", zh: "不使用广告 ID")),
        (r"HKLM\SOFTWARE\Policies\Microsoft\Windows\System", "EnableActivityFeed", 0, t!("без истории действий (временная шкала)", "no activity history (Timeline)", ja: "アクティビティ履歴 (タイムライン) なし", pt: "sem histórico de atividades (Linha do Tempo)", zh: "无活动历史记录（时间线）")),
        (r"HKLM\SOFTWARE\Policies\Microsoft\Windows\System", "PublishUserActivities", 0, t!("история действий остаётся локальной", "activity history stays local", ja: "アクティビティ履歴をローカルにのみ保持", pt: "histórico de atividades fica local", zh: "活动历史仅保存在本地")),
        (r"HKLM\SOFTWARE\Policies\Microsoft\Windows\GameDVR", "AllowGameDVR", 0, t!("фоновая запись Game Bar выключена", "Game Bar background capture off", ja: "Game Bar のバックグラウンド録画を無効化", pt: "captura em segundo plano do Game Bar desligada", zh: "关闭 Game Bar 后台录制")),
        (r"HKLM\SOFTWARE\Policies\Microsoft\Edge", "StartupBoostEnabled", 0, t!("Edge не подгружается при входе", "Edge does not preload at sign-in", ja: "サインイン時に Edge を先読みしない", pt: "o Edge não é pré-carregado ao entrar", zh: "登录时不预加载 Edge")),
        (r"HKLM\SOFTWARE\Policies\Microsoft\Edge", "BackgroundModeEnabled", 0, t!("Edge не работает после закрытия", "Edge does not run after it is closed", ja: "閉じた後に Edge を実行しない", pt: "o Edge não continua após fechar", zh: "关闭 Edge 后不后台运行")),
        (r"HKCU\Software\Policies\Microsoft\Windows\Explorer", "DisableSearchBoxSuggestions", 1, t!("без веб-результатов в поиске", "no web results in search", ja: "検索に Web の結果を出さない", pt: "sem resultados da web na pesquisa", zh: "搜索中不显示网络结果")),
        (r"HKCU\Software\Microsoft\Windows\CurrentVersion\ContentDeliveryManager", "SilentInstalledAppsEnabled", 0, t!("без приложений, установленных без спроса", "no apps installed behind your back", ja: "無断でインストールされるアプリなし", pt: "sem apps instalados pelas suas costas", zh: "不在后台偷偷安装应用")),
        (r"HKCU\Software\Microsoft\Windows\CurrentVersion\ContentDeliveryManager", "PreInstalledAppsEnabled", 0, t!("без предустановленных навязанных приложений", "no pre-installed promoted apps", ja: "プリインストールされたプロモーション アプリなし", pt: "sem apps promovidos pré-instalados", zh: "不预装推广应用")),
        (r"HKCU\Software\Microsoft\Windows\CurrentVersion\ContentDeliveryManager", "OemPreInstalledAppsEnabled", 0, t!("без навязанных приложений от OEM", "no OEM promoted apps", ja: "OEM のプロモーション アプリなし", pt: "sem apps promovidos do OEM", zh: "不预装 OEM 推广应用")),
        (r"HKCU\Software\Microsoft\Windows\CurrentVersion\ContentDeliveryManager", "SystemPaneSuggestionsEnabled", 0, t!("без рекламы в меню «Пуск»", "no ads in Start", ja: "スタート メニューの広告なし", pt: "sem anúncios no menu Iniciar", zh: "开始菜单无广告")),
        (r"HKCU\Software\Microsoft\Windows\CurrentVersion\ContentDeliveryManager", "SubscribedContent-338388Value", 0, t!("без предложений в меню «Пуск»", "no suggestions in Start", ja: "スタート メニューのおすすめなし", pt: "sem sugestões no menu Iniciar", zh: "开始菜单无建议")),
        (r"HKCU\Software\Microsoft\Windows\CurrentVersion\ContentDeliveryManager", "SubscribedContent-338389Value", 0, t!("без советов и предложений", "no tips and suggestions", ja: "ヒントやおすすめなし", pt: "sem dicas e sugestões", zh: "无提示和建议")),
        (r"HKCU\Software\Microsoft\Windows\CurrentVersion\ContentDeliveryManager", "SubscribedContent-353694Value", 0, t!("без предложений в Параметрах", "no suggestions in Settings", ja: "設定のおすすめなし", pt: "sem sugestões nas Configurações", zh: "设置中无建议")),
        (r"HKCU\Software\Microsoft\Windows\CurrentVersion\ContentDeliveryManager", "SubscribedContent-353696Value", 0, t!("без предложений в Параметрах", "no suggestions in Settings", ja: "設定のおすすめなし", pt: "sem sugestões nas Configurações", zh: "设置中无建议")),
        (r"HKCU\Software\Microsoft\Windows\CurrentVersion\ContentDeliveryManager", "RotatingLockScreenOverlayEnabled", 0, t!("без рекламы на экране блокировки", "no ads on the lock screen", ja: "ロック画面の広告なし", pt: "sem anúncios na tela de bloqueio", zh: "锁屏界面无广告")),
    ]
}

// ---------- optional groups ----------

/// Updater services of non-browser apps; winget updates the apps instead.
fn upd_services() -> Vec<(&'static str, &'static str)> {
    vec![
        ("AdobeARMservice", t!("Обновлятор Adobe Acrobat", "Adobe Acrobat updater", ja: "Adobe Acrobat のアップデーター", pt: "Atualizador do Adobe Acrobat", zh: "Adobe Acrobat 更新程序")),
        ("AdobeUpdateService", t!("Обновлятор Adobe", "Adobe updater", ja: "Adobe のアップデーター", pt: "Atualizador da Adobe", zh: "Adobe 更新程序")),
        ("AGSService", t!("Проверка подлинности Adobe", "Adobe Genuine Software check", ja: "Adobe 正規品チェック", pt: "Verificação de software genuíno da Adobe", zh: "Adobe 正版软件检查")),
        ("AGMService", t!("Монитор подлинности Adobe", "Adobe Genuine Monitor", ja: "Adobe 正規品モニター", pt: "Monitor de genuinidade da Adobe", zh: "Adobe 正版监视器")),
        ("MozillaMaintenance", t!("Обслуживание Mozilla (Firefox обновляется сам, пока запущен)", "Mozilla maintenance (Firefox still updates itself while it runs)", ja: "Mozilla メンテナンス (Firefox は実行中に自身を更新します)", pt: "Manutenção da Mozilla (o Firefox ainda se atualiza enquanto está aberto)", zh: "Mozilla 维护服务（Firefox 运行时仍会自行更新）")),
        ("dbupdate", t!("Обновлятор Dropbox", "Dropbox updater", ja: "Dropbox のアップデーター", pt: "Atualizador do Dropbox", zh: "Dropbox 更新程序")),
        ("dbupdatem", t!("Обновлятор Dropbox", "Dropbox updater", ja: "Dropbox のアップデーター", pt: "Atualizador do Dropbox", zh: "Dropbox 更新程序")),
        ("ccleaner", t!("Обновлятор CCleaner", "CCleaner updater", ja: "CCleaner のアップデーター", pt: "Atualizador do CCleaner", zh: "CCleaner 更新程序")),
        ("ccleanerm", t!("Обновлятор CCleaner", "CCleaner updater", ja: "CCleaner のアップデーター", pt: "Atualizador do CCleaner", zh: "CCleaner 更新程序")),
    ]
}

/// Scheduled tasks whose path starts with one of these (case ignored).
const UPD_TASKS: &[&str] = &[
    r"\Adobe Acrobat Update Task",
    r"\AdobeGCInvoker",
    r"\Mozilla\Firefox Background Update",
    r"\Mozilla\Firefox Default Browser Agent",
    r"\DropboxUpdateTaskMachine",
    r"\CCleaner Update",
    r"\CCleanerSkipUAC",
    r"\Opera scheduled Autoupdate",
    r"\Opera GX scheduled Autoupdate",
    r"\Opera scheduled assistant Autoupdate",
    r"\AppleSoftwareUpdate",
    r"\OneDrive Standalone Update Task",
    r"\OneDrive Reporting Task",
    r"\ZoomUpdateTask",
];

const WINGET_TASK: &str = r"\WinRust\Update apps";

// (key, value, DWORD data, what it does). A function, not a `const`: `t!` needs the language at run time.
fn ntfs_regs() -> Vec<(&'static str, &'static str, u32, &'static str)> {
    vec![
        (r"HKLM\SYSTEM\CurrentControlSet\Control\FileSystem", "NtfsDisableLastAccessUpdate", 0x8000_0001, t!("время последнего доступа не пишется при каждом чтении", "no last-access time written on every read", ja: "読み取りのたびに最終アクセス日時を書き込まない", pt: "o horário do último acesso não é gravado a cada leitura", zh: "每次读取时不写入最后访问时间")),
        (r"HKLM\SYSTEM\CurrentControlSet\Control\FileSystem", "NtfsDisable8dot3NameCreation", 1, t!("короткие имена 8.3 не создаются для новых файлов", "no 8.3 short name created with every new file", ja: "新しいファイルごとに 8.3 形式の短い名前を作成しない", pt: "nenhum nome curto 8.3 é criado para cada novo arquivo", zh: "不为每个新文件创建 8.3 短名称")),
    ]
}

const DEF_PROCS: &[&str] = &["cargo.exe", "rustc.exe"];

const HOSTS_BEGIN: &str = "# >>> WinRust DNS block >>>";
const HOSTS_END: &str = "# <<< WinRust DNS block <<<";

/// Ad and tracker hosts. No Microsoft domains: Defender reports hosts entries for them as SettingsModifier:Win32/HostsFileHijack.
const BLOCK: &[&str] = &[
    "doubleclick.net", "ad.doubleclick.net", "static.doubleclick.net", "stats.g.doubleclick.net", "googleads.g.doubleclick.net",
    "pubads.g.doubleclick.net", "securepubads.g.doubleclick.net", "cm.g.doubleclick.net", "googleadservices.com", "www.googleadservices.com",
    "pagead2.googlesyndication.com", "tpc.googlesyndication.com", "adservice.google.com", "partner.googleadservices.com",
    "google-analytics.com", "www.google-analytics.com", "ssl.google-analytics.com", "analytics.google.com", "googletagservices.com",
    "www.googletagservices.com", "app-measurement.com", "adnxs.com", "ib.adnxs.com", "secure.adnxs.com", "acdn.adnxs.com",
    "ads.yahoo.com", "analytics.yahoo.com", "advertising.com", "ads.pubmatic.com", "image6.pubmatic.com", "simage2.pubmatic.com",
    "ads.rubiconproject.com", "fastlane.rubiconproject.com", "pixel.rubiconproject.com", "openx.net", "us-ads.openx.net",
    "rtb.openx.net", "criteo.com", "static.criteo.net", "bidder.criteo.com", "dis.criteo.com", "taboola.com", "cdn.taboola.com",
    "trc.taboola.com", "outbrain.com", "widgets.outbrain.com", "log.outbrain.com", "amazon-adsystem.com", "aax.amazon-adsystem.com",
    "c.amazon-adsystem.com", "s.amazon-adsystem.com", "adsrvr.org", "match.adsrvr.org", "insight.adsrvr.org", "casalemedia.com",
    "as.casalemedia.com", "smartadserver.com", "www.smartadserver.com", "moatads.com", "z.moatads.com", "px.moatads.com",
    "scorecardresearch.com", "sb.scorecardresearch.com", "b.scorecardresearch.com", "quantserve.com", "pixel.quantserve.com",
    "quantcount.com", "hotjar.com", "static.hotjar.com", "script.hotjar.com", "vars.hotjar.com", "mouseflow.com", "cdn.mouseflow.com",
    "crazyegg.com", "script.crazyegg.com", "mixpanel.com", "api.mixpanel.com", "cdn.mxpnl.com", "segment.io", "api.segment.io",
    "cdn.segment.com", "amplitude.com", "api.amplitude.com", "api2.amplitude.com", "branch.io", "api2.branch.io", "app.adjust.com",
    "adjust.com", "appsflyer.com", "t.appsflyer.com", "inapps.appsflyer.com", "adcolony.com", "ads30.adcolony.com", "unityads.unity3d.com",
    "auction.unityads.unity3d.com", "applovin.com", "ms.applovin.com", "rt.applovin.com", "mopub.com", "ads.mopub.com", "inmobi.com",
    "chartboost.com", "live.chartboost.com", "vungle.com", "api.vungle.com", "yieldmo.com", "sharethrough.com", "btlr.sharethrough.com",
    "33across.com", "ssc.33across.com", "media.net", "contextual.media.net", "zedo.com", "revcontent.com", "trends.revcontent.com",
    "mgid.com", "servicer.mgid.com", "propellerads.com", "popads.net", "popcash.net", "exoclick.com", "syndication.exoclick.com",
    "adform.net", "track.adform.net", "serving-sys.com", "bs.serving-sys.com", "flashtalking.com", "servedby.flashtalking.com",
    "bluekai.com", "tags.bluekai.com", "krxd.net", "cdn.krxd.net", "demdex.net", "dpm.demdex.net", "everesttech.net",
    "omtrdc.net", "2o7.net", "tapad.com", "addthis.com", "s7.addthis.com", "sharethis.com", "w.sharethis.com", "ads.tiktok.com",
    "analytics.tiktok.com", "ads-api.tiktok.com", "an.facebook.com", "pixel.facebook.com", "ads.linkedin.com", "px.ads.linkedin.com",
    "ads.twitter.com", "static.ads-twitter.com", "ads-api.twitter.com", "analytics.twitter.com", "ads.reddit.com", "events.reddit.com",
    "ads.pinterest.com", "ct.pinterest.com", "an.yandex.ru", "mc.yandex.ru", "adfox.yandex.ru", "ads.adfox.ru", "top-fwz1.mail.ru",
    "ad.mail.ru", "rs.mail.ru", "counter.yadro.ru", "ssp.rambler.ru", "kraken.rambler.ru",
];

#[derive(Clone, Copy, PartialEq, Debug)]
enum Group {
    Base,
    Updaters,
    Defender,
    Ntfs,
    Dns,
}

impl Group {
    fn parse(s: &str) -> Option<Group> {
        Some(match s {
            "updaters" => Group::Updaters,
            "defender" => Group::Defender,
            "ntfs" => Group::Ntfs,
            "dns" => Group::Dns,
            _ => return None,
        })
    }
    fn file(self) -> &'static str {
        match self {
            Group::Base => "undo.txt",
            Group::Updaters => "undo-updaters.txt",
            Group::Defender => "undo-defender.txt",
            Group::Ntfs => "undo-ntfs.txt",
            Group::Dns => "undo-dns.txt",
        }
    }
}

fn run(prog: &str, args: &[&str]) -> (bool, String) {
    // HKCU = the signed-in user's when the installer was elevated as another administrator (WINRUST_SID)
    let args: Vec<String> = if prog == "reg.exe" { winrust_common::reg::user_args(args) } else { args.iter().map(|a| a.to_string()).collect() };
    match Command::new(prog).args(&args).output() {
        Ok(o) => (o.status.success(), String::from_utf8_lossy(&o.stdout).into_owned()),
        Err(e) => (false, e.to_string()),
    }
}

fn ps(script: &str) -> (bool, String) {
    run("powershell.exe", &["-NoProfile", "-NonInteractive", "-Command", script])
}

/// A PowerShell single-quoted string.
fn psq(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/// Current start mode as `sc config` spells it. None = service missing, or boot/system driver (never touched).
fn start_mode(svc: &str) -> Option<&'static str> {
    let (ok, out) = run("sc.exe", &["qc", svc]);
    let line = out.lines().find(|l| ok && l.contains("START_TYPE"))?;
    Some(if line.contains("DISABLED") {
        "disabled"
    } else if line.contains("DEMAND_START") {
        "demand"
    } else if line.contains("AUTO_START") {
        if line.contains("DELAYED") { "delayed-auto" } else { "auto" }
    } else {
        return None;
    })
}

#[derive(Debug, PartialEq)]
enum Val {
    Missing,
    Dword(u32),
    Other, // a value of another type: not ours to overwrite
}

/// Reads `reg query /v` output.
fn parse_val(ok: bool, out: &str) -> Val {
    if !ok {
        return Val::Missing;
    }
    match winrust_common::reg::value(out) {
        Some(_) => winrust_common::reg::dword(out).map_or(Val::Other, Val::Dword),
        None => Val::Missing,
    }
}

fn reg_val(key: &str, value: &str) -> Val {
    let (ok, out) = run("reg.exe", &["query", key, "/v", value]);
    parse_val(ok, &out)
}

fn undo_path(g: Group) -> PathBuf {
    PathBuf::from(env::var("ProgramData").unwrap_or_else(|_| r"C:\ProgramData".into())).join("winrust-debloat").join(g.file())
}

fn hosts_path() -> PathBuf {
    PathBuf::from(env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into())).join(r"System32\drivers\etc\hosts")
}

/// Task paths (`\Folder\Name`) from `schtasks /query /fo csv /nh`: the first column, quoted.
fn parse_task_names(csv: &str) -> Vec<String> {
    let mut v: Vec<String> = csv.lines().filter_map(|l| l.strip_prefix('"')?.split('"').next().map(str::to_string)).filter(|n| n.starts_with('\\')).collect();
    v.dedup();
    v
}

fn updater_tasks() -> Vec<String> {
    let (_, out) = run("schtasks.exe", &["/query", "/fo", "csv", "/nh"]);
    parse_task_names(&out).into_iter().filter(|n| UPD_TASKS.iter().any(|p| n.to_lowercase().starts_with(&p.to_lowercase()))).collect()
}

/// The task's own Enabled flag (inside <Settings>; triggers carry their own flags).
fn task_enabled(name: &str) -> Option<bool> {
    let (ok, xml) = run("schtasks.exe", &["/query", "/tn", name, "/xml"]);
    if !ok {
        return None;
    }
    let settings = xml.split("<Settings>").nth(1)?.split("</Settings>").next()?;
    Some(!settings.contains("<Enabled>false</Enabled>"))
}

fn has_winget() -> bool {
    run("where.exe", &["winget"]).0
}

/// Folders where a build or a game reads thousands of files: caches of cargo, rustup, npm, pip, NuGet, and game libraries.
fn defender_paths() -> Vec<String> {
    let mut v = vec![];
    if let Some(home) = env::var_os("USERPROFILE").map(PathBuf::from) {
        for d in [".cargo", ".rustup", r".nuget\packages", r".gradle\caches", r".m2\repository"] {
            v.push(home.join(d));
        }
    }
    if let Some(la) = env::var_os("LOCALAPPDATA").map(PathBuf::from) {
        v.push(la.join("npm-cache"));
        v.push(la.join(r"pip\cache"));
        v.push(la.join(r"Yarn\Cache"));
        v.push(la.join(r"pnpm\store"));
    }
    if let Some(pf) = env::var_os("ProgramFiles").map(PathBuf::from) {
        v.push(pf.join("Epic Games"));
    }
    v.extend(steam_libraries());
    // SteamPath is stored with forward slashes and in lower case: one spelling per folder
    let mut seen = std::collections::HashSet::new();
    v.into_iter().filter(|p| p.is_dir()).map(|p| p.display().to_string().replace('/', "\\")).filter(|p| seen.insert(p.to_lowercase())).collect()
}

fn steam_libraries() -> Vec<PathBuf> {
    let (_, out) = run("reg.exe", &["query", r"HKCU\Software\Valve\Steam", "/v", "SteamPath"]);
    let Some(("REG_SZ", root)) = winrust_common::reg::value(&out) else { return vec![] };
    let vdf = fs::read_to_string(Path::new(root).join(r"steamapps\libraryfolders.vdf")).unwrap_or_default();
    let mut libs = vec![PathBuf::from(root)];
    libs.extend(parse_vdf_paths(&vdf).into_iter().map(PathBuf::from));
    libs.into_iter().map(|l| l.join(r"steamapps\common")).collect()
}

/// `"path"		"D:\\SteamLibrary"` lines of libraryfolders.vdf (backslashes are escaped there).
fn parse_vdf_paths(vdf: &str) -> Vec<String> {
    vdf.lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split('"').collect();
            (f.len() >= 4 && f[1] == "path").then(|| f[3].replace(r"\\", r"\"))
        })
        .collect()
}

/// Current Defender exclusions (paths and processes), lower case. Err when Defender is not running (another antivirus).
fn defender_exclusions() -> Result<Vec<String>, String> {
    let (ok, out) = ps("$p=Get-MpPreference -ErrorAction Stop; $p.ExclusionPath; $p.ExclusionProcess");
    if !ok {
        return Err(t!("Microsoft Defender здесь не активен (другой антивирус?): исключать нечего", "Microsoft Defender is not active here (another antivirus?): nothing to exclude", ja: "ここでは Microsoft Defender が動作していません (別のウイルス対策ソフト?)。除外するものはありません", pt: "O Microsoft Defender não está ativo aqui (outro antivírus?): nada a excluir", zh: "此处 Microsoft Defender 未启用（装了其他杀毒软件？）：没有可添加的排除项").into());
    }
    Ok(out.lines().map(|l| l.trim().to_lowercase()).filter(|l| !l.is_empty()).collect())
}

fn hosts_block() -> String {
    let mut s = format!("{HOSTS_BEGIN}\r\n");
    for d in BLOCK {
        s += &format!("0.0.0.0 {d}\r\n");
    }
    s + HOSTS_END + "\r\n"
}

/// The hosts text without our block (and without the empty line we put before it).
fn strip_block(text: &str) -> String {
    let (Some(a), Some(b)) = (text.find(HOSTS_BEGIN), text.find(HOSTS_END)) else { return text.to_string() };
    if b < a {
        return text.to_string();
    }
    let mut end = b + HOSTS_END.len();
    let rest = &text[end..];
    end += if rest.starts_with("\r\n") { 2 } else { rest.starts_with('\n') as usize };
    let head = text[..a].trim_end_matches(['\r', '\n']);
    let tail = &text[end..];
    if head.is_empty() { tail.to_string() } else { format!("{head}\r\n{tail}") }
}

fn plan(g: Group) {
    println!("{}", t!("Пробный запуск: ничего не изменяется. Добавьте --apply (нужны права администратора), чтобы применить, или --undo, чтобы откатить.\n", "Dry run: nothing is changed. Add --apply (admin) to do it, --undo to revert.\n", ja: "ドライ ラン: 何も変更されません。--apply (管理者) で適用、--undo で元に戻します。\n", pt: "Simulação: nada é alterado. Adicione --apply (admin) para aplicar ou --undo para reverter.\n", zh: "试运行：不会更改任何内容。加上 --apply（管理员）即可应用，--undo 则回滚。\n"));
    let (svcs, regs): (Vec<(&str, &str)>, Vec<(&str, &str, u32, &str)>) = match g {
        Group::Base => (services(), regs()),
        Group::Updaters => (upd_services(), vec![]),
        Group::Ntfs => (vec![], ntfs_regs()),
        _ => (vec![], vec![]),
    };
    for (svc, why) in svcs {
        match start_mode(svc) {
            None => println!("{}", tf!("  служба {0:<18} отсутствует", "  service {0:<18} not present", ja: "  サービス {0:<18} が見つかりません", pt: "  serviço {0:<18} não existe", zh: "  服务 {0:<18} 不存在", svc)),
            Some("disabled") => println!("{}", tf!("  служба {0:<18} уже отключена", "  service {0:<18} already disabled", ja: "  サービス {0:<18} はすでに無効です", pt: "  serviço {0:<18} já está desativado", zh: "  服务 {0:<18} 已禁用", svc)),
            Some(m) => println!("{}", tf!("  служба {0:<18} {1} -> disabled   ({2})", "  service {0:<18} {1} -> disabled   ({2})", ja: "  サービス {0:<18} {1} -> disabled   ({2})", pt: "  serviço {0:<18} {1} -> disabled   ({2})", zh: "  服务 {0:<18} {1} -> disabled   ({2})", svc, m, why)),
        }
    }
    for (key, value, data, why) in regs {
        let state = match reg_val(key, value) {
            Val::Dword(d) if d == data => t!("уже установлено", "already set", ja: "設定済み", pt: "já definido", zh: "已设置").to_string(),
            Val::Dword(d) => tf!("{d} -> {data}", "{d} -> {data}", ja: "{d} -> {data}", pt: "{d} -> {data}", zh: "{d} -> {data}"),
            Val::Missing => tf!("(нет) -> {data}", "(none) -> {data}", ja: "(なし) -> {data}", pt: "(nenhum) -> {data}", zh: "（无） -> {data}"),
            Val::Other => t!("не DWORD, не трогаем", "not a DWORD, left alone", ja: "DWORD ではないため変更しません", pt: "não é DWORD, mantido", zh: "不是 DWORD，保持不变").into(),
        };
        println!("{}", tf!("  политика  {0:<34} {1}   ({2})", "  policy  {0:<34} {1}   ({2})", ja: "  ポリシー  {0:<34} {1}   ({2})", pt: "  política  {0:<34} {1}   ({2})", zh: "  策略    {0:<34} {1}   ({2})", value, state, why));
    }
    match g {
        Group::Updaters => {
            for t in updater_tasks() {
                println!("{}", tf!("  задача  {0}   -> disabled", "  task    {0}   -> disabled", ja: "  タスク  {0}   -> disabled", pt: "  tarefa  {0}   -> disabled", zh: "  任务    {0}   -> disabled", t));
            }
            let winget = if has_winget() {
                t!("еженедельно winget upgrade --all", "weekly winget upgrade --all", ja: "毎週 winget upgrade --all", pt: "semanalmente winget upgrade --all", zh: "每周 winget upgrade --all")
            } else {
                t!("пропущено: winget не установлен", "skipped: winget is not installed", ja: "スキップ: winget がインストールされていません", pt: "ignorado: o winget não está instalado", zh: "跳过：未安装 winget")
            };
            println!("{}", tf!("  задача  {0}   {1}", "  task    {0}   {1}", ja: "  タスク  {0}   {1}", pt: "  tarefa  {0}   {1}", zh: "  任务    {0}   {1}", WINGET_TASK, winget));
        }
        Group::Defender => {
            for p in defender_paths() {
                println!("{}", tf!("  исключить {0}", "  exclude {0}", ja: "  除外 {0}", pt: "  excluir {0}", zh: "  排除 {0}", p));
            }
            for p in DEF_PROCS {
                println!("{}", tf!("  исключить процесс {0}", "  exclude process {0}", ja: "  除外プロセス {0}", pt: "  excluir processo {0}", zh: "  排除进程 {0}", p));
            }
        }
        Group::Dns => println!("{}", tf!("  hosts   {0} рекламных и следящих доменов -> 0.0.0.0", "  hosts   {0} ad/tracker domains -> 0.0.0.0", ja: "  hosts   {0} 個の広告・トラッカー ドメイン -> 0.0.0.0", pt: "  hosts   {0} domínios de anúncios e rastreadores -> 0.0.0.0", zh: "  hosts   {0} 个广告和跟踪域名 -> 0.0.0.0", BLOCK.len())),
        _ => {}
    }
}

struct Log(File);

impl Log {
    fn line(&mut self, s: &str) -> Result<(), String> {
        writeln!(self.0, "{s}").and_then(|_| self.0.flush()).map_err(|e| e.to_string())
    }
}

/// The left-margin marker of an apply line: six columns in every language, so the text after it starts in the same column.
fn done(ok: bool) -> &'static str {
    if ok {
        t!("готово", "done  ", ja: "完了  ", pt: "pronto", zh: "完成  ")
    } else {
        t!("ОШИБКА", "FAILED", ja: "失敗  ", pt: "FALHOU", zh: "失败  ")
    }
}

fn apply_services(list: &[(&str, &str)], log: &mut Log) -> Result<u32, String> {
    let mut failed = 0;
    for (svc, _) in list {
        match start_mode(svc) {
            None => println!("{}", tf!("пропуск {0}: не найдена", "skip   {0}: not present", ja: "スキップ {0}: 見つかりません", pt: "ignorar {0}: não existe", zh: "跳过   {0}：不存在", svc)),
            Some("disabled") => println!("{}", tf!("пропуск {0}: уже отключена", "skip   {0}: already disabled", ja: "スキップ {0}: すでに無効です", pt: "ignorar {0}: já está desativado", zh: "跳过   {0}：已禁用", svc)),
            Some(mode) => {
                log.line(&format!("svc|{svc}|{mode}"))?; // log BEFORE changing
                let ok = run("sc.exe", &["config", svc, "start=", "disabled"]).0;
                let _ = run("sc.exe", &["stop", svc]);
                failed += !ok as u32;
                println!("{}", tf!("{0}  {1}: {2} -> disabled", "{0}  {1}: {2} -> disabled", ja: "{0}  {1}: {2} -> disabled", pt: "{0}  {1}: {2} -> disabled", zh: "{0}  {1}: {2} -> disabled", done(ok), svc, mode));
            }
        }
    }
    Ok(failed)
}

fn apply_regs(list: &[(&str, &str, u32, &str)], log: &mut Log) -> Result<u32, String> {
    let mut failed = 0;
    for (key, value, data, _) in list {
        let old = match reg_val(key, value) {
            Val::Dword(d) if d == *data => {
                println!("{}", tf!("пропуск {value}: уже {data}", "skip   {value}: already {data}", ja: "スキップ {value}: すでに {data}", pt: "ignorar {value}: já é {data}", zh: "跳过   {value}：已经是 {data}"));
                continue;
            }
            Val::Other => {
                println!("{}", tf!("пропуск {value}: не DWORD, не трогаем", "skip   {value}: not a DWORD, left alone", ja: "スキップ {value}: DWORD ではないため変更しません", pt: "ignorar {value}: não é DWORD, mantido", zh: "跳过   {value}：不是 DWORD，保持不变"));
                continue;
            }
            Val::Dword(d) => d.to_string(),
            Val::Missing => "-".into(),
        };
        log.line(&format!("reg|{key}|{value}|{old}"))?;
        let ok = run("reg.exe", &["add", key, "/v", value, "/t", "REG_DWORD", "/d", &data.to_string(), "/f"]).0;
        failed += !ok as u32;
        println!("{}", tf!("{0}  {value} = {data}", "{0}  {value} = {data}", ja: "{0}  {value} = {data}", pt: "{0}  {value} = {data}", zh: "{0}  {value} = {data}", done(ok)));
    }
    Ok(failed)
}

fn apply_updaters(log: &mut Log) -> Result<u32, String> {
    let mut failed = apply_services(&upd_services(), log)?;
    for t in updater_tasks() {
        if task_enabled(&t) != Some(true) {
            println!("{}", tf!("пропуск {t}: уже отключена", "skip   {t}: already disabled", ja: "スキップ {t}: すでに無効です", pt: "ignorar {t}: já está desativada", zh: "跳过   {t}：已禁用"));
            continue;
        }
        log.line(&format!("task|{t}"))?;
        let ok = run("schtasks.exe", &["/change", "/tn", &t, "/disable"]).0;
        failed += !ok as u32;
        println!("{}", tf!("{0}  {t}: отключена", "{0}  {t}: disabled", ja: "{0}  {t}: 無効にしました", pt: "{0}  {t}: desativada", zh: "{0}  {t}：已禁用", done(ok)));
    }
    if !has_winget() {
        println!("{}", tf!("пропуск {WINGET_TASK}: winget не установлен (App Installer из Store)", "skip   {WINGET_TASK}: winget is not installed (App Installer from the Store)", ja: "スキップ {WINGET_TASK}: winget がインストールされていません (Store のアプリ インストーラー)", pt: "ignorar {WINGET_TASK}: o winget não está instalado (Instalador de Aplicativos da Store)", zh: "跳过   {WINGET_TASK}：未安装 winget（商店里的应用安装程序）"));
    } else if task_enabled(WINGET_TASK).is_some() {
        println!("{}", tf!("пропуск {WINGET_TASK}: уже есть", "skip   {WINGET_TASK}: already there", ja: "スキップ {WINGET_TASK}: すでにあります", pt: "ignorar {WINGET_TASK}: já existe", zh: "跳过   {WINGET_TASK}：已存在"));
    } else {
        log.line(&format!("newtask|{WINGET_TASK}"))?;
        // as the signed-in user (winget is a per-user app), elevated so installers do not ask; no window (conhost --headless)
        let user = env::var(winrust_common::reg::SID_ENV).ok().filter(|s| winrust_common::reg::valid_sid(s)).map_or("([Security.Principal.WindowsIdentity]::GetCurrent().Name)".to_string(), |s| psq(&s));
        let script = format!(
            "$a=New-ScheduledTaskAction -Execute 'conhost.exe' -Argument '--headless cmd.exe /c winget upgrade --all --silent --accept-source-agreements --accept-package-agreements --disable-interactivity'; \
             $t=New-ScheduledTaskTrigger -Weekly -DaysOfWeek Sunday -At 12:00; \
             $s=New-ScheduledTaskSettingsSet -StartWhenAvailable -RunOnlyIfNetworkAvailable -AllowStartIfOnBatteries -ExecutionTimeLimit (New-TimeSpan -Hours 2); \
             $p=New-ScheduledTaskPrincipal -UserId {user} -LogonType Interactive -RunLevel Highest; \
             Register-ScheduledTask -TaskPath '\\WinRust\\' -TaskName 'Update apps' -Action $a -Trigger $t -Settings $s -Principal $p -Force -ErrorAction Stop | Out-Null"
        );
        let ok = ps(&script).0;
        failed += !ok as u32;
        println!("{}", tf!("{0}  {WINGET_TASK}: еженедельно winget upgrade --all (в воскресенье в 12:00 или при следующем запуске)", "{0}  {WINGET_TASK}: weekly winget upgrade --all (Sunday 12:00, or at the next start)", ja: "{0}  {WINGET_TASK}: 毎週 winget upgrade --all (日曜 12:00、または次回起動時)", pt: "{0}  {WINGET_TASK}: winget upgrade --all semanalmente (domingo às 12:00 ou na próxima inicialização)", zh: "{0}  {WINGET_TASK}：每周 winget upgrade --all（周日 12:00，或下次启动时）", done(ok)));
    }
    Ok(failed)
}

fn apply_defender(log: &mut Log) -> Result<u32, String> {
    let have = defender_exclusions()?;
    let paths: Vec<String> = defender_paths().into_iter().filter(|p| !have.contains(&p.to_lowercase())).collect();
    let procs: Vec<&str> = DEF_PROCS.iter().copied().filter(|p| !have.contains(&p.to_lowercase())).collect();
    if paths.is_empty() && procs.is_empty() {
        println!("{}", t!("пропуск новых исключений нет", "skip   nothing new to exclude", ja: "スキップ 新しく除外するものはありません", pt: "ignorar nada novo para excluir", zh: "跳过   没有新的排除项"));
        return Ok(0);
    }
    for p in &paths {
        log.line(&format!("mpx|path|{p}"))?;
    }
    for p in &procs {
        log.line(&format!("mpx|proc|{p}"))?;
    }
    let list = |v: &[&str]| v.iter().map(|s| psq(s)).collect::<Vec<_>>().join(",");
    let mut cmd = "Add-MpPreference -ErrorAction Stop".to_string();
    if !paths.is_empty() {
        cmd += &format!(" -ExclusionPath {}", list(&paths.iter().map(String::as_str).collect::<Vec<_>>()));
    }
    if !procs.is_empty() {
        cmd += &format!(" -ExclusionProcess {}", list(&procs));
    }
    let ok = ps(&cmd).0;
    for p in paths.iter().map(String::as_str).chain(procs.iter().copied()) {
        println!("{}", tf!("{0}  исключить {1}", "{0}  exclude {1}", ja: "{0}  除外 {1}", pt: "{0}  excluir {1}", zh: "{0}  排除 {1}", done(ok), p));
    }
    Ok(!ok as u32)
}

fn apply_dns(log: &mut Log) -> Result<u32, String> {
    let path = hosts_path();
    let text = fs::read_to_string(&path).unwrap_or_default();
    if text.contains(HOSTS_BEGIN) {
        println!("{}", t!("пропуск hosts: блок уже на месте", "skip   hosts: the block is already there", ja: "スキップ hosts: ブロックはすでにあります", pt: "ignorar hosts: o bloco já está lá", zh: "跳过   hosts：该区块已存在"));
        return Ok(0);
    }
    log.line("hosts|block")?;
    let new = format!("{}\r\n\r\n{}", text.trim_end_matches(['\r', '\n']), hosts_block());
    let ok = fs::write(&path, new.trim_start_matches(['\r', '\n'])).is_ok();
    let _ = run("ipconfig.exe", &["/flushdns"]);
    println!("{}", tf!("{0}  hosts: заблокировано доменов: {1}", "{0}  hosts: {1} domains blocked", ja: "{0}  hosts: {1} 個のドメインをブロックしました", pt: "{0}  hosts: {1} domínios bloqueados", zh: "{0}  hosts：已拦截 {1} 个域名", done(ok), BLOCK.len()));
    Ok(!ok as u32)
}

fn apply(g: Group) -> Result<(), String> {
    let path = undo_path(g);
    let dir = path.parent().unwrap();
    fs::create_dir_all(dir).map_err(|e| tf!("не удалось создать {}: {e} (запустите от администратора)", "cannot create {}: {e} (run as admin)", ja: "{} を作成できません: {e} (管理者として実行してください)", pt: "não foi possível criar {}: {e} (execute como administrador)", zh: "无法创建 {}：{e}（请以管理员身份运行）", dir.display()))?;
    let f = OpenOptions::new().create(true).append(true).open(&path).map_err(|e| tf!("{}: {e}", "{}: {e}", ja: "{}: {e}", pt: "{}: {e}", zh: "{}: {e}", path.display()))?;
    let mut log = Log(f);
    let failed = match g {
        Group::Base => apply_services(&services(), &mut log)? + apply_regs(&regs(), &mut log)?,
        Group::Updaters => apply_updaters(&mut log)?,
        Group::Defender => apply_defender(&mut log)?,
        Group::Ntfs => apply_regs(&ntfs_regs(), &mut log)?,
        Group::Dns => apply_dns(&mut log)?,
    };
    drop(log);
    // nothing was changed (all skipped): no undo file, so the group does not show as applied
    if fs::metadata(&path).is_ok_and(|m| m.len() == 0) {
        let _ = fs::remove_file(&path);
    }
    println!("{}", tf!("\nИнформация для отмены сохранена в {}. Некоторые изменения вступят в силу после перезагрузки или выхода из учётной записи.", "\nUndo information saved to {}. Some changes need a reboot or sign-out.", ja: "\n元に戻す情報を {} に保存しました。一部の変更は再起動またはサインアウト後に有効になります。", pt: "\nInformações de desfazer salvas em {}. Algumas alterações exigem reiniciar ou sair da conta.", zh: "\n撤销信息已保存到 {}。部分更改需要重启或注销后生效。", path.display()));
    if failed > 0 { Err(tf!("не удалось применить изменения: {failed} (запустите от администратора)", "{failed} change(s) failed (run as administrator)", ja: "{failed} 件の変更に失敗しました (管理者として実行してください)", pt: "falha em {failed} alteração(ões) (execute como administrador)", zh: "{failed} 项更改失败（请以管理员身份运行）")) } else { Ok(()) }
}

/// The left-margin marker of an undo line: eight columns in every language.
fn restored(ok: bool) -> &'static str {
    if ok {
        t!("восстановлено", "restored", ja: "復元    ", pt: "restaurado", zh: "已恢复  ")
    } else {
        t!("ОШИБКА  ", "FAILED  ", ja: "失敗    ", pt: "FALHOU   ", zh: "失败    ")
    }
}

fn undo(g: Group) -> Result<(), String> {
    let path = undo_path(g);
    let Ok(text) = fs::read_to_string(&path) else {
        println!("{}", t!("отменять нечего (нет файла отмены)", "nothing to undo (no undo file)", ja: "元に戻すものはありません (元に戻すファイルがありません)", pt: "nada para desfazer (sem arquivo de desfazer)", zh: "没有可撤销的内容（没有撤销文件）"));
        return Ok(());
    };
    let mut failed = 0;
    // the first line per item holds the ORIGINAL value (an earlier --apply may have logged the same item again)
    for line in text.lines().rev() {
        let f: Vec<&str> = line.splitn(4, '|').collect();
        let ok = match f.as_slice() {
            ["svc", name, mode] => run("sc.exe", &["config", name, "start=", mode]).0,
            ["reg", key, value, "-"] => run("reg.exe", &["delete", key, "/v", value, "/f"]).0 || reg_val(key, value) == Val::Missing, // never written (the change itself failed): nothing to remove
            ["reg", key, value, old] => run("reg.exe", &["add", key, "/v", value, "/t", "REG_DWORD", "/d", old, "/f"]).0,
            ["task", name] => run("schtasks.exe", &["/change", "/tn", name, "/enable"]).0 || task_enabled(name).is_none(), // uninstalled with its app meanwhile
            ["newtask", name] => run("schtasks.exe", &["/delete", "/tn", name, "/f"]).0 || task_enabled(name).is_none(),
            ["mpx", "path", p] => ps(&format!("Remove-MpPreference -ErrorAction Stop -ExclusionPath {}", psq(p))).0,
            ["mpx", "proc", p] => ps(&format!("Remove-MpPreference -ErrorAction Stop -ExclusionProcess {}", psq(p))).0,
            ["hosts", "block"] => {
                let p = hosts_path();
                let ok = fs::read_to_string(&p).is_ok_and(|t| fs::write(&p, strip_block(&t)).is_ok());
                let _ = run("ipconfig.exe", &["/flushdns"]);
                ok
            }
            _ => continue,
        };
        failed += !ok as u32;
        println!("{}", tf!("{0}  {1}", "{0}  {1}", ja: "{0}  {1}", pt: "{0}  {1}", zh: "{0}  {1}", restored(ok), line));
    }
    if failed > 0 {
        return Err(tf!("{failed} элемент(ов) не восстановлено; файл отмены сохранён в {}", "{failed} item(s) not restored; the undo file is kept at {}", ja: "{failed} 件を元に戻せませんでした。元に戻すファイルは {} に残っています", pt: "{failed} item(ns) não restaurado(s); o arquivo de desfazer foi mantido em {}", zh: "{failed} 项未能恢复；撤销文件保留在 {}", path.display()));
    }
    fs::remove_file(&path).map_err(|e| e.to_string())?;
    println!("{}", t!("\nВсё восстановлено (службы снова запустятся после перезагрузки или вручную).", "\nRestored (services start again after a reboot or by hand).", ja: "\n復元しました (サービスは再起動後、または手動で再び開始されます)。", pt: "\nRestaurado (os serviços voltam a iniciar após reiniciar ou manualmente).", zh: "\n已恢复（服务会在重启后或手动启动时重新运行）。"));
    Ok(())
}

fn main() {
    winrust_common::utf8_console();
    let args: Vec<String> = env::args().skip(1).collect();
    let group = match args.iter().find(|a| !a.starts_with("--")) {
        None => Group::Base,
        Some(g) => match Group::parse(g) {
            Some(g) => g,
            None => {
                eprintln!("{}", tf!("неизвестная группа '{g}' (updaters, defender, ntfs, dns)", "unknown group '{g}' (updaters, defender, ntfs, dns)", ja: "不明なグループ '{g}' (updaters、defender、ntfs、dns)", pt: "grupo desconhecido '{g}' (updaters, defender, ntfs, dns)", zh: "未知的组 '{g}'（updaters、defender、ntfs、dns）"));
                std::process::exit(2);
            }
        },
    };
    let has = |f: &str| args.iter().any(|a| a == f);
    let r = if has("--status") {
        let on = undo_path(group).is_file();
        println!("{}", if on { t!("включено", "on", ja: "オン", pt: "ligado", zh: "开") } else { t!("выключено", "off", ja: "オフ", pt: "desligado", zh: "关") });
        std::process::exit(!on as i32);
    } else if has("--apply") {
        apply(group)
    } else if has("--undo") {
        undo(group)
    } else {
        plan(group);
        Ok(())
    };
    if let Err(e) = r {
        eprintln!("{e}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_reg_query_output() {
        let dw = "\r\nHKEY_LOCAL_MACHINE\\X\r\n    AllowTelemetry    REG_DWORD    0x1\r\n\r\n";
        assert_eq!(parse_val(true, dw), Val::Dword(1));
        assert_eq!(parse_val(true, "    V    REG_SZ    hello\r\n"), Val::Other);
        assert_eq!(parse_val(false, "ERROR: The system was unable to find the specified registry key or value."), Val::Missing);
        assert_eq!(parse_val(true, "    NtfsDisableLastAccessUpdate    REG_DWORD    0x80000002\r\n"), Val::Dword(0x8000_0002));
    }

    #[test]
    fn every_registry_item_is_hkcu_or_hklm_and_unique() {
        let mut seen = std::collections::HashSet::new();
        for (k, v, _, _) in regs().into_iter().chain(ntfs_regs()) {
            assert!(k.starts_with("HKLM\\") || k.starts_with("HKCU\\"), "{k}");
            assert!(seen.insert((k, v)), "duplicate {k} {v}");
        }
    }

    #[test]
    fn task_names_from_schtasks_csv() {
        let csv = "\r\n\"\\Adobe Acrobat Update Task\",\"21.11.2026 10:00:00\",\"Готово\"\r\n\"\\Mozilla\\Firefox Background Update 308046B0AF4A39CB\",\"N/A\",\"Ready\"\r\n\"\\Mozilla\\Firefox Background Update 308046B0AF4A39CB\",\"N/A\",\"Ready\"\r\nINFO: x\r\n";
        assert_eq!(parse_task_names(csv), vec![r"\Adobe Acrobat Update Task", r"\Mozilla\Firefox Background Update 308046B0AF4A39CB"]);
    }

    #[test]
    fn steam_library_paths() {
        let vdf = "\"libraryfolders\"\n{\n\t\"0\"\n\t{\n\t\t\"path\"\t\t\"C:\\\\Program Files (x86)\\\\Steam\"\n\t}\n\t\"1\"\n\t{\n\t\t\"path\"\t\t\"D:\\\\SteamLibrary\"\n\t}\n}";
        assert_eq!(parse_vdf_paths(vdf), vec![r"C:\Program Files (x86)\Steam", r"D:\SteamLibrary"]);
    }

    #[test]
    fn hosts_block_round_trips() {
        let orig = "# Copyright\r\n127.0.0.1 localhost\r\n";
        let with = format!("{}\r\n\r\n{}", orig.trim_end(), hosts_block());
        assert_eq!(strip_block(&with), "# Copyright\r\n127.0.0.1 localhost\r\n");
        let with_tail = format!("{with}10.0.0.1 nas\r\n");
        assert_eq!(strip_block(&with_tail), "# Copyright\r\n127.0.0.1 localhost\r\n10.0.0.1 nas\r\n");
        assert_eq!(strip_block(&hosts_block()), "");
        assert_eq!(strip_block(orig), orig);
    }

    #[test]
    fn block_list_is_clean() {
        let mut seen = std::collections::HashSet::new();
        for d in BLOCK {
            assert!(seen.insert(*d), "duplicate {d}");
            assert!(!d.contains(' ') && d.contains('.'), "{d}");
            for ms in ["microsoft", "windows", "msn.com", "live.com", "bing.com", "office", "skype", "xbox"] {
                assert!(!d.contains(ms), "Microsoft domain {d}: Defender flags it");
            }
        }
    }

    #[test]
    fn groups_parse_and_have_own_files() {
        let gs = ["updaters", "defender", "ntfs", "dns"].map(|g| Group::parse(g).unwrap());
        let files: std::collections::HashSet<_> = gs.iter().map(|g| g.file()).chain([Group::Base.file()]).collect();
        assert_eq!(files.len(), 5);
        assert_eq!(Group::parse("x"), None);
    }
}
