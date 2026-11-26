//! Who the per-user settings belong to. UAC can be answered with another administrator's password: the installer then runs as
//! that account, and its HKCU, `$PROFILE` and scheduled-task user are the wrong ones. The right account is the one whose
//! explorer.exe (or, with Explorer replaced, topbar / rust-explorer) runs in this session; its SID goes to every child in
//! WINRUST_SID, and `out()` aims `HKCU\...` at `HKU\<sid>` while it is set.
use crate::*;
use std::sync::OnceLock;
use windows_sys::Win32::{
    Foundation::{CloseHandle, INVALID_HANDLE_VALUE},
    Security::{GetTokenInformation, LookupAccountNameW, SID_NAME_USE, TOKEN_QUERY, TOKEN_USER, TokenUser},
    System::{
        Diagnostics::ToolHelp::{CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS},
        RemoteDesktop::{
            ProcessIdToSessionId, WTS_CURRENT_SERVER_HANDLE, WTS_INFO_CLASS, WTSDomainName, WTSFreeMemory, WTSQuerySessionInformationW, WTSUserName,
        },
        Threading::{GetCurrentProcessId, OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION},
    },
};

pub(crate) struct Session {
    pub sid: String,
    /// The elevated token belongs to someone else: only then is WINRUST_SID set, so the usual case keeps using plain HKCU.
    pub other: bool,
}

static SESSION: OnceLock<Result<Session, String>> = OnceLock::new();

/// Base64 of UTF-16LE, as `powershell -EncodedCommand` takes it.
fn b64(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for c in bytes.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        s.push(T[(n >> 18) as usize & 63] as char);
        s.push(T[(n >> 12) as usize & 63] as char);
        s.push(if c.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
        s.push(if c.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
    s
}

/// Runs a PowerShell script and returns its stdout. Scripts with double quotes in them (`-Filter "Name='…'"`) do not
/// survive the quoting a Rust `Command` puts around an argument, so `-Command <script>` is not an option for them;
/// `-EncodedCommand` takes Base64 of the UTF-16LE script and never re-quotes anything.
fn ps(script: &str) -> Result<String, String> {
    let utf16: Vec<u8> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
    out("powershell", &["-NoProfile", "-NonInteractive", "-EncodedCommand", &b64(&utf16)])
}

/// "S-1-5-21-…" of a SID.
fn sid_text(sid: *mut core::ffi::c_void) -> String {
    let p = sid as *const u8;
    unsafe {
        let (revision, count) = (*p, *p.add(1) as usize);
        let mut authority = 0u64;
        for i in 0..6 {
            authority = authority << 8 | *p.add(2 + i) as u64;
        }
        let mut s = format!("S-{revision}-{authority}");
        for i in 0..count {
            let o = 8 + i * 4;
            let v = u32::from_le_bytes([*p.add(o), *p.add(o + 1), *p.add(o + 2), *p.add(o + 3)]);
            s.push('-');
            s.push_str(&v.to_string());
        }
        s
    }
}

/// The SID of a process's user (PROCESS_QUERY_LIMITED_INFORMATION + TOKEN_QUERY are enough, no admin needed).
fn process_sid(pid: u32) -> Option<String> {
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if h.is_null() {
            return None;
        }
        let mut token = std::ptr::null_mut();
        let ok = OpenProcessToken(h, TOKEN_QUERY, &mut token);
        CloseHandle(h);
        if ok == 0 || token.is_null() {
            return None;
        }
        let mut need = 0u32;
        GetTokenInformation(token, TokenUser, std::ptr::null_mut(), 0, &mut need);
        let mut buf = vec![0u8; need.max(1) as usize];
        let ok = GetTokenInformation(token, TokenUser, buf.as_mut_ptr().cast(), need, &mut need);
        CloseHandle(token);
        if ok == 0 || need < size_of::<TOKEN_USER>() as u32 {
            return None;
        }
        let user = &*(buf.as_ptr() as *const TOKEN_USER);
        Some(sid_text(user.User.Sid))
    }
}

/// "DOMAIN\user" of a session, straight from the terminal-services session table: no process list, no WMI, and it
/// answers in sessions where enumerating processes is restricted or the shell is not a process we know.
fn wts_user(session: u32) -> Option<String> {
    unsafe fn query(session: u32, class: WTS_INFO_CLASS) -> Option<String> {
        let mut buf: *mut u16 = std::ptr::null_mut();
        let mut len = 0u32;
        if unsafe { WTSQuerySessionInformationW(WTS_CURRENT_SERVER_HANDLE, session, class, &mut buf, &mut len) } == 0 || buf.is_null() {
            return None;
        }
        let units = unsafe { std::slice::from_raw_parts(buf, (len / 2) as usize) };
        let s = String::from_utf16_lossy(&units[..units.iter().position(|&c| c == 0).unwrap_or(units.len())]);
        unsafe { WTSFreeMemory(buf as *mut core::ffi::c_void) };
        Some(s)
    }
    let name = unsafe { query(session, WTSUserName) }?;
    if name.is_empty() {
        return None;
    }
    let domain = unsafe { query(session, WTSDomainName) }.unwrap_or_default();
    Some(if domain.is_empty() { name } else { format!("{domain}\\{name}") })
}

/// The SID of an account name ("DOMAIN\user" or "user").
fn name_sid(account: &str) -> Option<String> {
    let name = wide(account);
    unsafe {
        let mut sid = [0u8; 68]; // SECURITY_MAX_SID_SIZE
        let mut len = sid.len() as u32;
        let mut domain = [0u16; 256];
        let mut dlen = domain.len() as u32;
        let mut kind: SID_NAME_USE = 0;
        let ok = LookupAccountNameW(std::ptr::null(), name.as_ptr(), sid.as_mut_ptr().cast(), &mut len, domain.as_mut_ptr(), &mut dlen, &mut kind);
        (ok != 0).then(|| sid_text(sid.as_mut_ptr().cast()))
    }
}

/// The SID of whoever runs this session's shell: explorer.exe, or topbar / rust-explorer / rs-shell when Explorer is
/// replaced. A plain Win32 probe: the earlier one asked PowerShell, and on a machine where that child cannot see the
/// process list (WMI is not always reachable from an elevated installer) it answered "no shell" although the shell was
/// right there.
fn shell_sid(session: u32) -> Option<String> {
    const NAMES: [&str; 4] = ["explorer.exe", "topbar.exe", "rust-explorer.exe", "rs-shell.exe"];
    let mut best: Option<(usize, bool, String)> = None; // (rank of the name, same session, SID)
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS as u32, 0);
        if snap == INVALID_HANDLE_VALUE {
            return None;
        }
        let mut e: PROCESSENTRY32W = std::mem::zeroed();
        e.dwSize = size_of::<PROCESSENTRY32W>() as u32;
        let mut more = Process32FirstW(snap, &mut e) != 0;
        while more {
            let len = e.szExeFile.iter().position(|&c| c == 0).unwrap_or(e.szExeFile.len());
            let name = String::from_utf16_lossy(&e.szExeFile[..len]);
            if let Some(rank) = NAMES.iter().position(|n| n.eq_ignore_ascii_case(&name)) {
                let mut s = 0u32;
                let same = ProcessIdToSessionId(e.th32ProcessID, &mut s) != 0 && s == session;
                if let Some(sid) = process_sid(e.th32ProcessID) {
                    let better = match &best {
                        None => true,
                        Some((r, bs, _)) => (same, rank) > (*bs, *r) || (same == *bs && rank < *r),
                    };
                    if better {
                        best = Some((rank, same, sid));
                    }
                }
            }
            more = Process32NextW(snap, &mut e) != 0;
        }
        CloseHandle(snap);
    }
    best.map(|(_, _, sid)| sid)
}

fn detect() -> Result<Session, String> {
    unsafe {
        let mut session = 0u32;
        ProcessIdToSessionId(GetCurrentProcessId(), &mut session);
        // The session table first: it is a query about the session, not an enumeration, so it answers even where
        // walking the process list does not (a restricted token, an unreachable WMI, an unknown shell executable).
        let sid = wts_user(session).and_then(|a| name_sid(&a)).or_else(|| shell_sid(session));
        let token = process_sid(GetCurrentProcessId()).ok_or_else(|| t!("не удалось прочитать SID учётной записи установщика", "could not read the installer's own account SID", ja: "インストーラーのアカウント SID を読み取れませんでした", pt: "não foi possível ler o SID da conta do instalador", zh: "无法读取安装程序自身账户的 SID").to_string())?;
        // Neither way worked: change nothing, since the elevated account may be another admin's (init_soft lets uninstall go on).
        let sid = sid.ok_or_else(|| t!("не удалось определить пользователя сеанса", "could not determine the signed-in user", ja: "サインイン中のユーザーを特定できませんでした", pt: "não foi possível identificar o usuário conectado", zh: "无法确定登录用户").to_string())?;
        if matches!(sid.as_str(), "S-1-5-18" | "S-1-5-19" | "S-1-5-20") {
            return Err(t!("оболочка сеанса запущена не от обычного пользователя (SYSTEM или служба)", "the session shell does not run as a normal user (SYSTEM or a service)", ja: "セッションのシェルが通常のユーザーとして実行されていません (SYSTEM またはサービス)", pt: "o shell da sessão não roda como um usuário comum (SYSTEM ou um serviço)", zh: "会话外壳未以普通用户身份运行（SYSTEM 或服务）").into());
        }
        Ok(Session { other: sid != token, sid })
    }
}

/// Settles the target user once and, when it is not the elevated account, tells the children. An Err means: change nothing.
pub(crate) fn init() -> Result<&'static Session, String> {
    let s = SESSION.get_or_init(detect).as_ref().map_err(Clone::clone)?;
    if s.other {
        log(&format!("elevated as another account: per-user settings go to {}", s.sid));
        unsafe { env::set_var(i18n::reg::SID_ENV, &s.sid) };
    }
    Ok(s)
}

/// Uninstall must work even when the user cannot be told (a broken session is when it is needed most): then the elevated
/// account's own HKCU is used and the failure becomes a warning.
pub(crate) fn init_soft() {
    if let Err(e) = init() {
        soft(t!("пользователь сеанса не определён, настройки пользователя убираются у текущей учётной записи", "signed-in user unknown, per-user settings are removed from the current account", ja: "サインイン中のユーザーが不明なため、ユーザーごとの設定は現在のアカウントから削除されます", pt: "usuário conectado desconhecido; as configurações por usuário são removidas da conta atual", zh: "登录用户未知，将从当前账户移除用户设置"), Err(e));
    }
}

/// PowerShell: `$u` = the account scheduled tasks are registered for.
pub(crate) fn task_user() -> String {
    match env::var(i18n::reg::SID_ENV).ok().filter(|s| i18n::reg::valid_sid(s)) {
        Some(sid) => format!("$u=(New-Object Security.Principal.SecurityIdentifier '{sid}').Translate([Security.Principal.NTAccount]).Value;"),
        None => "$u=[Security.Principal.WindowsIdentity]::GetCurrent().Name;".into(),
    }
}

/// PowerShell: `$p` = this shell's CurrentUserCurrentHost profile of the target user (Documents may be redirected: the
/// User Shell Folders value of that user's hive says where).
/// Logon task (Limited) for the signed-in user: unlike a HKCU Run value it cannot land in an MSIX package's private registry copy.
pub(crate) fn logon_task(name: &str, exe: &Path) -> bool {
    let task = format!("{} Register-ScheduledTask -TaskName {name} -Force -Action (New-ScheduledTaskAction -Execute '{}') -Trigger (New-ScheduledTaskTrigger -AtLogOn -User $u) -Principal (New-ScheduledTaskPrincipal -UserId $u -LogonType Interactive -RunLevel Limited) -Settings (New-ScheduledTaskSettingsSet -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries -ExecutionTimeLimit ([TimeSpan]::Zero)) | Out-Null", task_user(), exe.display());
    ps(&task).is_ok()
}

pub(crate) fn profile_path() -> String {
    match env::var(i18n::reg::SID_ENV).ok().filter(|s| i18n::reg::valid_sid(s)) {
        Some(sid) => format!(
            r"$h=(Get-ItemProperty 'Registry::HKEY_LOCAL_MACHINE\SOFTWARE\Microsoft\Windows NT\CurrentVersion\ProfileList\{sid}').ProfileImagePath; $d=(Get-ItemProperty 'Registry::HKEY_USERS\{sid}\Software\Microsoft\Windows\CurrentVersion\Explorer\User Shell Folders').Personal; $d=[Environment]::ExpandEnvironmentVariables($d.Replace('%USERPROFILE%',$h)); $p=Join-Path $d ($(if($PSVersionTable.PSEdition -eq 'Core'){{'PowerShell'}}else{{'WindowsPowerShell'}}) + '\Microsoft.PowerShell_profile.ps1');"
        ),
        None => "$p=$PROFILE;".into(),
    }
}
