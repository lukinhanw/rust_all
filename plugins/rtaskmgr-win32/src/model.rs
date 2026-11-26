//! Модель интерфейса: страницы, колонки, строки таблиц и их построение из данных.
use crate::{data::*, suspect, ui::*};
use std::collections::{HashMap, HashSet};

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Page {
    Processes,
    Performance,
    History,
    Startup,
    Users,
    Details,
    Services,
    Insights,
    Rules,
    Settings,
    Boost,
}

/// Пункты боковой панели: страница, символ значка, подпись.
pub static NAV: std::sync::LazyLock<[(Page, char, &'static str); 11]> = std::sync::LazyLock::new(|| [
    (Page::Processes, '\u{E9D9}', t!("Процессы", "Processes", ja: "プロセス", pt: "Processos", zh: "进程")),
    (Page::Performance, '\u{E9D2}', t!("Производительность", "Performance", ja: "パフォーマンス", pt: "Desempenho", zh: "性能")),
    (Page::History, '\u{E81C}', t!("Журнал приложений", "App history", ja: "アプリの履歴", pt: "Histórico de apps", zh: "应用历史记录")),
    (Page::Startup, '\u{EC4A}', t!("Автозагрузка", "Startup", ja: "スタートアップ", pt: "Inicialização", zh: "启动项")),
    (Page::Users, '\u{E77B}', t!("Пользователи", "Users", ja: "ユーザー", pt: "Usuários", zh: "用户")),
    (Page::Details, '\u{EA37}', t!("Подробности", "Details", ja: "詳細", pt: "Detalhes", zh: "详细信息")),
    (Page::Services, '\u{E9F5}', t!("Службы", "Services", ja: "サービス", pt: "Serviços", zh: "服务")),
    (Page::Insights, '\u{E946}', t!("Что тормозит", "What's slow", ja: "遅い原因", pt: "O que está lento", zh: "什么在变慢")),
    (Page::Rules, '\u{E7BA}', t!("Правила", "Rules", ja: "ルール", pt: "Regras", zh: "规则")),
    (Page::Settings, '\u{E713}', t!("Параметры", "Settings", ja: "設定", pt: "Configurações", zh: "设置")),
    // последней: настройки хранят номер открытой страницы, вставка в середину сдвинула бы его
    (Page::Boost, '\u{E945}', t!("Ускорение", "Speed", ja: "速度", pt: "Velocidade", zh: "速度")),
]);

pub fn page_title(p: Page) -> &'static str {
    NAV.iter().find(|n| n.0 == p).map_or("", |n| n.2)
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Align {
    Left,
    Right,
}

#[derive(Clone, Copy)]
pub struct Col {
    pub title: &'static str,
    pub w: f64, // логические px; 0 — всё оставшееся место
    pub align: Align,
    pub metric: Option<Metric>, // колонка с агрегатом в шапке и тепловой подсветкой
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Metric {
    Cpu,
    Mem,
    Disk,
    Net,
    Gpu,
}

const fn col(title: &'static str, w: f64, align: Align) -> Col {
    Col { title, w, align, metric: None }
}
const fn mcol(title: &'static str, m: Metric) -> Col {
    Col { title, w: 84.0, align: Align::Right, metric: Some(m) }
}

pub fn columns(p: Page) -> Vec<Col> {
    use Align::*;
    use Metric::*;
    match p {
        Page::Processes => vec![
            col(t!("Имя", "Name", ja: "名前", pt: "Nome", zh: "名称"), 270.0, Left),
            col(t!("Состояние", "Status", ja: "状態", pt: "Status", zh: "状态"), 112.0, Left),
            mcol(t!("ЦП", "CPU", ja: "CPU", pt: "CPU", zh: "CPU"), Cpu),
            mcol(t!("Память", "Memory", ja: "メモリ", pt: "Memória", zh: "内存"), Mem),
            mcol(t!("Диск", "Disk", ja: "ディスク", pt: "Disco", zh: "磁盘"), Disk),
            mcol(t!("Сеть", "Network", ja: "ネットワーク", pt: "Rede", zh: "网络"), Net),
            mcol("GPU", Gpu),
            col(t!("Подозрительное", "Suspicious", ja: "不審", pt: "Suspeito", zh: "可疑"), 140.0, Left),
        ],
        Page::Users => vec![
            col(t!("Пользователь", "User", ja: "ユーザー", pt: "Usuário", zh: "用户"), 270.0, Left),
            col(t!("Процессов", "Processes", ja: "プロセス", pt: "Processos", zh: "进程"), 112.0, Right),
            mcol(t!("ЦП", "CPU", ja: "CPU", pt: "CPU", zh: "CPU"), Cpu),
            mcol(t!("Память", "Memory", ja: "メモリ", pt: "Memória", zh: "内存"), Mem),
            mcol(t!("Диск", "Disk", ja: "ディスク", pt: "Disco", zh: "磁盘"), Disk),
            mcol(t!("Сеть", "Network", ja: "ネットワーク", pt: "Rede", zh: "网络"), Net),
            mcol("GPU", Gpu),
        ],
        Page::Details => vec![col(t!("Имя", "Name", ja: "名前", pt: "Nome", zh: "名称"), 220.0, Left), col("PID", 70.0, Right), col(t!("Состояние", "Status", ja: "状態", pt: "Status", zh: "状态"), 100.0, Left), col(t!("ЦП", "CPU", ja: "CPU", pt: "CPU", zh: "CPU"), 70.0, Right), col(t!("Память", "Memory", ja: "メモリ", pt: "Memória", zh: "内存"), 110.0, Right), col(t!("Диск", "Disk", ja: "ディスク", pt: "Disco", zh: "磁盘"), 100.0, Right), col(t!("Командная строка", "Command line", ja: "コマンド ライン", pt: "Linha de comando", zh: "命令行"), 0.0, Left)],
        Page::Services => vec![col(t!("Имя", "Name", ja: "名前", pt: "Nome", zh: "名称"), 200.0, Left), col("PID", 70.0, Right), col(t!("Описание", "Description", ja: "説明", pt: "Descrição", zh: "描述"), 360.0, Left), col(t!("Состояние", "Status", ja: "状態", pt: "Status", zh: "状态"), 110.0, Left)],
        Page::Startup => vec![col(t!("Имя", "Name", ja: "名前", pt: "Nome", zh: "名称"), 240.0, Left), col(t!("Издатель", "Publisher", ja: "発行元", pt: "Editor", zh: "发布者"), 180.0, Left), col(t!("Состояние", "Status", ja: "状態", pt: "Status", zh: "状态"), 100.0, Left), col(t!("Замедляет загрузку", "Slows boot by", ja: "起動を遅らせる時間", pt: "Atrasa a inicialização em", zh: "拖慢启动"), 150.0, Right), col(t!("Команда", "Command", ja: "コマンド", pt: "Comando", zh: "命令"), 0.0, Left)],
        Page::History => vec![col(t!("Время", "Time", ja: "時刻", pt: "Hora", zh: "时间"), 150.0, Left), col(t!("ЦП %", "CPU %", ja: "CPU %", pt: "CPU %", zh: "CPU %"), 80.0, Right), col(t!("Память %", "Memory %", ja: "メモリ %", pt: "Memória %", zh: "内存 %"), 90.0, Right), col(t!("Ввод-вывод КБ/с", "I/O KB/s", ja: "I/O KB/秒", pt: "E/S KB/s", zh: "I/O KB/s"), 130.0, Right), col(t!("Сеть КБ/с", "Network KB/s", ja: "ネットワーク KB/秒", pt: "Rede KB/s", zh: "网络 KB/s"), 110.0, Right), col(t!("Основной потребитель", "Top consumer", ja: "最大の使用元", pt: "Maior consumidor", zh: "占用最多"), 0.0, Left)],
        Page::Boost => vec![col(t!("Что", "What", ja: "対象", pt: "O quê", zh: "项目"), 320.0, Left), col(t!("Состояние", "Status", ja: "状態", pt: "Status", zh: "状态"), 220.0, Left), col(t!("Что делает", "What it does", ja: "動作", pt: "O que faz", zh: "作用"), 0.0, Left)],
        Page::Rules => vec![col(t!("Процесс", "Process", ja: "プロセス", pt: "Processo", zh: "进程"), 220.0, Left), col(t!("Условие", "Condition", ja: "条件", pt: "Condição", zh: "条件"), 280.0, Left), col(t!("Действие", "Action", ja: "操作", pt: "Ação", zh: "操作"), 200.0, Left), col(t!("Сработало, раз", "Times triggered", ja: "発動回数", pt: "Vezes acionada", zh: "触发次数"), 130.0, Right)],
        _ => vec![],
    }
}

/// Сортировка по умолчанию: (индекс колонки, по убыванию).
pub fn default_sort(p: Page) -> (usize, bool) {
    match p {
        Page::Processes | Page::Users => (2, true), // ЦП
        Page::Details => (3, true),
        Page::History => (0, true),
        _ => (0, false),
    }
}

#[derive(Clone, Default)]
pub struct Row {
    pub key: String,
    pub depth: u8,
    pub expandable: bool,
    pub expanded: bool,
    pub icon_path: Option<String>,
    pub cells: Vec<String>,
    pub heat: Vec<Option<(f64, f64)>>, // (значение, шкала) для колонок-метрик
    pub sortv: Vec<f64>,               // числовые ключи сортировки; NaN — сортировать как текст
    pub sev: u8,
    pub tip: String,
    pub pids: Vec<u32>,
    pub aux: String, // служебное: имя службы, ключ автозагрузки и т.п.
}

pub fn dec(v: f64, digits: usize) -> String {
    let s = format!("{:.*}", digits, v);
    if dot!() { s } else { s.replace('.', ",") }
}

pub fn fmt_cpu(v: f32) -> String {
    if v < 0.05 { "0%".into() } else { format!("{}%", dec(v as f64, 1)) }
}

pub fn fmt_mem_ru(b: u64) -> String {
    let mb = b as f64 / 1_048_576.0;
    if mb >= 1000.0 { tf!("{} МБ", "{} MB", ja: "{} MB", pt: "{} MB", zh: "{} MB", { let g = group_thousands(mb as u64); if dot!() { g } else { g.replace(',', " ") } }) } else if mb >= 100.0 { tf!("{} МБ", "{} MB", ja: "{} MB", pt: "{} MB", zh: "{} MB", dec(mb, 0)) } else { tf!("{} МБ", "{} MB", ja: "{} MB", pt: "{} MB", zh: "{} MB", dec(mb, 1)) }
}

pub fn fmt_disk_ru(bps: f64) -> String {
    let mb = bps / 1_048_576.0;
    if mb < 0.05 { t!("0 МБ/с", "0 MB/s", ja: "0 MB/秒", pt: "0 MB/s", zh: "0 MB/s").into() } else { tf!("{} МБ/с", "{} MB/s", ja: "{} MB/秒", pt: "{} MB/s", zh: "{} MB/s", dec(mb, 1)) }
}

pub fn fmt_net_ru(bps: f64, known: bool) -> String {
    if !known {
        return "—".into();
    }
    let mbit = bps * 8.0 / 1e6;
    if mbit < 0.05 { t!("0 Мбит/с", "0 Mbps", ja: "0 Mbps", pt: "0 Mbps", zh: "0 Mbps").into() } else { tf!("{} Мбит/с", "{} Mbps", ja: "{} Mbps", pt: "{} Mbps", zh: "{} Mbps", dec(mbit, 1)) }
}

/// Описание/имя для отображения: описание файла, если известно, иначе имя без «.exe».
fn friendly(d: &Data, p: &Proc) -> String {
    if let Some(r) = p.exe.as_deref().and_then(|e| d.res.get(e))
        && !r.desc.is_empty() {
            return r.desc;
        }
    p.name.strip_suffix(".exe").or_else(|| p.name.strip_suffix(".EXE")).unwrap_or(&p.name).to_string()
}

fn svc_display(d: &Data, pid: u32) -> Option<String> {
    let name = d.svc_pids.get(&pid)?.first()?.clone();
    Some(d.svcs.iter().find(|s| s.name == name).map_or(name, |s| s.display.clone()))
}

fn child_name(d: &Data, p: &Proc) -> String {
    if p.name.eq_ignore_ascii_case("svchost.exe")
        && let Some(s) = svc_display(d, p.pid) {
            return tf!("Узел службы: {s}", "Service host: {s}", ja: "サービス ホスト: {s}", pt: "Host de serviço: {s}", zh: "服务主机：{s}");
        }
    if let Some(t) = &p.title {
        return t.clone();
    }
    friendly(d, p)
}

fn tip_for(procs: &[&Proc], group: bool) -> String {
    let mut by_text: Vec<(u8, String, Vec<u32>)> = vec![];
    for p in procs {
        for r in &p.reasons {
            match by_text.iter_mut().find(|e| e.1 == r.text) {
                Some(e) => e.2.push(p.pid),
                None => by_text.push((r.sev, r.text.clone(), vec![p.pid])),
            }
        }
    }
    if by_text.is_empty() {
        return String::new();
    }
    by_text.sort_by_key(|e| std::cmp::Reverse(e.0));
    let total = by_text.len();
    let mut s = String::from(t!("Почему помечено:", "Why flagged:", ja: "フラグが付いた理由:", pt: "Por que foi sinalizado:", zh: "标记原因："));
    for (sev, t, pids) in by_text.into_iter().take(6) {
        let who = if pids.len() > 1 { tf!(" (процессов: {})", " (processes: {})", ja: " (プロセス: {})", pt: " (processos: {})", zh: " （进程数：{}）", pids.len()) } else if group && procs.len() > 1 { format!(" [PID {}]", pids[0]) } else { String::new() };
        s += &format!("\n{} — {t}{who}", suspect::sev_word(sev));
    }
    if total > 6 {
        s += &tf!("\n… и ещё причин: {}", "\n… and {} more reasons", ja: "\n… ほか {} 件の理由", pt: "\n… e mais {} motivos", zh: "\n…以及另外 {} 个原因", total - 6);
    }
    s
}

fn proc_row(d: &Data, key: String, depth: u8, name: String, members: &[&Proc], expandable: bool, expanded: bool) -> Row {
    let cpu: f32 = members.iter().map(|p| p.cpu).sum();
    let mem: u64 = members.iter().map(|p| p.mem).sum();
    let disk: f64 = members.iter().map(|p| p.disk).sum();
    let net: f64 = members.iter().map(|p| p.net).sum();
    let gpu: f32 = members.iter().map(|p| p.gpu).fold(0.0, f32::max);
    let sev = members.iter().flat_map(|p| p.reasons.iter()).map(|r| r.sev).max().unwrap_or(0);
    let status = if members.iter().all(|p| d.suspended.contains(&p.pid)) {
        t!("Приостановлен", "Suspended", ja: "中断中", pt: "Suspenso", zh: "已挂起")
    } else if members.iter().all(|p| d.efficiency.contains(&p.pid)) {
        t!("Эффективность", "Efficiency", ja: "効率", pt: "Eficiência", zh: "能效")
    } else {
        ""
    };
    let etw = d.etw.is_some();
    Row {
        key,
        depth,
        expandable,
        expanded,
        icon_path: members.iter().find_map(|p| p.exe.clone()),
        cells: vec![name, status.into(), fmt_cpu(cpu), fmt_mem_ru(mem), fmt_disk_ru(disk), fmt_net_ru(net, etw), if gpu >= 0.05 { format!("{}%", dec(gpu as f64, 0)) } else { "0%".into() }, suspect::label(sev).to_string()],
        heat: vec![None, None, Some((cpu as f64, 40.0)), Some((mem as f64, 1.5e9)), Some((disk, 5e6)), Some((net, 5e6)), Some((gpu as f64, 40.0)), None],
        sortv: vec![f64::NAN, f64::NAN, cpu as f64, mem as f64, disk, net, gpu as f64, sev as f64],
        sev,
        tip: tip_for(members, members.len() > 1),
        pids: members.iter().map(|p| p.pid).collect(),
        aux: String::new(),
    }
}

pub struct BuildCtx<'a> {
    pub page: Page,
    pub search: &'a str,
    pub expanded: &'a HashSet<String>,
    pub sort: (usize, bool),
}

fn matches(d: &Data, p: &Proc, q: &str) -> bool {
    p.name.to_lowercase().contains(q)
        || p.pid.to_string() == q
        || friendly(d, p).to_lowercase().contains(q)
        || p.exe.as_deref().and_then(|e| d.res.get(e)).is_some_and(|r| r.company.to_lowercase().contains(q))
        || p.title.as_deref().is_some_and(|t| t.to_lowercase().contains(q))
}

pub fn build_rows(d: &Data, c: &BuildCtx) -> Vec<Row> {
    let q = c.search.trim().to_lowercase();
    let mut rows: Vec<Row> = match c.page {
        Page::Processes => {
            let mut groups: HashMap<String, Vec<&Proc>> = HashMap::new();
            for p in &d.procs {
                groups.entry(group_key(&p.name)).or_default().push(p);
            }
            let mut out: Vec<(Row, Vec<Row>)> = vec![];
            for (key, mut members) in groups {
                if !q.is_empty() && !members.iter().any(|p| matches(d, p, &q) || suspect::label(p.reasons.iter().map(|r| r.sev).max().unwrap_or(0)).to_lowercase().contains(&q)) {
                    continue;
                }
                members.sort_by(|a, b| b.cpu.total_cmp(&a.cpu));
                let multi = members.len() > 1;
                let base = if members[0].name.eq_ignore_ascii_case("svchost.exe") { t!("Узел службы", "Service host", ja: "サービス ホスト", pt: "Host de serviço", zh: "服务主机").to_string() } else { friendly(d, members[0]) };
                let name = if multi { format!("{base} ({})", members.len()) } else if members[0].name.eq_ignore_ascii_case("svchost.exe") { child_name(d, members[0]) } else { members[0].title.clone().filter(|_| false).unwrap_or(base) };
                let expanded = multi && c.expanded.contains(&key);
                let head = proc_row(d, key.clone(), 0, name, &members, multi, expanded);
                let kids: Vec<Row> = if expanded {
                    members.iter().map(|p| proc_row(d, format!("{key}#{}", p.pid), 1, child_name(d, p), &[p], false, false)).collect()
                } else {
                    vec![]
                };
                out.push((head, kids));
            }
            sort_pairs(&mut out, c.sort);
            out.into_iter().flat_map(|(h, k)| std::iter::once(h).chain(k)).collect()
        }
        Page::Users => {
            let mut groups: HashMap<String, Vec<&Proc>> = HashMap::new();
            for p in &d.procs {
                groups.entry(p.user.clone()).or_default().push(p);
            }
            let mut out: Vec<(Row, Vec<Row>)> = vec![];
            for (user, mut members) in groups {
                if !q.is_empty() && !user.to_lowercase().contains(&q) && !members.iter().any(|p| matches(d, p, &q)) {
                    continue;
                }
                members.sort_by(|a, b| b.cpu.total_cmp(&a.cpu));
                let key = format!("user:{user}");
                let expanded = c.expanded.contains(&key);
                let mut head = proc_row(d, key.clone(), 0, user.clone(), &members, true, expanded);
                head.cells[1] = members.len().to_string();
                head.sortv[1] = members.len() as f64;
                head.icon_path = None;
                head.tip = String::new();
                let kids: Vec<Row> = if expanded { members.iter().map(|p| proc_row(d, format!("{key}#{}", p.pid), 1, child_name(d, p), &[p], false, false)).collect() } else { vec![] };
                out.push((head, kids));
            }
            sort_pairs(&mut out, c.sort);
            out.into_iter().flat_map(|(h, k)| std::iter::once(h).chain(k)).collect()
        }
        Page::Details => d
            .procs
            .iter()
            .filter(|p| q.is_empty() || matches(d, p, &q))
            .map(|p| Row {
                key: p.pid.to_string(),
                icon_path: p.exe.clone(),
                cells: vec![p.name.clone(), p.pid.to_string(), (if d.suspended.contains(&p.pid) { t!("Приостановлен", "Suspended", ja: "中断中", pt: "Suspenso", zh: "已挂起") } else { t!("Выполняется", "Running", ja: "実行中", pt: "Em execução", zh: "正在运行") }).into(), fmt_cpu(p.cpu), fmt_mem_ru(p.mem), fmt_disk_ru(p.disk), process_cmd(d, p.pid)],
                sortv: vec![f64::NAN, p.pid as f64, f64::NAN, p.cpu as f64, p.mem as f64, p.disk, f64::NAN],
                pids: vec![p.pid],
                heat: vec![None; 7],
                ..Default::default()
            })
            .collect(),
        Page::Services => d
            .svcs
            .iter()
            .filter(|s| q.is_empty() || s.name.to_lowercase().contains(&q) || s.display.to_lowercase().contains(&q))
            .map(|s| Row {
                key: s.name.clone(),
                cells: vec![s.name.clone(), if s.pid != 0 { s.pid.to_string() } else { String::new() }, s.display.clone(), (if s.running { t!("Выполняется", "Running", ja: "実行中", pt: "Em execução", zh: "正在运行") } else { t!("Остановлена", "Stopped", ja: "停止", pt: "Parado", zh: "已停止") }).into()],
                sortv: vec![f64::NAN, s.pid as f64, f64::NAN, f64::NAN],
                pids: vec![s.pid],
                heat: vec![None; 4],
                aux: s.name.clone(),
                ..Default::default()
            })
            .collect(),
        Page::Startup => d
            .startup
            .iter()
            .filter(|s| q.is_empty() || s.name.to_lowercase().contains(&q) || s.command.to_lowercase().contains(&q))
            .map(|s| {
                let exe = exe_from_command(&s.command);
                let company = exe.as_deref().and_then(|e| d.res.get(e)).map(|r| r.company).unwrap_or_default();
                // без администратора журнал загрузки не читается: «нет данных», а не «не замедляет»
                let impact = if d.is_admin { crate::startup::boot_impact(s, crate::startup::boot_log()) } else { None };
                let impact_txt = match impact {
                    Some(ms) => tf!("+{} с", "+{} s", ja: "+{} 秒", pt: "+{} s", zh: "+{} 秒", dec(ms as f64 / 1000.0, 1)),
                    None if d.is_admin => "—".into(),
                    None => t!("нужен администратор", "administrator needed", ja: "管理者権限が必要です", pt: "requer administrador", zh: "需要管理员权限").into(),
                };
                Row {
                    key: s.key(),
                    icon_path: exe,
                    cells: vec![s.name.clone(), company, (if s.enabled { t!("Включено", "Enabled", ja: "有効", pt: "Ativada", zh: "已启用") } else { t!("Отключено", "Disabled", ja: "無効", pt: "Desativada", zh: "已禁用") }).into(), impact_txt, s.command.clone()],
                    sortv: vec![f64::NAN, f64::NAN, f64::NAN, impact.unwrap_or(0) as f64, f64::NAN],
                    heat: vec![None; 5],
                    tip: tf!("Источник: {}.{}", "Source: {}.{}", ja: "ソース: {}.{}", pt: "Fonte: {}.{}", zh: "来源：{}.{}", s.source_label(), if s.needs_admin() && !d.is_admin { t!(" Для изменения нужны права администратора.", " Administrator rights are needed to change it.", ja: " 変更するには管理者権限が必要です。", pt: " São necessários direitos de administrador para alterar isso.", zh: " 更改此项需要管理员权限。") } else { "" }),
                    ..Default::default()
                }
            })
            .collect(),
        Page::Boost => crate::boost::ALL
            .iter()
            .map(|&id| {
                use crate::boost::State;
                let st = crate::boost::state(id, d.is_admin);
                let text = match &st {
                    State::On => t!("Включено", "On", ja: "オン", pt: "Ligado", zh: "开").to_string(),
                    State::Off => t!("Выключено", "Off", ja: "オフ", pt: "Desligado", zh: "关").to_string(),
                    State::Busy => t!("Выполняется…", "Running…", ja: "実行中…", pt: "Executando…", zh: "正在运行…").to_string(),
                    State::Info(s) => s.clone(),
                    State::Unavailable(s) => tf!("Недоступно: {s}", "Unavailable: {s}", ja: "利用できません: {s}", pt: "Indisponível: {s}", zh: "不可用：{s}"),
                };
                Row {
                    key: id.key().into(),
                    cells: vec![id.title().into(), text, id.desc().into()],
                    sortv: vec![f64::NAN; 3],
                    heat: vec![None; 3],
                    tip: id.desc().into(),
                    ..Default::default()
                }
            })
            .collect(),
        Page::History => d
            .hist
            .iter()
            .filter(|h| q.is_empty() || h.iter().any(|c| c.to_lowercase().contains(&q)))
            .map(|h| Row {
                key: h[0].clone(),
                cells: h.clone(),
                sortv: vec![f64::NAN, h[1].parse().unwrap_or(0.0), h[2].parse().unwrap_or(0.0), h[3].parse().unwrap_or(0.0), h[4].parse().unwrap_or(0.0), f64::NAN],
                heat: vec![None; 6],
                ..Default::default()
            })
            .collect(),
        Page::Rules => d
            .rules
            .iter()
            .enumerate()
            .map(|(i, r)| Row {
                key: i.to_string(),
                cells: vec![
                    if r.proc == "*" { t!("любой", "any", ja: "任意", pt: "qualquer", zh: "任意").into() } else { r.proc.clone() },
                    tf!("ЦП > {:.0}% дольше {} с", "CPU > {:.0}% for over {} s", ja: "CPU {:.0}% 超が {} 秒を超えて継続", pt: "CPU > {:.0}% por mais de {} s", zh: "CPU 超过 {:.0}% 持续 {} 秒以上", r.cpu, r.secs),
                    (if r.act == crate::store::RuleAct::Lower { t!("Понизить приоритет", "Lower priority", ja: "優先度を下げる", pt: "Reduzir a prioridade", zh: "降低优先级") } else { t!("Уведомить", "Notify", ja: "通知", pt: "Avisar", zh: "通知") }).into(),
                    r.hits.to_string(),
                ],
                sortv: vec![f64::NAN, f64::NAN, f64::NAN, r.hits as f64],
                heat: vec![None; 4],
                ..Default::default()
            })
            .collect(),
        _ => vec![],
    };
    if !matches!(c.page, Page::Processes | Page::Users | Page::Boost) {
        let (ci, desc) = c.sort;
        rows.sort_by(|a, b| {
            let o = cmp_cells(a, b, ci);
            if desc { o.reverse() } else { o }
        });
    }
    rows
}

fn cmp_cells(a: &Row, b: &Row, ci: usize) -> std::cmp::Ordering {
    let (x, y) = (a.sortv.get(ci).copied().unwrap_or(f64::NAN), b.sortv.get(ci).copied().unwrap_or(f64::NAN));
    if x.is_nan() || y.is_nan() {
        a.cells.get(ci).map(|s| s.to_lowercase()).cmp(&b.cells.get(ci).map(|s| s.to_lowercase()))
    } else {
        x.total_cmp(&y)
    }
}

fn sort_pairs(v: &mut [(Row, Vec<Row>)], sort: (usize, bool)) {
    let (ci, desc) = sort;
    let ord = |a: &Row, b: &Row| {
        let o = cmp_cells(a, b, ci);
        if desc { o.reverse() } else { o }
    };
    v.sort_by(|a, b| ord(&a.0, &b.0));
    for (_, kids) in v.iter_mut() {
        kids.sort_by(|a, b| ord(a, b));
    }
}

/// Путь к исполняемому файлу из команды автозагрузки (в кавычках или до первого пробела после «.exe»).
pub fn exe_from_command(cmd: &str) -> Option<String> {
    let c = cmd.trim();
    let path = if let Some(rest) = c.strip_prefix('"') {
        rest.split('"').next()?.to_string()
    } else {
        let l = c.to_lowercase();
        let end = l.find(".exe").map(|i| i + 4).unwrap_or(c.len());
        c[..end].to_string()
    };
    let path = path.replace("%windir%", &std::env::var("WINDIR").unwrap_or_default());
    (!path.is_empty() && path.contains('\\')).then_some(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exe_extraction() {
        assert_eq!(exe_from_command(r#""C:\Program Files\X\x.exe" --silent"#).as_deref(), Some(r"C:\Program Files\X\x.exe"));
        assert_eq!(exe_from_command(r"C:\Tools\a.exe -k b").as_deref(), Some(r"C:\Tools\a.exe"));
        assert_eq!(exe_from_command("cmd /c foo"), None);
    }

    #[test]
    fn number_formats_use_comma() {
        assert_eq!(fmt_cpu(2.04), "2,0%");
        assert_eq!(fmt_cpu(0.01), "0%");
        assert_eq!(fmt_mem_ru(3_746_000_000), "3 572 МБ");
        assert_eq!(fmt_mem_ru(73 * 1_048_576 + 100_000), "73,1 МБ");
        assert_eq!(fmt_disk_ru(0.0), "0 МБ/с");
        assert_eq!(fmt_net_ru(1.0, false), "—");
    }

    #[test]
    fn every_page_with_table_has_valid_default_sort() {
        for p in [Page::Processes, Page::Users, Page::Details, Page::Services, Page::Startup, Page::History, Page::Rules] {
            assert!(default_sort(p).0 < columns(p).len(), "{p:?}");
        }
    }

    #[test]
    fn users_page_groups_processes_by_account() {
        let mut d = Data::new(crate::store::Settings::default());
        d.refresh();
        let me = d.procs.iter().find(|p| p.pid == std::process::id()).expect("свой процесс").user.clone();
        assert!(!me.is_empty() && me != "СИСТЕМА", "свой токен должен читаться: {me}");
        let mut exp = HashSet::new();
        let rows = build_rows(&d, &BuildCtx { page: Page::Users, search: "", expanded: &exp, sort: default_sort(Page::Users) });
        assert!(rows.iter().all(|r| r.depth == 0 && r.expandable));
        let mine = rows.iter().find(|r| r.cells[0] == me).expect("строка текущего пользователя");
        assert!(mine.pids.contains(&std::process::id()));
        exp.insert(mine.key.clone());
        let rows2 = build_rows(&d, &BuildCtx { page: Page::Users, search: "", expanded: &exp, sort: default_sort(Page::Users) });
        assert!(rows2.iter().filter(|r| r.depth == 1).count() == mine.pids.len());
    }

    #[test]
    fn processes_rows_group_and_expand() {
        let mut d = Data::new(crate::store::Settings::default());
        d.refresh();
        let exp: HashSet<String> = HashSet::new();
        let ctx = BuildCtx { page: Page::Processes, search: "", expanded: &exp, sort: default_sort(Page::Processes) };
        let rows = build_rows(&d, &ctx);
        assert!(!rows.is_empty() && rows.iter().all(|r| r.depth == 0));
        let multi = rows.iter().find(|r| r.expandable).expect("есть группа из нескольких процессов");
        let mut exp2 = HashSet::new();
        exp2.insert(multi.key.clone());
        let ctx2 = BuildCtx { page: Page::Processes, search: "", expanded: &exp2, sort: default_sort(Page::Processes) };
        let rows2 = build_rows(&d, &ctx2);
        assert!(rows2.len() > rows.len());
        assert!(rows2.iter().any(|r| r.depth == 1));
        // поиск по PID находит нужную группу
        let pid = std::process::id().to_string();
        let ctx3 = BuildCtx { page: Page::Processes, search: &pid, expanded: &exp, sort: default_sort(Page::Processes) };
        assert!(build_rows(&d, &ctx3).iter().any(|r| r.pids.contains(&std::process::id())));
    }
}
