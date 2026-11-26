// Cleaner for ordinary users: every item says what it is, how risky it is and how it goes away.
// Safe/Rebuild items regenerate on their own and are picked by default; Ask items (old files, recycle bin, Windows.old)
// are only removed when chosen. Files go to a per-drive quarantine for 30 days (`--restore`, `--purge`).
// shortcuts: Windows.old/SoftwareDistribution may need ownership rights (reported as "partly kept");
// leftovers of uninstalled programs in AppData are not detected; no restore point is created (quarantine instead).
use super::*;
use std::{cmp::Reverse, time::{Duration, SystemTime, UNIX_EPOCH}};
use windows_sys::Win32::System::Console::{ENABLE_VIRTUAL_TERMINAL_PROCESSING, GetConsoleMode, GetStdHandle, STD_OUTPUT_HANDLE, SetConsoleMode};

const QUAR_DAYS: u64 = 30;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Risk { Safe, Rebuild, Ask }
#[derive(Clone, Copy, PartialEq, Eq)]
enum How { Wipe, Remove, Quarantine }

struct Item { risk: Risk, how: How, label: String, note: &'static str, paths: Vec<PathBuf>, bytes: u64 }

fn size_of(p: &Path) -> u64 { if p.is_dir() { dir_size(p) } else { p.metadata().map_or(0, |m| m.len()) } }

fn now() -> u64 { SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs()) }

/// True when nothing touched the file (written, read, created) for `days`.
fn quiet_for(m: &fs::Metadata, days: u64) -> bool {
    [m.modified(), m.accessed(), m.created()].into_iter().flatten().max()
        .is_some_and(|t| t.elapsed().is_ok_and(|e| e > Duration::from_secs(days * 86400)))
}

/// A dir is idle when it and its direct children (release/, debug/, .package-lock.json...) are quiet for `days`.
fn idle(p: &Path, days: u64) -> bool {
    p.metadata().is_ok_and(|m| quiet_for(&m, days)) && fs::read_dir(p).into_iter().flatten().flatten().all(|e| e.metadata().is_ok_and(|m| quiet_for(&m, days)))
}

fn subdirs(base: &Path, pred: impl Fn(&str) -> bool) -> Vec<PathBuf> {
    fs::read_dir(base).into_iter().flatten().flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()) && pred(&e.file_name().to_string_lossy())).map(|e| e.path()).collect()
}

/// The known places. Paths that do not exist are dropped later, so rules for software you do not have cost nothing.
fn rules() -> Vec<Item> {
    let var = |k: &str| env::var_os(k).map(PathBuf::from);
    let (local, roam, home, win, pdata) = (var("LOCALAPPDATA"), var("APPDATA"), var("USERPROFILE"), var("SystemRoot"), var("ProgramData"));
    let at = |b: &Option<PathBuf>, rel: &[&str]| -> Vec<PathBuf> { b.iter().flat_map(|b| rel.iter().map(move |r| b.join(r))).collect() };
    let mut v: Vec<Item> = vec![];
    let mut add = |risk, label: &str, note, paths: Vec<PathBuf>| v.push(Item { risk, how: How::Wipe, label: label.into(), note, paths, bytes: 0 });

    add(Risk::Safe, t!("Временные файлы", "Temporary files", ja: "一時ファイル", pt: "Arquivos temporários", zh: "临时文件"), t!("Мусор от программ и установщиков. Программы создадут нужное заново.", "Junk left by programs and installers. Programs will recreate what they need.", ja: "プログラムやインストーラーが残した不要ファイルです。必要なものはプログラムが再作成します。", pt: "Lixo deixado por programas e instaladores. Os programas recriam o que precisarem.", zh: "程序和安装程序留下的垃圾文件。程序会重新创建所需内容。"),
        [var("TEMP").into_iter().collect::<Vec<_>>(), at(&win, &["Temp"])].concat());
    add(Risk::Safe, t!("Отчёты об ошибках и дампы падений", "Error reports and crash dumps", ja: "エラーレポートとクラッシュダンプ", pt: "Relatórios de erros e despejos de falhas", zh: "错误报告和崩溃转储"), t!("Данные о сбоях программ для разработчиков. Вам они не нужны.", "Program crash data for developers. You do not need it.", ja: "開発者向けのプログラムのクラッシュデータです。通常は不要です。", pt: "Dados de falhas de programas para desenvolvedores. Você não precisa deles.", zh: "供开发者使用的程序崩溃数据。你不需要它。"),
        [at(&pdata, &["Microsoft/Windows/WER/ReportArchive", "Microsoft/Windows/WER/ReportQueue"]), at(&local, &["CrashDumps"]), at(&win, &["Minidump"])].concat());
    add(Risk::Safe, t!("Кеш обновлений Windows", "Windows Update cache", ja: "Windows Update のキャッシュ", pt: "Cache do Windows Update", zh: "Windows 更新缓存"), t!("Уже установленные обновления. Что понадобится, Windows скачает заново.", "Updates that are already installed. Windows re-downloads what it needs.", ja: "インストール済みの更新プログラムです。必要なものは Windows が再ダウンロードします。", pt: "Atualizações já instaladas. O Windows baixa de novo o que precisar.", zh: "已安装的更新。Windows 会重新下载所需内容。"),
        at(&win, &["SoftwareDistribution/Download", "ServiceProfiles/NetworkService/AppData/Local/Microsoft/Windows/DeliveryOptimization/Cache"]));
    add(Risk::Safe, t!("Кеш интернета Windows", "Windows internet cache", ja: "Windows のインターネット キャッシュ", pt: "Cache de internet do Windows", zh: "Windows 互联网缓存"), t!("Временные копии страниц и картинок для системных программ.", "Temporary copies of pages and images for system programs.", ja: "システム プログラム用のページと画像の一時コピーです。", pt: "Cópias temporárias de páginas e imagens para programas do sistema.", zh: "系统程序使用的页面和图片临时副本。"), at(&local, &["Microsoft/Windows/INetCache"]));

    let web = t!("Копии страниц и картинок. Сайты первый раз откроются чуть медленнее. Пароли, закладки и история не затрагиваются.", "Copies of pages and images. Sites open a little slower the first time. Passwords, bookmarks and history are not touched.", ja: "ページと画像のコピーです。初回はサイトの表示が少し遅くなります。パスワード、ブックマーク、履歴は削除されません。", pt: "Cópias de páginas e imagens. Os sites abrem um pouco mais devagar na primeira vez. Senhas, favoritos e histórico não são afetados.", zh: "页面和图片的副本。网站首次打开会稍慢。密码、书签和历史记录不受影响。");
    for (name, rel) in [("Google Chrome", "Google/Chrome/User Data"), ("Microsoft Edge", "Microsoft/Edge/User Data"), (t!("Яндекс Браузер", "Yandex Browser", ja: "Yandex ブラウザー", pt: "Yandex Browser", zh: "Yandex 浏览器"), "Yandex/YandexBrowser/User Data"), ("Brave", "BraveSoftware/Brave-Browser/User Data")] {
        let base = local.as_ref().map(|l| l.join(rel)).unwrap_or_default();
        let ps = subdirs(&base, |n| n == "Default" || n.starts_with("Profile ")).into_iter()
            .flat_map(|p| ["Cache/Cache_Data", "Code Cache", "GPUCache", "Service Worker/CacheStorage"].map(|c| p.join(c))).collect();
        add(Risk::Rebuild, name, web, if base.as_os_str().is_empty() { vec![] } else { ps });
    }
    let ff = local.as_ref().map(|l| l.join("Mozilla/Firefox/Profiles")).unwrap_or_default();
    add(Risk::Rebuild, "Firefox", web, if ff.as_os_str().is_empty() { vec![] } else { subdirs(&ff, |_| true).into_iter().map(|p| p.join("cache2")).collect() });
    add(Risk::Rebuild, t!("Telegram: кеш медиа", "Telegram: media cache", ja: "Telegram: メディア キャッシュ", pt: "Telegram: cache de mídia", zh: "Telegram：媒体缓存"), t!("Картинки и видео из чатов. Скачаются снова, когда откроете чат.", "Pictures and videos from chats. They download again when you open a chat.", ja: "チャットの画像と動画です。チャットを開くと再ダウンロードされます。", pt: "Fotos e vídeos dos chats. Eles são baixados de novo ao abrir um chat.", zh: "聊天中的图片和视频。打开聊天时会重新下载。"), at(&roam, &["Telegram Desktop/tdata/user_data/media_cache", "Telegram Desktop/tdata/user_data/cache"]));
    add(Risk::Rebuild, "Discord", t!("Кеш картинок и интерфейса. Создастся заново.", "Image and UI cache. It will be recreated.", ja: "画像と UI のキャッシュです。再作成されます。", pt: "Cache de imagens e da interface. Será recriado.", zh: "图片和界面缓存。会自动重建。"), at(&roam, &["discord/Cache", "discord/Code Cache", "discord/GPUCache"]));
    add(Risk::Rebuild, "Microsoft Teams", t!("Кеш интерфейса и вложений. Создастся заново.", "UI and attachment cache. It will be recreated.", ja: "UI と添付ファイルのキャッシュです。再作成されます。", pt: "Cache da interface e de anexos. Será recriado.", zh: "界面和附件缓存。会自动重建。"), at(&roam, &["Microsoft/Teams/Cache", "Microsoft/Teams/blob_storage", "Microsoft/Teams/GPUCache", "Microsoft/Teams/tmp"]));
    add(Risk::Rebuild, "Visual Studio Code", t!("Кеш редактора. Создастся заново.", "Editor cache. It will be recreated.", ja: "エディターのキャッシュです。再作成されます。", pt: "Cache do editor. Será recriado.", zh: "编辑器缓存。会自动重建。"), at(&roam, &["Code/Cache", "Code/CachedData", "Code/GPUCache", "Code/Code Cache"]));
    add(Risk::Rebuild, t!("Кеш шейдеров видеокарты (NVIDIA, AMD, DirectX)", "GPU shader cache (NVIDIA, AMD, DirectX)", ja: "GPU シェーダー キャッシュ (NVIDIA、AMD、DirectX)", pt: "Cache de shaders da GPU (NVIDIA, AMD, DirectX)", zh: "GPU 着色器缓存（NVIDIA、AMD、DirectX）"), t!("Игры пересоберут его сами, первый запуск игры может чуть подтормаживать.", "Games rebuild it themselves, the first launch of a game may stutter a little.", ja: "ゲームが自動的に再構築します。ゲームの初回起動時に少しカクつくことがあります。", pt: "Os jogos o recriam sozinhos; a primeira abertura de um jogo pode engasgar um pouco.", zh: "游戏会自行重建，首次启动游戏时可能略有卡顿。"),
        at(&local, &["NVIDIA/DXCache", "NVIDIA/GLCache", "D3DSCache", "AMD/DxCache", "AMD/DxcCache"]));
    add(Risk::Rebuild, t!("Steam: кеш браузера", "Steam: browser cache", ja: "Steam: ブラウザー キャッシュ", pt: "Steam: cache do navegador", zh: "Steam：浏览器缓存"), t!("Кеш встроенного браузера Steam. Игры не затрагиваются.", "Cache of Steam's built-in browser. Games are not touched.", ja: "Steam 内蔵ブラウザーのキャッシュです。ゲームには影響しません。", pt: "Cache do navegador integrado do Steam. Os jogos não são afetados.", zh: "Steam 内置浏览器的缓存。不会影响游戏。"), at(&local, &["Steam/htmlcache"]));
    add(Risk::Rebuild, t!("Кеши программистов (cargo, npm, pip, NuGet)", "Developer caches (cargo, npm, pip, NuGet)", ja: "開発者向けキャッシュ (cargo、npm、pip、NuGet)", pt: "Caches de desenvolvedor (cargo, npm, pip, NuGet)", zh: "开发者缓存（cargo、npm、pip、NuGet）"), t!("Скачанные пакеты. Загрузятся снова при следующей сборке.", "Downloaded packages. They download again on the next build.", ja: "ダウンロード済みのパッケージです。次回のビルドで再ダウンロードされます。", pt: "Pacotes baixados. Eles são baixados de novo na próxima compilação.", zh: "已下载的软件包。下次构建时会重新下载。"),
        [at(&home, &[".cargo/registry/cache", ".cargo/registry/src"]), at(&local, &["npm-cache", "pip/cache", "NuGet/v3-cache"]), at(&roam, &["npm-cache"])].concat());
    if let Some(w) = win.as_ref().and_then(|w| w.parent()) {
        v.push(Item { risk: Risk::Ask, how: How::Remove, label: t!("Windows.old (прежняя версия Windows)", "Windows.old (previous Windows version)", ja: "Windows.old (以前の Windows)", pt: "Windows.old (versão anterior do Windows)", zh: "Windows.old（以前的 Windows 版本）").into(),
            note: t!("Нужна только для отката на старую Windows.", "Only needed to roll back to the old Windows.", ja: "以前の Windows に戻すときにのみ必要です。", pt: "Necessário apenas para voltar ao Windows anterior.", zh: "仅在回滚到旧版 Windows 时才需要。"), paths: vec![w.join("Windows.old")], bytes: 0 });
    }
    v
}

/// Project build dirs: target next to Cargo.toml, node_modules next to package.json (outermost only, never inside apps).
fn projects(recs: &HashMap<u32, Rec>, tot: &HashMap<u32, u64>, drive: char) -> (Vec<PathBuf>, u64) {
    let is_cand = |r: &Rec| r.dir && (r.name.eq_ignore_ascii_case("target") || r.name.eq_ignore_ascii_case("node_modules"));
    let parents: HashSet<u32> = recs.values().filter(|r| is_cand(r)).map(|r| r.parent).collect();
    let marks: HashSet<(u32, String)> = recs.values().filter(|r| !r.dir && parents.contains(&r.parent) && (r.name.eq_ignore_ascii_case("Cargo.toml") || r.name.eq_ignore_ascii_case("package.json"))).map(|r| (r.parent, r.name.to_ascii_lowercase())).collect();
    let ok = |r: &Rec| marks.contains(&(r.parent, if r.name.eq_ignore_ascii_case("target") { "cargo.toml" } else { "package.json" }.into()));
    let cands: HashSet<u32> = recs.iter().filter(|(_, r)| is_cand(r) && ok(r)).map(|(&i, _)| i).collect();
    let (mut out, mut bytes) = (vec![], 0);
    for &id in &cands {
        let (mut p, mut nested) = (recs[&id].parent, false);
        for _ in 0..128 { if cands.contains(&p) { nested = true; break; } if p == ROOT { break; } p = match recs.get(&p) { Some(x) => x.parent, None => break }; }
        if nested { continue; }
        let path = path_of(recs, id, drive).0;
        // installed apps ship their own node_modules (VS Code, Discord, Node, Unity...): deleting those breaks them
        let low = path.to_lowercase();
        if low.contains("\\appdata\\") || low[3..].starts_with("program files") || low[3..].starts_with("windows") { continue; }
        let p = PathBuf::from(path);
        if !idle(&p, 30) { continue; } // still being built or installed into: somebody is working here
        bytes += tot.get(&id).copied().unwrap_or(0);
        out.push(p);
    }
    (out, bytes)
}

/// Old installers and archives lying in Downloads (top level only).
fn old_downloads() -> Option<Item> {
    const EXT: [&str; 7] = ["exe", "msi", "msix", "iso", "zip", "rar", "7z"];
    let dl = env::var_os("USERPROFILE").map(|h| PathBuf::from(h).join("Downloads"))?;
    let paths: Vec<PathBuf> = fs::read_dir(&dl).ok()?.flatten().filter(|e| {
        let p = e.path();
        e.metadata().is_ok_and(|m| m.is_file() && quiet_for(&m, 90)) && p.extension().is_some_and(|x| EXT.contains(&x.to_string_lossy().to_lowercase().as_str()))
    }).map(|e| e.path()).collect();
    Some(Item { risk: Risk::Ask, how: How::Quarantine, label: tf!("Старые установщики и архивы в «Загрузках» ({} шт., старше 90 дней)", "Old installers and archives in Downloads ({} items, older than 90 days)", ja: "ダウンロード フォルダー内の古いインストーラーとアーカイブ ({} 個、90 日以上前)", pt: "Instaladores e arquivos antigos em Downloads ({} itens, com mais de 90 dias)", zh: "下载文件夹中的旧安装程序和压缩包（{} 项，超过 90 天）", paths.len()),
        note: t!("Скорее всего, программы уже установлены.", "Most likely the programs are already installed.", ja: "おそらくそのプログラムはすでにインストール済みです。", pt: "Provavelmente os programas já estão instalados.", zh: "程序很可能已经安装。"), paths, bytes: 0 })
}

/// Biggest forgotten videos/archives/installers in the user's own folders (not cloud folders, not AppData).
fn big_old(recs: &HashMap<u32, Rec>, tot: &HashMap<u32, u64>, drive: char, skip: &[PathBuf]) -> Vec<Item> {
    const EXT: [&str; 10] = ["mp4", "mkv", "avi", "mov", "iso", "zip", "rar", "7z", "exe", "msi"];
    const NO: [&str; 6] = ["\\appdata\\", "\\programdata\\", "onedrive", "google drive", "dropbox", "yandex"];
    let mut c: Vec<(u64, u32)> = recs.iter()
        .filter(|(id, r)| !r.dir && tot.get(*id).is_some_and(|&s| s >= 1 << 30) && r.name.rsplit_once('.').is_some_and(|(_, e)| EXT.contains(&e.to_lowercase().as_str())))
        .map(|(&id, _)| (tot[&id], id)).collect();
    c.sort_by_key(|&(s, _)| Reverse(s));
    let mut out = vec![];
    for (s, id) in c {
        let p = PathBuf::from(path_of(recs, id, drive).0);
        let low = p.to_string_lossy().to_lowercase();
        if !low[3..].starts_with("users\\") || NO.iter().any(|k| low.contains(k)) || skip.contains(&p) || !p.metadata().is_ok_and(|m| quiet_for(&m, 180)) { continue; }
        out.push(Item { risk: Risk::Ask, how: How::Quarantine, label: p.to_string_lossy().into(), note: t!("Большой файл, который давно не открывали.", "A large file that has not been opened for a long time.", ja: "長期間開かれていない大きなファイルです。", pt: "Um arquivo grande que não é aberto há muito tempo.", zh: "很久没有打开过的大文件。"), paths: vec![p], bytes: s });
        if out.len() == 15 { break; }
    }
    out
}

fn vt() -> bool {
    unsafe {
        let h = GetStdHandle(STD_OUTPUT_HANDLE);
        let mut m = 0;
        GetConsoleMode(h, &mut m) != 0 && SetConsoleMode(h, m | ENABLE_VIRTUAL_TERMINAL_PROCESSING) != 0
    }
}

/// Enter = defaults, "a" = everything, "n" = nothing, numbers (1-based, any separator) = defaults plus those.
fn select(s: &str, sel: &mut [bool]) {
    match s.trim().to_lowercase().as_str() {
        "n" | "q" => sel.fill(false),
        "a" => sel.fill(true),
        t => for n in t.split(|c: char| !c.is_ascii_digit()).filter_map(|x| x.parse::<usize>().ok()) {
            if let Some(x) = sel.get_mut(n.wrapping_sub(1)) { *x = true; }
        },
    }
}

fn qroot(drive: char) -> PathBuf { PathBuf::from(format!(r"{}:\.rs-du-quarantine", drive.to_ascii_uppercase())) }

/// The list of quarantined items lives next to rs-du.exe (Program Files, admin-only), not in the quarantine folder: under a
/// drive root any user can create and edit files, and a forged row would make this elevated process delete or move any path.
fn mpath(drive: char) -> PathBuf {
    env::current_exe().expect("exe path").with_file_name(format!("quarantine-{}.tsv", drive.to_ascii_uppercase()))
}

/// Closes the quarantine folder to everyone but administrators and SYSTEM (inherited from a drive root, users could edit it).
// shortcut: a folder a user created before rs-du ever ran stays owned by them; the trusted list above keeps that harmless.
fn lock(root: &Path) {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| { let _ = std::process::Command::new("icacls").arg(root).args(["/inheritance:r", "/grant:r", "*S-1-5-32-544:(OI)(CI)F", "*S-1-5-18:(OI)(CI)F", "/Q"]).output(); });
}

fn quarantine(p: &Path, stamp: u64, n: usize) -> bool {
    let drive = p.to_string_lossy().chars().next().unwrap_or('C');
    let root = qroot(drive);
    let dir = root.join(stamp.to_string());
    let to = dir.join(format!("{n}_{}", p.file_name().unwrap_or_default().to_string_lossy()));
    let made = fs::create_dir_all(&dir).is_ok();
    if made { lock(&root); }
    let ok = made && fs::rename(p, &to).is_ok();
    if ok && let Ok(mut f) = fs::OpenOptions::new().append(true).create(true).open(mpath(drive)) {
        let _ = writeln!(f, "{stamp}\t{}\t{}", p.display(), to.display());
    }
    ok
}

type Row = (u64, PathBuf, PathBuf); // (when, original path, quarantined path)

/// Rows whose quarantined path is not inside this drive's quarantine folder are dropped: purge deletes that path.
fn parse(text: &str, drive: char) -> Vec<Row> {
    let root = qroot(drive);
    text.lines().filter_map(|l| {
        let mut s = l.splitn(3, '\t');
        Some((s.next()?.parse().ok()?, s.next()?.into(), s.next()?.into()))
    }).filter(|r: &Row| r.2.starts_with(&root) && !r.2.components().any(|c| c == std::path::Component::ParentDir)).collect()
}

fn manifest(drive: char) -> (PathBuf, Vec<Row>) {
    let m = mpath(drive);
    let rows = parse(&fs::read_to_string(&m).unwrap_or_default(), drive);
    (m, rows)
}

fn save(m: &Path, rows: &[Row]) {
    let _ = fs::write(m, rows.iter().map(|(t, a, b)| format!("{t}\t{}\t{}\n", a.display(), b.display())).collect::<String>());
}

fn remove(p: &Path) { let _ = if p.is_dir() { fs::remove_dir_all(p) } else { fs::remove_file(p) }; }

/// Deletes quarantined items older than 30 days (all of them with `all`). This is when the space really comes back.
pub fn purge(drive: char, all: bool) {
    let (m, rows) = manifest(drive);
    let (mut keep, mut freed) = (vec![], 0);
    for r in rows {
        if all || now().saturating_sub(r.0) > QUAR_DAYS * 86400 {
            let before = size_of(&r.2);
            remove(&r.2);
            freed += before.saturating_sub(size_of(&r.2));
            if !r.2.exists() { continue; }
        }
        keep.push(r);
    }
    save(&m, &keep);
    for d in subdirs(&qroot(drive), |_| true) { let _ = fs::remove_dir(d); } // only empty ones go
    if freed > 0 { println!("{}", tf!("карантин очищен: освобождено {}", "quarantine emptied: {} freed", ja: "隔離を空にしました: {} を解放", pt: "quarentena esvaziada: {} liberados", zh: "隔离区已清空：释放了 {}", human(freed))); }
}

/// Puts quarantined items back where they were.
pub fn restore(drive: char) {
    let (m, rows) = manifest(drive);
    let mut keep = vec![];
    for r in rows {
        if !r.2.exists() { continue; }
        if r.1.exists() { println!("{}", tf!("не восстановлено (место занято): {}", "not restored (the place is taken): {}", ja: "復元できません (場所が使われています): {}", pt: "não restaurado (o local está ocupado): {}", zh: "未恢复（原位置已被占用）：{}", r.1.display())); keep.push(r); continue; }
        let moved = r.1.parent().is_some_and(|d| fs::create_dir_all(d).is_ok()) && fs::rename(&r.2, &r.1).is_ok();
        println!("{} {}", if moved { t!("восстановлено", "restored", ja: "復元しました", pt: "restaurado", zh: "已恢复") } else { t!("ОШИБКА", "FAILED", ja: "失敗", pt: "FALHOU", zh: "失败") }, r.1.display());
        if !moved { keep.push(r); }
    }
    save(&m, &keep);
}

pub fn clean(recs: &HashMap<u32, Rec>, tot: &HashMap<u32, u64>, drive: char, apply: bool, ask: bool, all: bool, safe_only: bool) {
    let mut items = rules();
    let (proj, pb) = projects(recs, tot, drive);
    items.push(Item { risk: Risk::Rebuild, how: How::Remove, label: t!("Папки сборки проектов (target, node_modules)", "Project build folders (target, node_modules)", ja: "プロジェクトのビルド フォルダー (target、node_modules)", pt: "Pastas de compilação de projetos (target, node_modules)", zh: "项目构建文件夹（target、node_modules）").into(), note: t!("Пересоберутся при следующей сборке или npm install.", "They are rebuilt on the next build or npm install.", ja: "次回のビルドまたは npm install で再作成されます。", pt: "São recriadas na próxima compilação ou npm install.", zh: "会在下次构建或 npm install 时重新生成。"), paths: proj, bytes: pb });
    items.push(Item { risk: Risk::Ask, how: How::Wipe, label: t!("Корзина", "Recycle Bin", ja: "ごみ箱", pt: "Lixeira", zh: "回收站").into(), note: t!("Файлы, которые вы уже удалили.", "Files you have already deleted.", ja: "すでに削除したファイルです。", pt: "Arquivos que você já excluiu.", zh: "你已经删除的文件。"), paths: vec![PathBuf::from(format!("{drive}:\\$Recycle.Bin"))], bytes: 0 }); // per-user subfolders need admin
    let dl = old_downloads();
    let skip = dl.as_ref().map(|d| d.paths.clone()).unwrap_or_default();
    items.extend(dl);
    items.extend(big_old(recs, tot, drive, &skip));
    items.retain_mut(|i| {
        i.paths.retain(|p| p.exists());
        if i.bytes == 0 { i.bytes = i.paths.iter().map(|p| size_of(p)).sum(); }
        i.bytes >= 1 << 20 // crumbs (an "empty" recycle bin is 987 bytes) are noise
    });
    items.sort_by_key(|i| (i.risk as u8, Reverse(i.bytes)));

    let c = vt();
    let (rst, col) = (if c { "\x1b[0m" } else { "" }, |r: Risk| if !c { "" } else { match r { Risk::Safe => "\x1b[32m", Risk::Rebuild => "\x1b[33m", Risk::Ask => "\x1b[31m" } });
    let mut last = None;
    for (n, i) in items.iter().enumerate() {
        if last != Some(i.risk) {
            last = Some(i.risk);
            println!("\n{}{}{rst}", col(i.risk), match i.risk {
                Risk::Safe => t!("БЕЗОПАСНО: Windows и программы создадут это заново сами", "SAFE: Windows and programs recreate this on their own", ja: "安全: Windows とプログラムが自動的に再作成します", pt: "SEGURO: o Windows e os programas recriam isso sozinhos", zh: "安全：Windows 和程序会自行重新创建"),
                Risk::Rebuild => t!("МОЖНО УДАЛИТЬ: пересоздастся, но первое время может работать чуть медленнее", "CAN BE DELETED: it is recreated, but may run a little slower for a while", ja: "削除可能: 再作成されますが、しばらく動作が少し遅くなることがあります", pt: "PODE EXCLUIR: é recriado, mas pode ficar um pouco mais lento por um tempo", zh: "可以删除：会自动重建，但可能暂时运行稍慢"),
                Risk::Ask => t!("ТОЛЬКО ПО ВАШЕМУ ВЫБОРУ: проверьте, что это вам не нужно", "YOUR CHOICE ONLY: make sure you do not need this", ja: "ご自身で判断: 不要かどうか確認してください", pt: "SÓ VOCÊ DECIDE: confirme que você não precisa disso", zh: "仅由你决定：请确认你不需要此项"),
            });
        }
        let fate = match (i.risk, i.how) { (Risk::Ask, How::Quarantine) => t!(" Уйдёт в карантин на 30 дней, можно вернуть (rs-du --restore).", " It goes to quarantine for 30 days and can be restored (rs-du --restore).", ja: "30 日間は隔離され、復元できます (rs-du --restore)。", pt: " Vai para a quarentena por 30 dias e pode ser restaurado (rs-du --restore).", zh: " 会进入隔离区保留 30 天，可以恢复（rs-du --restore）。"), (Risk::Ask, _) => t!(" Удалится навсегда.", " It is deleted permanently.", ja: "完全に削除されます。", pt: " É excluído permanentemente.", zh: " 将永久删除。"), _ => "" };
        println!("{:>3}. {:>9}  {}\n{:>16}{}{fate}", n + 1, human(i.bytes), i.label, "", i.note);
    }
    let mut sel: Vec<bool> = items.iter().map(|i| all || if safe_only { i.risk == Risk::Safe } else { i.risk != Risk::Ask }).collect();
    let sum = |sel: &[bool]| -> u64 { items.iter().zip(sel).filter(|(_, s)| **s).map(|(i, _)| i.bytes).sum() };
    println!("{}", tf!("\nМожно освободить: {} безопасно, {} всего ({} мест)", "\nYou can free: {} safely, {} in total ({} places)", ja: "\n解放できる容量: 安全なもの {}、合計 {} ({} 件)", pt: "\nVocê pode liberar: {} com segurança, {} no total ({} locais)", zh: "\n可以安全释放：{}，总计 {}（{} 处）", human(sum(&sel)), human(items.iter().map(|i| i.bytes).sum()), items.len()));
    let line = |p: &str| { print!("{p}"); let _ = std::io::stdout().flush(); let mut l = String::new(); let _ = std::io::stdin().read_line(&mut l); l };
    if !apply && !ask { println!("{}", t!("(только просмотр; --apply удалит безопасное, --apply --all всё, --apply --safe только зелёное)", "(preview only; --apply deletes the safe items, --apply --all everything, --apply --safe only the green ones)", ja: "(プレビューのみ。--apply で安全な項目を削除、--apply --all ですべて、--apply --safe で緑の項目のみ)", pt: "(apenas visualização; --apply exclui os itens seguros, --apply --all tudo, --apply --safe só os verdes)", zh: "（仅预览；--apply 删除安全项，--apply --all 删除全部，--apply --safe 只删除绿色项）")); return; }
    if !apply {
        select(&line(t!("Enter = очистить безопасное (зелёное и жёлтое), a = всё, 12,13 = безопасное + эти номера, n = отмена: ", "Enter = clean the safe items (green and yellow), a = everything, 12,13 = safe + these numbers, n = cancel:", ja: "Enter = 安全な項目 (緑と黄) を削除、a = すべて、12,13 = 安全な項目 + 指定番号、n = キャンセル:", pt: "Enter = limpar os itens seguros (verdes e amarelos), a = tudo, 12,13 = seguros + esses números, n = cancelar:", zh: "Enter = 清理安全项（绿色和黄色），a = 全部，12,13 = 安全项 + 这些编号，n = 取消：")), &mut sel);
    }
    if !sel.contains(&true) { if ask { line(t!("Ничего не удалено. Нажмите Enter, чтобы закрыть.", "Nothing was deleted. Press Enter to close.", ja: "何も削除されませんでした。Enter キーを押すと閉じます。", pt: "Nada foi excluído. Pressione Enter para fechar.", zh: "未删除任何内容。按 Enter 关闭。")); } return; }

    purge(drive, false);
    let log = env::var_os("LOCALAPPDATA").map(|l| PathBuf::from(l).join("rs-du")).unwrap_or_default();
    let _ = fs::create_dir_all(&log);
    if let Ok(mut f) = fs::File::create(log.join("clean-log.txt")) {
        for (i, _) in items.iter().zip(&sel).filter(|(_, s)| **s) { for p in &i.paths { let _ = writeln!(f, "{}\t{}", i.bytes, p.display()); } }
    }
    let (stamp, mut seq, mut freed, mut held) = (now(), 0, 0u64, 0u64);
    for (i, _) in items.iter().zip(&sel).filter(|(_, s)| **s) {
        for p in &i.paths {
            match i.how {
                How::Wipe => wipe(p),
                How::Remove => remove(p),
                How::Quarantine => { seq += 1; if !quarantine(p, stamp, seq) { println!("{}", tf!("не удалось убрать: {}", "could not remove: {}", ja: "削除できませんでした: {}", pt: "não foi possível remover: {}", zh: "无法移除：{}", p.display())); } }
            }
        }
        let left: u64 = i.paths.iter().map(|p| size_of(p)).sum();
        let gone = i.bytes.saturating_sub(left);
        if i.how == How::Quarantine { held += gone } else { freed += gone }
        if left > 0 && i.how != How::Quarantine { println!("{}", tf!("частично осталось {} (занято программой или нет прав): {}", "partially left {} (in use by a program or no rights): {}", ja: "{} が残りました (プログラムが使用中、または権限がありません): {}", pt: "{} restaram parcialmente (em uso por um programa ou sem permissão): {}", zh: "部分残留 {}（被程序占用或没有权限）：{}", human(left), i.label)); }
    }
    println!("{}", tf!("\nОсвобождено: {}. В карантине: {} (место вернётся через {QUAR_DAYS} дней или после rs-du --purge).", "\nFreed: {}. In quarantine: {} (the space returns in {QUAR_DAYS} days or after rs-du --purge).", ja: "\n解放: {}。隔離中: {} ({QUAR_DAYS} 日後、または rs-du --purge の後に容量が戻ります)。", pt: "\nLiberado: {}. Em quarentena: {} (o espaço volta em {QUAR_DAYS} dias ou após rs-du --purge).", zh: "已释放：{}。隔离区中：{}（这部分空间将在 {QUAR_DAYS} 天后或执行 rs-du --purge 后收回）。", human(freed), human(held)));
    println!("{}", tf!("Список: {}", "List: {}", ja: "一覧: {}", pt: "Lista: {}", zh: "列表：{}", log.join("clean-log.txt").display()));
    if ask { line(t!("Нажмите Enter, чтобы закрыть.", "Press Enter to close.", ja: "Enter キーを押すと閉じます。", pt: "Pressione Enter para fechar.", zh: "按 Enter 关闭。")); }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn select_defaults_all_none_and_numbers() {
        let mut s = [true, true, false, false];
        select("", &mut s); assert_eq!(s, [true, true, false, false]);
        select("4", &mut s); assert_eq!(s, [true, true, false, true]);
        select("3, 99,0", &mut s); assert_eq!(s, [true, true, true, true]);
        select("n", &mut s); assert_eq!(s, [false; 4]);
        select("a", &mut s); assert_eq!(s, [true; 4]);
    }

    #[test]
    fn rule_paths_are_absolute() {
        // a missing env var must never turn into a relative path that wipes something in the current dir
        for i in rules() { for p in &i.paths { assert!(p.is_absolute(), "{}", p.display()); } }
    }

    #[test]
    fn manifest_round_trip() {
        let d = env::temp_dir().join("rs-du-test-manifest");
        let _ = fs::create_dir_all(&d);
        let m = d.join("manifest.tsv");
        save(&m, &[(5, r"C:\a".into(), r"C:\.rs-du-quarantine\5\1_a".into())]);
        assert_eq!(parse(&fs::read_to_string(&m).unwrap(), 'c'), vec![(5, PathBuf::from(r"C:\a"), PathBuf::from(r"C:\.rs-du-quarantine\5\1_a"))]);
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn manifest_rows_outside_the_quarantine_are_dropped() {
        let t = [r"C:\Windows\System32", r"C:\.rs-du-quarantine\..\Windows", r"D:\.rs-du-quarantine\1\a"].map(|q| format!("0\tC:\\x\t{q}")).join("\n");
        assert!(parse(&t, 'C').is_empty());
    }
}
