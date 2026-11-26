<p align="center"><img src="assets/logo.svg" alt="WinRust" width="160"></p>

# WinRust

**用 Rust 编写的 Windows 外壳：比系统自带的更快、更轻。** 一批原生小程序（不用 .NET，不用 Electron）替换掉 Windows 的部件：任务栏、资源管理器、文件搜索、任务管理器，另外还有磁盘清理、磁盘占用图和图片查看器。一个安装程序全部装好，一个按钮全部卸掉。

[English](README.md) | [Русский](README.ru.md) | [日本語](README.ja.md) | [Português (Brasil)](README.pt-BR.md) | **简体中文**

<p align="center"><img src="assets/th.jpg" alt="WinRust" width="480"></p>

> **请先读这里。** WinRust 会关闭 Windows 搜索、隐藏任务栏，还可以把资源管理器当作外壳关掉。exe 没有代码签名，SmartScreen 会报警。安装前会创建还原点，但仍建议先在一台备用机器或虚拟机里试。

## 安装

1. 从[最新版本](https://github.com/Lemxy/WinRust/releases/latest)下载 `WinRust.exe`（Windows 10/11，x64）。
2. 以管理员身份运行它。
3. 勾选需要的组件，然后按安装按钮。全新安装时 `rust-explorer` 默认不勾选，勾上它才会替换 Windows 资源管理器外壳（也可以在 topbar 那一行的齿轮里打开 **Shell: rust-explorer** 开关）。组件会按正确顺序安装；某个失败或被跳过不会中断其它组件，窗口里会列出哪里出了问题。

Windows 搜索是可选项：在精简过的 Windows 上（例如去掉 Windows 搜索服务的去臃肿 LTSC 镜像），这一步会被跳过并给出警告，而不是让安装失败。

如果 UAC 要求的是**另一个管理员**的密码，那么按用户保存的设置（启动项、计划任务、PowerShell 配置文件、搜索权限）仍然写到当前登录的用户名下，而不是你输入的那个管理员。安装程序判断不出当前用户时，会在改动任何东西之前停下。

**自动更新：** 首次安装后，计划任务 `WinRust-update`（以 SYSTEM 运行，不弹 UAC）会在开机时和每天一次检查最新版本。它下载新的 `WinRust.exe`（以及版本单独附带的程序），用项目密钥签名的清单逐个校验文件（被篡改或未签名的版本会被拒绝）；然后新的 `WinRust.exe` 会替换与其内置版本不同的已安装程序。只替换已经安装过的文件。正在运行的程序会在下次启动（登录或重启）时换到新版本。日志：`%ProgramData%\plugin-launcher\install.log`（`autoupdate:` 开头的行）。想关掉它，就在「任务计划程序」里禁用 `WinRust-update` 任务；**全部移除**会删除这个任务。

**手动更新：** 从 releases 页面下载新的 `WinRust.exe`，以管理员身份运行，选要更新或安装的组件。更新时已安装的 `rust-explorer` 默认保持勾选。你的设置和保存下来的 Windows 原值都会保留。

安装程序打开时是英文。右上角是语言下拉框（English、Русский、日本語、Português (Brasil)、简体中文），任务栏的齿轮里也有同一份列表（**任务栏设置 → 语言**），rust-settings 的「个性化」页里同样可以切换（**界面语言**）。选择保存在 `%ProgramData%\WinRust\lang.txt`（`en`、`ru`、`ja`、`pt`、`zh`）：切换的这一个程序立刻重画，其它程序在下次启动时生效，所以切换后请重启它们（或注销后重新登录）。

## 卸载

- 安装程序窗口里的**全部移除**会撤销一切：Windows 搜索、任务栏、资源管理器和相关服务都会回来。
- 没有安装程序时（例如被杀毒软件删除）：**设置 → 应用 → 已安装的应用 → WinRust → 卸载**，效果与“移除 WinRust”相同。
- 一次卸一个插件：`plugin-launcher uninstall <plugin>`（见[命令行](#命令行)）。
- **如果屏幕上的任务栏或桌面没了：** 按 Ctrl+Alt+Del → 任务管理器 → 运行新任务 → `explorer.exe`，然后运行安装程序并按「全部移除」。

卸载时 `rust-explorer` 会还原安装前保存的服务启动类型和 svchost 分组阈值。没有这些备份的旧安装会保留当前值，并给出一条警告。

## 装了些什么

| 插件 | 你会得到什么 | 它改动了 Windows 的什么 |
|---|---|---|
| **rust-search** | 覆盖全部 NTFS 驱动器的文件搜索，Alt+Space。配合 topbar 时 Win+R 打开它的「运行」模式：历史记录、文件夹补全（Tab）、Ctrl+Shift+Enter 以管理员身份运行 | 关闭 Windows 搜索服务、拦截 SearchHost.exe、隐藏任务栏搜索框 |
| **rtaskmgr-win32** | 任务管理器（Ctrl+Shift+Esc 打开它） | `taskmgr.exe` 被重定向到它（系统文件本身不动）。同时装 **rs-du** 磁盘清理。**加速**页里有一批自愿开启、可逆的开关：游戏 / 工作配置文件（停止后台服务、关闭启动器或同步类应用，之后再把它们启动回来）、清理内存备用缓存（立即，或 topbar 运行时自动）、拦截 Windows 开始菜单、小组件和「手机连接」的进程、用每周一次的 `winget upgrade` 代替第三方更新器、给构建缓存和游戏库加 Defender 排除项、更快的 NTFS（不记录最后访问时间、不用 8.3 短名）、广告拦截 hosts 文件。**启动项**页还会列出登录/启动任务和第三方自启动服务，并给出它们各自拖慢了最近几次启动多少（用 Windows 自己的启动日志）。**什么在变慢**页显示驱动耗时（DPC/中断）、无响应的窗口、慢磁盘和启动耗时。卸载会把这一切改回去 |
| **topbar** | Windows 11 风格的底部栏：窗口、时钟、音量、各种菜单 | 隐藏系统任务栏。同时装 **reshell**（快捷键：Win+Ctrl+1/2/3 和 Win+\` 切换桌面（Win+1..9 打开栏上的应用按钮）、Win+D 显示桌面、Win+I 打开设置、Win+E 打开文件管理器；栏挂了它会把任务栏找回来）、**rs-du**，以及 PowerShell 配置文件里的一段（`zoxide`、`grep`=rg、`ll`=eza、`cat`=bat；没装就什么都不做）。齿轮菜单里有几页短设置：主题、颜色与透明度（栏和 WinRust 窗口的任何颜色、栏的不透明度）、模板（保存自己的一套外观并在其间切换）、位置、大小、行为（底部 / 左侧 / 右侧、大小、自动隐藏、像 Windows 10 那样把按钮放在左端）以及「将所有设置恢复默认」 |
| **rust-explorer** | 带标签页的文件管理器，还有桌面 | 文件夹和驱动器在它里面打开；**资源管理器不再是外壳**（topbar 出问题时它会自己回来）。全新安装时，会在能备份原值的前提下调整 PcaSvc、InventorySvc、lfsvc、cbdhsvc 和 DoSvc 的启动设置，以及 svchost 分组阈值。它自带的复制引擎（SSD 上同时复制多个文件、大文件用无缓冲方式、进度显示在状态行）和右键菜单的**压缩为 ZIP**（用满所有核心）/ **解压到这里**（ZIP 和 7z），都是我们自己的 ZIP 引擎；`.zip` 像文件夹一样打开（浏览、往外复制、直接打开里面的文件）。其它程序里的「在文件夹中显示」会选中该文件（对于常见文件夹：下载、文档、桌面、图片、音乐、视频以及已固定的文件夹，即使没有打开窗口也可以）；鼠标的「后退 / 前进」键可用；右键单击桌面可以隐藏桌面图标。需要 topbar |
| **wiztree** | 磁盘占用图（开始菜单里的「WinRust Disk」） | 除了一个文件和一个快捷方式，什么都不改 |
| **photoview** | 一个很快的图片查看器 | 把它加到「打开方式」里（不会成为默认程序） |
| **rust-settings** | 用 Rust 写的设置：显示（分辨率、刷新率）、声音（音量）、电源计划和驱动器空间、按应用调音量和选设备、夜间模式、拖动排列显示器、网络适配器、每个适配器上的 Wi-Fi（密码或 802.1X）、VPN 和代理、蓝牙、应用、Windows 更新、磁盘清理、主题、关于 | 除了一个文件和一个快捷方式，什么都不改。Win+I 和栏上的菜单会打开这个窗口而不是 Windows 设置（只有少数罕见页面仍然打开系统自带的） |
| **debloat** | 关闭：遥测、诊断、Copilot、Recall、开始菜单广告、后台 Edge | 服务和注册表策略；旧值会被保存，卸载时还原。**仅命令行：** `install debloat --yes` |
| **purge** | 像 Tiny11 那样移除捆绑的垃圾：Clipchamp、新闻、天气、Xbox 覆盖层、Teams、Cortana、小组件、Skype 等等，外加遥测任务 | **不可逆，无法撤销。** 应用会对所有用户和镜像一并移除；应用商店、设置、开始菜单和运行库保留。先运行 `purge.exe`：它只列出清单。**仅命令行：** `install purge --yes`（OneDrive 和 Edge：`purge.exe --apply --onedrive --edge`） |
| chrome-rs | 一个浏览器 | 暂时还不能安装 |

每一处改动都是可逆的：卸载会把原来的值放回去。

## 样式

栏左端的齿轮打开**任务栏设置**；在它的**主题**页选一个样式，会立刻应用到栏、文件管理器（rust-explorer）和搜索窗口（Alt+Space）：

| 样式 | 外观 |
|---|---|
| **正常** | Windows 11 蓝，栏在底部 |
| **终端** | 黑底绿字，Consolas，直角 |
| **悬浮停靠** | 居中的胶囊形浮动栏；最大化的窗口占满整屏，浮动栏滑到它们上面再回到下边缘 |
| **平铺** | 顶部一条细栏，琥珀色点缀，圆角同「正常」；「平铺窗口」把打开的窗口并排放好 |
| **白色道路** | 浅象牙色配金色：栏、面板和 rust-settings 背后是淡淡的多边形三角形，有些用金色勾边 |
| **红宝石** | 深石榴红配明亮的红宝石色点缀：平滑的上下渐变，带一层柔和光泽，出现在栏和面板背后 |

齿轮菜单还能设置栏的位置（底部、左侧或右侧：侧栏只显示图标，面板在旁边弹出）和自动隐藏。

选择保存在 `%ProgramData%\WinRust\style.txt`。

## 基准测试

在一台机器上的一次性测量（Windows 11 IoT Enterprise 10.0.26300、RTX 4060 Laptop、NTFS）。把它们看作数量级，而不是实验室数据。

| 项目 | Windows | WinRust | 差别 |
|---|---|---|---|
| 文件搜索（rust-search 搜遍整个 C: vs `dir /s` 搜一个文件夹） | 8,340 ms | 45 ms | **快 99.5%** |
| 同上 vs `Get-ChildItem -Recurse` | 16,200 – 28,600 ms | 45 ms | **快 99.7–99.8%** |
| 打开一个文件夹窗口（rust-explorer，5 次的中位数） | 420 ms | 60 ms | **快 86%** |
| 外壳内存（topbar + rust-explorer + rs-shell + reshell vs explorer.exe） | 366 MB | 46 MB | **少 87%** |
| 同上，再加上搜索索引（53 MB） | 366 MB | 99 MB | **少 73%** |
| 文件管理器二进制大小（现在内含 ZIP 和 7z） | 3,280 KB | 1,179 KB | **小 64%** |
| 任务管理器二进制大小 | 5,791 KB | 954 KB | **小 84%** |
| 栏的闲置 CPU（优化后的 topbar，单核百分比） | 0.00–0.10%（explorer） | 0.05–0.10% | 与资源管理器相当 |
| 复制 5,000 个小文件，约 160 MB（rust-explorer：IFileOperation，即资源管理器的引擎，vs 它自带的复制引擎） | 16.3 s | 2.3 s | **快 86%（7 倍）** |
| 复制一个 1 GB 的文件（同上） | 0.58 s | 0.60 s | 持平 |
| 把同样的 5,000 个文件打成 ZIP（`Compress-Archive` vs rust-explorer 的压缩为 ZIP） | 48.4 s | 0.75 s | **快 98%（64 倍）** |

一句话：**在大小/速度测试里快或小 64–99%，在闲置 CPU 和复制单个大文件上与资源管理器持平。** 需要注意：rs 搜索的是一份现成的 MFT 索引（为此占用 53 MB 内存），而 `dir` 和 PowerShell 要从头遍历磁盘；测试时 explorer.exe 还开着窗口在运行，所以内存上的优势被略微夸大了；没有与 Windows 搜索和系统自带的任务管理器对比。复制和 ZIP 测试（`cargo test -p rust-explorer --release bench -- --ignored`）是对刚写入的数据各跑一次，两边都从内存缓存里读；我们的 ZIP 略大（0.9 MB vs 0.7 MB），因为每个文件都在自己的核心上单独压缩。

## 截图

**任务栏**

![桌面上的任务栏](docs/screenshots/taskbar.webp)
![任务栏菜单](docs/screenshots/taskbar-menu.png)
![栏菜单，新外观](docs/screenshots/topbar-menu-new.png)

**任务管理器**（俄文界面）

![任务管理器](docs/screenshots/task-manager.webp)

**文件管理器**

![文件管理器](docs/screenshots/explorer.png)

---

## 给开发者

### 目录结构
```
src/            安装程序（plugin-launcher.exe）
  main.rs         插件表 PLUGINS：名字、文案、安装/卸载函数对、图标
  install/        每个插件改了什么、怎么撤销（一个插件一个文件）
  cli.rs, ui.rs   命令行和窗口；两者调用同一批安装函数
common/         winrust-common：语言切换（t!/tf!: 英文、俄文、日文、巴西葡萄牙文、简体中文）、UI 样式和配色、reg.exe 输出解析、NTFS MFT 读取
plugins/        所有插件的源码，每个都是独立的 crate
ReShell/        reshell（桌面快捷键、topbar 的安全网），随 topbar 一起装
payload.txt     内嵌进安装程序的 exe，以及构建产物放哪里
payload/        构建出来的那些 exe（不在 git 里）
scripts/        构建、更新、发布
```
全部是一个 cargo workspace（一份 `Cargo.lock`，所有 exe 都在 `target\release`）。`plugins/rust-search` 和 `plugins/chrome-rs` 单独构建，因为它们需要 `panic = "unwind"`（见 `Cargo.toml`）。

### 添加一种语言
`t!("русский", "english", ja: "日本語", pt: "português", zh: "中文")` 和 `tf!(…)` 会返回当前语言的文本（不写 `ja:`/`pt:`/`zh:` 时，这几种语言显示英文）。语言列表就是 `common/src/lib.rs` 里的 `Lang`。

### 构建
需要 Windows x64、Rust stable（1.85+）和 MSVC 工具链。
```powershell
powershell -ExecutionPolicy Bypass -File scripts\build-all.ps1
```
产物：`target\release\plugin-launcher.exe`，所有插件都在里面。先关掉正在运行的安装程序。测试：`cargo test --workspace`（以及在 `plugins\rust-search` 里跑 `cargo test`）。CI 在每次推送时跑同样的步骤（`.github/workflows/build.yml`）。

| 脚本 | 做什么 |
|---|---|
| `build-all.ps1` | 全量：插件 → `payload\` → 安装程序 |
| `build-plugins.ps1` | 只构建插件 → `payload\`（不重建安装程序） |
| `update-plugin.ps1 <plugin>` | 不重建安装程序，直接重编译并重装某个已安装的插件（会通过 UAC 请求管理员） |
| `user-mode.ps1`、`dev-update.ps1` | 开发者模式：从 `%LOCALAPPDATA%` 在资源管理器之上运行 topbar、rust-explorer、rs-shell 和 reshell，不用管理员就能替换它们 |
| `bench.ps1` | 搜索和任务管理器的基准测试 |
| `publish.ps1`、`release.ps1` | 提交并推送；带 `WinRust.exe` 的 GitHub 发布 |

### 命令行
在**有管理员权限**的终端里：
```
plugin-launcher list
plugin-launcher install <plugin>... --yes (一个或多个名字，或者 `all` 表示默认组件；不带 --yes 时只打印警告)
plugin-launcher update <plugin>|all        (从 exe 旁边的 payload\ 重装)
plugin-launcher uninstall <plugin>
plugin-launcher rollback --yes           (全部移除)
plugin-launcher autoupdate               (立刻检查一次更新：WinRust-update 任务跑的就是它)
```

要在全新批量安装里带上 `rust-explorer`，运行 `plugin-launcher install all rust-explorer --yes`。如果 `topbar` 已经装好，也可以单独安装 `rust-explorer`。
一条命令里的多个插件按插件表的顺序安装（topbar 在 rust-explorer 之前）；某个失败不会中断其余的。退出码：0 成功，1 出错（至少一个插件失败），2 参数不对或缺少 `--yes`。不带参数时打开窗口。安装日志：`%ProgramData%\plugin-launcher\install.log`。

### 添加一个插件
1. 在 `plugins/<name>` 建一个 crate，并加到根 `Cargo.toml` 的 `members` 里。
2. 在 `payload.txt` 里加一行。
3. 写 `src/install/<name>.rs`，里面一对安装/卸载函数（并在 `src/install/mod.rs` 里加 `mod` 行）。改动任何东西之前先保存原值，这样卸载才能还原。
4. 在 `PLUGINS`（`src/main.rs`）里加一条。窗口、它的高度和命令行会自己带上它。

## 许可证

可任选其一：[MIT](LICENSE-MIT) 或 [Apache-2.0](LICENSE-APACHE)。
