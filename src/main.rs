// WinRust installer (plugin-launcher.exe): a small dark Win32 window and a command line that install / remove the plugins.
// The plugin exes are embedded (payload.txt, build.rs); payload\ next to this exe wins over them. Needs admin (see app.manifest).
//   main.rs     the plugin table (PLUGINS) and shared state
//   install/    what each plugin changes in the system and how it is undone
//   cli.rs      the command line;  ui.rs  the window
#![windows_subsystem = "windows"]
#![allow(unsafe_op_in_unsafe_fn)]
#[macro_use]
extern crate winrust_common;
use winrust_common as i18n;
mod cli;
mod install;
mod selection;
mod ui;
mod update;
use {cli::*, install::*, selection::*, ui::*, update::*};
use std::{
    cell::Cell,
    env, fs,
    io::Write,
    mem::{size_of, zeroed},
    os::windows::process::CommandExt,
    path::{Path, PathBuf},
    process::Command,
    ptr::{null, null_mut},
    sync::{
        Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering::Relaxed},
    },
    thread,
};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::{
        Dwm::DwmSetWindowAttribute,
        Gdi::*,
        GdiPlus::{
            FillModeAlternate, GdipAddPathArc, GdipClosePathFigure, GdipCreateFromHDC, GdipCreatePath, GdipCreateSolidFill, GdipDeleteBrush, GdipDeleteGraphics, GdipDeletePath, GdipFillPath,
            GdipSetSmoothingMode, GdiplusStartup, GdiplusStartupInput, GpBrush, GpGraphics, GpPath, GpSolidFill, SmoothingModeAntiAlias,
        },
    },
    System::{Console::{ATTACH_PARENT_PROCESS, AttachConsole, GetStdHandle, STD_OUTPUT_HANDLE}, LibraryLoader::GetModuleHandleW},
    UI::{Input::KeyboardAndMouse::*, WindowsAndMessaging::*},
};

const BG: u32 = rgb(32, 32, 32); // a dark setup wizard: charcoal page, near-black header, one lime accent
const PANEL: u32 = rgb(40, 40, 40);
const BORDER: u32 = rgb(62, 62, 62);
const TEXT: u32 = rgb(240, 240, 240);
const DIM: u32 = rgb(160, 160, 160);
const ACCENT: u32 = rgb(118, 185, 0);
const GOOD: u32 = winrust_common::style::GOOD;
const BAD: u32 = winrust_common::style::BAD;
const WM_DONE: u32 = WM_APP + 1;
const WM_MOUSELEAVE: u32 = 0x02A3; // not exported by windows-sys under the enabled features

const fn rgb(r: u8, g: u8, b: u8) -> u32 {
    r as u32 | (g as u32) << 8 | (b as u32) << 16
}

/// A text in all languages (ru, en, ja, pt, zh) for static tables: `.get()` picks the current one.
#[derive(Clone, Copy)]
struct Tr(&'static str, &'static str, &'static str, &'static str, &'static str);
impl Tr {
    fn get(&self) -> &'static str {
        t!(self.0, self.1, ja: self.2, pt: self.3, zh: self.4)
    }
}

struct Plugin {
    name: &'static str,
    desc: Tr,
    warn: Tr, // shown in a confirmation box before install
    installed: fn() -> bool,
    actions: Option<(fn() -> Result<(), String>, fn() -> Result<(), String>)>, // (install, uninstall); None = stub
    settings: bool, // shows a gear button that opens the plugin's settings panel
    bulk: bool, // visible in the window and eligible for bulk install; false = command line only
    glyph: &'static str, // Segoe MDL2 Assets icon
}
const PLUGINS: &[Plugin] = &[
    Plugin {
        name: "rust-search",
        desc: Tr("Быстрый поиск файлов по NTFS (Alt+Space). Заменяет поиск Windows: служба WSearch отключается, SearchHost блокируется.", "Fast NTFS file search (Alt+Space). Replaces Windows Search: the WSearch service is disabled and SearchHost is blocked.", "NTFS の高速ファイル検索 (Alt+Space)。Windows 検索を置き換えます: WSearch サービスは無効になり、SearchHost はブロックされます。", "Pesquisa rápida de arquivos no NTFS (Alt+Espaço). Substitui a Pesquisa do Windows: o serviço WSearch é desativado e o SearchHost é bloqueado.", "基于 NTFS 的快速文件搜索（Alt+Space）。取代 Windows 搜索：停用 WSearch 服务并拦截 SearchHost。"),
        warn: Tr("Служба поиска Windows (WSearch) будет ОТКЛЮЧЕНА, SearchHost.exe заблокирован, строка поиска на панели задач скрыта: поиск в Пуске, Проводнике и Outlook перестанет работать.", "The Windows Search service (WSearch) will be DISABLED, SearchHost.exe blocked and the taskbar search box hidden: search in Start, Explorer and Outlook will stop working.", "Windows Search サービス (WSearch) が無効になり、SearchHost.exe がブロックされ、タスクバーの検索ボックスが非表示になります。スタート、エクスプローラー、Outlook での検索は動作しなくなります。", "O serviço Pesquisa do Windows (WSearch) será DESATIVADO, o SearchHost.exe bloqueado e a caixa de pesquisa da barra de tarefas ocultada: a pesquisa no Iniciar, no Explorer e no Outlook vai parar de funcionar.", "Windows 搜索服务（WSearch）将被停用，SearchHost.exe 被拦截，任务栏搜索框被隐藏：开始菜单、资源管理器和 Outlook 里的搜索将不再可用。"),
        installed: || prog_dir().join("rs.exe").is_file(),
        actions: Some((install, uninstall)),
        settings: false,
        bulk: true,
        glyph: "\u{E721}", // search
    },
    Plugin {
        name: "rtaskmgr-win32",
        desc: Tr("Диспетчер задач на Rust. Заменяет стандартный: Ctrl+Shift+Esc и запуск taskmgr.exe открывают его.", "A task manager in Rust. Replaces the stock one: Ctrl+Shift+Esc and launching taskmgr.exe open it.", "Rust 製のタスク マネージャー。標準のものを置き換えます: Ctrl+Shift+Esc や taskmgr.exe の起動でこちらが開きます。", "Um gerenciador de tarefas em Rust. Substitui o padrão: Ctrl+Shift+Esc e a execução do taskmgr.exe abrem este.", "用 Rust 写的任务管理器。取代系统自带的：Ctrl+Shift+Esc 和启动 taskmgr.exe 都会打开它。"),
        warn: Tr("Стандартный диспетчер задач будет ЗАМЕНЁН: любой запуск taskmgr.exe откроет rtaskmgr-win32.", "The stock Task Manager will be REPLACED: any launch of taskmgr.exe opens rtaskmgr-win32.", "標準のタスク マネージャーが置き換えられます: taskmgr.exe を起動すると rtaskmgr-win32 が開きます。", "O Gerenciador de Tarefas padrão será SUBSTITUÍDO: qualquer execução do taskmgr.exe abre o rtaskmgr-win32.", "系统自带的任务管理器将被替换：任何启动 taskmgr.exe 的操作都会打开 rtaskmgr-win32。"),
        installed: tm_installed,
        actions: Some((tm_install, tm_uninstall)),
        settings: false,
        bulk: true,
        glyph: "\u{E9D9}", // diagnostic
    },
    Plugin {
        name: "topbar",
        desc: Tr("Нижняя панель в стиле Windows 11 вместо панели задач: окна, часы, меню. Стандартная панель скрывается. Вместе с ней ставятся хоткеи рабочих столов (Win+1/2/3, Win+`).", "A Windows 11-style bottom bar instead of the taskbar: windows, clock, menus. The stock taskbar is hidden. Desktop hotkeys (Win+1/2/3, Win+`) are installed with it.", "タスクバーの代わりに Windows 11 風の下部バー: ウィンドウ、時計、メニュー。標準のタスクバーは非表示になります。デスクトップのホットキー (Win+1/2/3、Win+`) も一緒にインストールされます。", "Uma barra inferior no estilo do Windows 11 no lugar da barra de tarefas: janelas, relógio, menus. A barra de tarefas padrão fica oculta. Os atalhos da área de trabalho (Win+1/2/3, Win+`) são instalados junto.", "取代任务栏的 Windows 11 风格底部栏：窗口、时钟、菜单。系统任务栏被隐藏。同时装上桌面快捷键（Win+1/2/3、Win+`）。"),
        warn: Tr("Панель задач Windows будет СКРЫТА и заменена панелью внизу экрана. Удаление плагина возвращает её.", "The Windows taskbar will be HIDDEN and replaced by a bar at the bottom of the screen. Uninstalling the plugin brings it back.", "Windows のタスクバーは非表示になり、画面下部のバーに置き換わります。プラグインをアンインストールすると元に戻ります。", "A barra de tarefas do Windows será OCULTADA e substituída por uma barra na parte inferior da tela. Desinstalar o plugin a traz de volta.", "Windows 任务栏将被隐藏，由屏幕底部的一条栏取而代之。卸载这个插件就会恢复。"),
        installed: || bar_dir().join("topbar.exe").is_file(),
        actions: Some((bar_install, bar_uninstall)),
        settings: true,
        bulk: true,
        glyph: "\u{E700}", // menu
    },
    Plugin {
        name: RUST_EXPLORER,
        desc: Tr("Файловый менеджер на Rust вместо Проводника: папки и диски открываются в нём (Win+E тоже), рабочий стол рисует он сам. Обычный explorer.exe как оболочка ОТКЛЮЧАЕТСЯ. Нужен topbar.", "A Rust file manager instead of Explorer: folders and drives open in it (Win+E too), and it draws the desktop itself. The regular explorer.exe as the shell is DISABLED. Requires topbar.", "エクスプローラーの代わりの Rust 製ファイル マネージャー: フォルダーとドライブはこちらで開き (Win+E も)、デスクトップも自分で描画します。シェルとしての通常の explorer.exe は無効になります。topbar が必要です。", "Um gerenciador de arquivos em Rust no lugar do Explorer: pastas e unidades abrem nele (Win+E também) e ele desenha a área de trabalho. O explorer.exe comum como shell é DESATIVADO. Requer a topbar.", "取代资源管理器的 Rust 文件管理器：文件夹和驱动器（包括 Win+E）都在它里面打开，桌面也由它绘制。作为外壳的常规 explorer.exe 会被停用。需要 topbar。"),
        warn: Tr("ОБЫЧНЫЙ Explorer будет ОТКЛЮЧЁН: после установки панель задач, рабочий стол и окна папок дают topbar и rust-explorer. Сначала установите topbar, без него не будет панели. Ещё для экономии RAM отключаются службы InventorySvc, lfsvc (геолокация), DoSvc ставится вручную, службы svchost объединяются (полностью после перезагрузки); удаление плагина вернёт их. Если что-то пойдёт не так: Ctrl+Alt+Del, Диспетчер задач, Запустить новую задачу, explorer.exe; затем удалите плагин.", "The REGULAR Explorer will be DISABLED: after install, topbar and rust-explorer provide the taskbar, the desktop and folder windows. Install topbar first, without it there will be no bar. To save RAM the services InventorySvc and lfsvc (geolocation) are also disabled, DoSvc is set to manual, and svchost services are merged (fully after a reboot); uninstalling the plugin restores them. If something goes wrong: Ctrl+Alt+Del, Task Manager, Run new task, explorer.exe; then uninstall the plugin.", "通常のエクスプローラーが無効になります: インストール後は topbar と rust-explorer がタスクバー、デスクトップ、フォルダー ウィンドウを提供します。先に topbar をインストールしてください。ないとバーが表示されません。メモリ節約のため、サービス InventorySvc、lfsvc (位置情報) も無効になり、DoSvc は手動に設定され、svchost のサービスは統合されます (完全に反映されるのは再起動後)。プラグインをアンインストールすると元に戻ります。問題が起きたら: Ctrl+Alt+Del、タスク マネージャー、新しいタスクの実行、explorer.exe の順に操作し、その後プラグインをアンインストールしてください。", "O Explorer COMUM será DESATIVADO: depois da instalação, a topbar e o rust-explorer fornecem a barra de tarefas, a área de trabalho e as janelas de pastas. Instale a topbar primeiro; sem ela não haverá barra. Para economizar RAM, os serviços InventorySvc e lfsvc (geolocalização) também são desativados, o DoSvc passa para manual e os serviços do svchost são unidos (totalmente após reiniciar); desinstalar o plugin os restaura. Se algo der errado: Ctrl+Alt+Del, Gerenciador de Tarefas, Executar nova tarefa, explorer.exe; depois desinstale o plugin.", "常规资源管理器将被停用：安装后，任务栏、桌面和文件夹窗口都由 topbar 和 rust-explorer 提供。请先安装 topbar，没有它就没有任务栏。为了省内存，还会停用 InventorySvc 和 lfsvc（定位）服务，把 DoSvc 设为手动，并合并 svchost 服务（重启后完全生效）；卸载插件会把它们恢复。如果出了问题：按 Ctrl+Alt+Del，打开任务管理器，运行新任务，输入 explorer.exe；然后卸载这个插件。"),
        installed: || rx_dir().join("rust-explorer.exe").is_file(),
        actions: Some((rx_install, rx_uninstall)),
        settings: false,
        bulk: true,
        glyph: "\u{E8B7}", // folder
    },
    Plugin {
        name: "wiztree",
        desc: Tr("Анализ диска: карта занятого места (treemap) по MFT за секунды. Ярлык «WinRust Disk» в меню Пуск, запускается от администратора.", "Disk analysis: a treemap of used space from the MFT in seconds. A 'WinRust Disk' shortcut in the Start menu, runs as administrator.", "ディスク分析: MFT から使用領域のツリーマップを数秒で表示します。スタート メニューに「WinRust Disk」のショートカットが作られ、管理者として実行されます。", "Análise de disco: um treemap do espaço usado, a partir da MFT, em segundos. Um atalho 'WinRust Disk' no menu Iniciar, executado como administrador.", "磁盘分析：几秒内从 MFT 画出已用空间的矩形树图。开始菜单里有「WinRust Disk」快捷方式，以管理员身份运行。"),
        warn: Tr("Ничего системного не меняется: файл в Program Files и ярлык в меню Пуск.", "Nothing in the system is changed: a file in Program Files and a Start menu shortcut.", "システムは何も変更されません: Program Files 内のファイルとスタート メニューのショートカットのみです。", "Nada no sistema é alterado: um arquivo em Program Files e um atalho no menu Iniciar.", "不更改系统里的任何东西：只在 Program Files 里放一个文件，并建一个开始菜单快捷方式。"),
        installed: || wz_dir().join("wiztree.exe").is_file(),
        actions: Some((wz_install, wz_uninstall)),
        settings: false,
        bulk: true,
        glyph: "\u{EDA2}", // hard drive
    },
    Plugin {
        name: "photoview",
        desc: Tr("Просмотр фото: открывается мгновенно, листает папку стрелками, учитывает поворот из EXIF. Появляется в «Открыть с помощью» (по умолчанию не назначается).", "Photo viewer: opens instantly, browses the folder with the arrow keys, honors EXIF rotation. Appears in 'Open with' (not set as the default).", "フォト ビューアー: 一瞬で開き、矢印キーでフォルダー内を移動し、EXIF の回転情報に対応します。「プログラムから開く」に表示されます (既定にはなりません)。", "Visualizador de fotos: abre na hora, percorre a pasta com as setas e respeita a rotação EXIF. Aparece em 'Abrir com' (não vira o padrão).", "图片查看器：瞬间打开，用方向键浏览整个文件夹，遵循 EXIF 旋转信息。会出现在「打开方式」里（不会设为默认程序）。"),
        warn: Tr("Программа добавляется в «Открыть с помощью» для jpg, png, bmp, gif, tiff, webp, heic, avif. Приложением по умолчанию её делаете вы сами в Параметрах.", "The program is added to 'Open with' for jpg, png, bmp, gif, tiff, webp, heic, avif. You make it the default app yourself in Settings.", "jpg、png、bmp、gif、tiff、webp、heic、avif の「プログラムから開く」にプログラムが追加されます。既定のアプリにするのは設定で自分で行ってください。", "O programa é adicionado a 'Abrir com' para jpg, png, bmp, gif, tiff, webp, heic e avif. Você mesmo o define como app padrão nas Configurações.", "程序会被加入 jpg、png、bmp、gif、tiff、webp、heic、avif 的「打开方式」。要不要设为默认应用，由你在「设置」里决定。"),
        installed: || pv_dir().join("photoview.exe").is_file(),
        actions: Some((pv_install, pv_uninstall)),
        settings: false,
        bulk: true,
        glyph: "\u{E91B}", // picture
    },
    Plugin {
        name: "rust-settings",
        desc: Tr("Параметры на Rust: экран (разрешение, частота), звук (громкость), тема, о системе. Win+I и меню панели открывают их, а не Параметры Windows, поэтому Проводник не нужен. Остальные разделы открывают системные.", "Settings in Rust: display (resolution, refresh rate), sound (volume), theme, about. Win+I and the bar menus open it instead of Windows Settings, so Explorer is not needed. Other pages open the system ones.", "Rust 製の設定: ディスプレイ (解像度、リフレッシュ レート)、サウンド (音量)、テーマ、バージョン情報。Win+I とバーのメニューが Windows の設定の代わりにこれを開くため、エクスプローラーは不要です。その他のページはシステムの設定を開きます。", "Configurações em Rust: tela (resolução, taxa de atualização), som (volume), tema, sobre. Win+I e os menus da barra a abrem no lugar das Configurações do Windows, então o Explorer não é necessário. As outras páginas abrem as do sistema.", "用 Rust 写的设置：显示（分辨率、刷新率）、声音（音量）、主题、关于。Win+I 和栏上的菜单会打开它而不是 Windows 设置，因此不需要资源管理器。其它页面仍然打开系统自带的。"),
        warn: Tr("Ничего системного не меняется: файл в Program Files и ярлык «WinRust Settings» в меню Пуск. Win+I начинает открывать это окно; удалите плагин, и всё вернётся.", "Nothing in the system is changed: a file in Program Files and a 'WinRust Settings' Start menu shortcut. Win+I starts opening this window; uninstall the plugin and it goes back.", "システムは何も変更されません: Program Files 内のファイルと、スタート メニューの「WinRust Settings」ショートカットのみです。Win+I でこのウィンドウが開くようになります。プラグインをアンインストールすると元に戻ります。", "Nada no sistema é alterado: um arquivo em Program Files e um atalho 'WinRust Settings' no menu Iniciar. O Win+I passa a abrir esta janela; desinstale o plugin e ele volta ao normal.", "不更改系统里的任何东西：只在 Program Files 里放一个文件，并建一个「WinRust Settings」开始菜单快捷方式。Win+I 会开始打开这个窗口；卸载插件后就会恢复原状。"),
        installed: || st_dir().join("rust-settings.exe").is_file(),
        actions: Some((st_install, st_uninstall)),
        settings: false,
        bulk: true,
        glyph: "\u{E713}", // settings
    },
    Plugin {
        name: "debloat",
        desc: Tr("Отключает мусор Windows: телеметрию, Copilot, Recall, рекламу в Пуске, фон Edge. Только обратимое, старые значения запоминаются.", "Disables Windows junk: telemetry, Copilot, Recall, Start menu ads, the Edge background. Reversible changes only, old values are remembered.", "Windows の不要なものを無効にします: テレメトリ、Copilot、Recall、スタート メニューの広告、Edge のバックグラウンド動作。元に戻せる変更のみで、元の値は記憶されます。", "Desativa o lixo do Windows: telemetria, Copilot, Recall, anúncios no menu Iniciar, o segundo plano do Edge. Apenas alterações reversíveis; os valores antigos são lembrados.", "关闭 Windows 的垃圾功能：遥测、Copilot、Recall、开始菜单广告、后台 Edge。只做可逆的改动，旧值会被记住。"),
        warn: Tr("Будут ОТКЛЮЧЕНЫ несколько служб (телеметрия, карты, отчёты об ошибках, диагностика и средства устранения неполадок, AI Fabric) и выставлены политики реестра (Copilot, Recall, реклама, фон Edge; Edge покажет «управляется организацией»). Старые значения сохраняются, uninstall debloat возвращает всё. Приложения не удаляются.", "Several services will be DISABLED (telemetry, maps, error reporting, diagnostics and troubleshooters, AI Fabric) and registry policies set (Copilot, Recall, ads, Edge background; Edge will show 'managed by your organization'). Old values are saved, uninstall debloat restores everything. No apps are removed.", "いくつかのサービス (テレメトリ、マップ、エラー報告、診断とトラブルシューティング ツール、AI Fabric) が無効になり、レジストリ ポリシー (Copilot、Recall、広告、Edge のバックグラウンド動作) が設定されます (Edge には「組織によって管理されています」と表示されます)。元の値は保存され、debloat をアンインストールするとすべて元に戻ります。アプリは削除されません。", "Vários serviços serão DESATIVADOS (telemetria, mapas, relatório de erros, diagnósticos e solucionadores de problemas, AI Fabric) e políticas de registro serão definidas (Copilot, Recall, anúncios, segundo plano do Edge; o Edge mostrará 'gerenciado pela sua organização'). Os valores antigos são salvos; desinstalar o debloat restaura tudo. Nenhum app é removido.", "将停用若干服务（遥测、地图、错误报告、诊断与疑难解答、AI Fabric），并设置注册表策略（Copilot、Recall、广告、后台 Edge；Edge 会显示「由你的组织管理」）。旧值会被保存，卸载 debloat 会全部还原。不会删除任何应用。"),
        installed: || db_dir().join("debloat.exe").is_file(),
        actions: Some((db_install, db_uninstall)),
        settings: false,
        bulk: false, // changes system behaviour: only on request, never as part of "install everything"
        glyph: "\u{EA18}", // shield
    },
    Plugin {
        name: "purge",
        desc: Tr("БЕЗВОЗВРАТНО удаляет предустановленный мусор, как Tiny11: Clipchamp, Новости, Погода, Xbox-оверлеи, Teams, Cortana, виджеты, Skype, Solitaire и др., а также задачи телеметрии. Отката нет.", "PERMANENTLY removes the bundled junk like Tiny11 does: Clipchamp, News, Weather, Xbox overlays, Teams, Cortana, Widgets, Skype, Solitaire and more, plus telemetry tasks. There is no undo.", "Tiny11 のように、プリインストールの不要なアプリを完全に削除します: Clipchamp、ニュース、天気、Xbox オーバーレイ、Teams、Cortana、ウィジェット、Skype、ソリティアなどと、テレメトリ タスク。元に戻すことはできません。", "Remove DE VEZ o lixo pré-instalado, como o Tiny11 faz: Clipchamp, Notícias, Clima, sobreposições do Xbox, Teams, Cortana, Widgets, Skype, Paciência e mais, além de tarefas de telemetria. Não há como desfazer.", "像 Tiny11 那样永久移除捆绑的垃圾：Clipchamp、新闻、天气、Xbox 覆盖层、Teams、Cortana、小组件、Skype、纸牌等等，还有遥测任务。无法撤销。"),
        warn: Tr("НЕОБРАТИМО: приложения будут удалены для всех пользователей и из образа (новые пользователи их тоже не получат), задачи телеметрии удалены. Вернуть можно только из Store или winget. Магазин, Параметры, Пуск, Калькулятор, Фото и среды выполнения не затрагиваются. OneDrive и Edge не входят: purge.exe --apply --onedrive --edge. Сначала посмотрите список: purge.exe без параметров ничего не меняет.", "IRREVERSIBLE: apps are removed for all users and from the image (new users will not get them), telemetry tasks are deleted. They come back only from the Store or winget. The Store, Settings, Start, Calculator, Photos and runtimes are not touched. OneDrive and Edge are not included: purge.exe --apply --onedrive --edge. Look at the list first: purge.exe with no arguments changes nothing.", "元に戻せません: アプリはすべてのユーザーから、およびイメージから削除され (新しいユーザーにも入りません)、テレメトリ タスクも削除されます。復活するのは Store または winget からのみです。Store、設定、スタート、電卓、フォト、ランタイムは対象外です。OneDrive と Edge は含まれません: purge.exe --apply --onedrive --edge。まず一覧を確認してください: 引数なしの purge.exe は何も変更しません。", "IRREVERSÍVEL: os apps são removidos para todos os usuários e da imagem (novos usuários não os receberão) e as tarefas de telemetria são excluídas. Eles só voltam pela Store ou pelo winget. A Store, as Configurações, o Iniciar, a Calculadora, o Fotos e os runtimes não são afetados. OneDrive e Edge não estão incluídos: purge.exe --apply --onedrive --edge. Veja a lista primeiro: o purge.exe sem argumentos não altera nada.", "不可逆：应用会对所有用户并从镜像中移除（新用户也不会再得到它们），遥测任务会被删除。只能从应用商店或 winget 装回来。应用商店、设置、开始菜单、计算器、照片和运行库不受影响。不包括 OneDrive 和 Edge：`purge.exe --apply --onedrive --edge`。先看清单：不带参数的 purge.exe 不会改动任何东西。"),
        installed: || pu_dir().join("purge.exe").is_file(),
        actions: Some((pu_install, pu_uninstall)),
        settings: false,
        bulk: false, // destructive and permanent: only on request, never as part of "install everything"
        glyph: "\u{E74D}", // delete
    },
    Plugin { name: "chrome-rs", desc: Tr("Браузер на Rust. Установка пока недоступна (заглушка).", "A browser in Rust. Installation is not available yet (placeholder).", "Rust 製のブラウザー。インストールはまだ利用できません (プレースホルダー)。", "Um navegador em Rust. A instalação ainda não está disponível (provisório).", "用 Rust 写的浏览器。暂时还不能安装（占位）。"), warn: Tr("", "", "", "", ""), installed: || false, actions: None, settings: false, bulk: true, glyph: "\u{E774}" }, // globe
];

static BUSY: AtomicBool = AtomicBool::new(false);
static JOB: AtomicUsize = AtomicUsize::new(usize::MAX); // what BUSY is working on: plugin index, or PLUGINS.len() = rollback
static INSTALLED: [AtomicBool; PLUGINS.len()] = [const { AtomicBool::new(false) }; PLUGINS.len()];
fn refresh() {
    for (p, f) in PLUGINS.iter().zip(&INSTALLED) {
        f.store((p.installed)(), Relaxed);
    }
}
// Initialized when the window opens. Refreshing installed state preserves the user's checkbox choices.
static SELECTED: [AtomicBool; PLUGINS.len()] = [const { AtomicBool::new(false) }; PLUGINS.len()];
static MSG: Mutex<(String, bool)> = Mutex::new((String::new(), true));
thread_local! { static SETTINGS: Cell<Option<usize>> = const { Cell::new(None) }; } // plugin whose settings panel is open
thread_local! { static HOVER: Cell<Option<usize>> = const { Cell::new(None) }; }

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    if !args.is_empty() {
        std::process::exit(cli(&args));
    }
    refresh();
    reset_selection();
    unsafe {
        let (mut token, mut input): (usize, GdiplusStartupInput) = (0, zeroed());
        input.GdiplusVersion = 1;
        GdiplusStartup(&mut token, &input, null_mut());
        let hi = GetModuleHandleW(null());
        let cls = wide("WinRust");
        let wc = WNDCLASSW { lpfnWndProc: Some(proc), hInstance: hi, hCursor: LoadCursorW(null_mut(), IDC_ARROW), lpszClassName: cls.as_ptr(), ..zeroed() };
        RegisterClassW(&wc);
        let s = ui::scale();
        let style = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX;
        let mut r = RECT { left: 0, top: 0, right: (WIN_W * s) as i32, bottom: (WIN_H * s) as i32 };
        AdjustWindowRect(&mut r, style, 0);
        let title = wide("WinRust");
        let hwnd = CreateWindowExW(0, cls.as_ptr(), title.as_ptr(), style | WS_VISIBLE, CW_USEDEFAULT, CW_USEDEFAULT, r.right - r.left, r.bottom - r.top, null_mut(), null_mut(), hi, null());
        let h = hwnd as isize;
        thread::spawn(move || {
            if update::look_for_update().is_some() {
                InvalidateRect(h as HWND, null(), 0);
            }
        });
        DwmSetWindowAttribute(hwnd, 20, &1i32 as *const i32 as *const _, 4); // dark title bar
        DwmSetWindowAttribute(hwnd, 35, &ui::HEAD as *const u32 as *const _, 4); // caption colour = the header's (Win11; ignored on Win10)
        let mut m: MSG = zeroed();
        while GetMessageW(&mut m, null_mut(), 0, 0) > 0 {
            TranslateMessage(&m);
            DispatchMessageW(&m);
        }
    }
}
