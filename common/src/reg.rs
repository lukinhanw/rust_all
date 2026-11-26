//! Reads the output of `reg query <key> /v <name>` (the installer and debloat both change and restore values through reg.exe).

/// The value line as (type, data), e.g. ("REG_DWORD", "0x1"). None = no value in the output.
/// reg.exe writes it as `    <name>    <type>    <data>`, four spaces apart; names and data may contain spaces themselves.
pub fn value(out: &str) -> Option<(&str, &str)> {
    out.lines().find_map(|l| {
        let mut f = l.strip_prefix("    ")?.splitn(3, "    ");
        let (_, ty) = (f.next()?, f.next()?);
        ty.starts_with("REG_").then(|| (ty, f.next().unwrap_or("").trim_end()))
    })
}

/// Native read, no reg.exe process: `path` like `HKLM\SOFTWARE\X` or `HKCU\...`; `name` None = the key itself.
/// true when the key (or the value in it) exists.
pub fn exists(path: &str, name: Option<&str>) -> bool {
    raw(path, name).is_some()
}

/// A REG_DWORD read natively; None when missing or of another type.
pub fn get_dword(path: &str, name: &str) -> Option<u32> {
    raw(path, Some(name)).and_then(|(ty, b)| (ty == 4 && b.len() >= 4).then(|| u32::from_le_bytes(b[..4].try_into().unwrap())))
}

fn raw(path: &str, name: Option<&str>) -> Option<(u32, Vec<u8>)> {
    use windows_sys::Win32::System::Registry::*;
    let (root, sub) = path.split_once('\\').unwrap_or((path, ""));
    let root = match root.to_ascii_uppercase().as_str() {
        "HKLM" | "HKEY_LOCAL_MACHINE" => HKEY_LOCAL_MACHINE,
        "HKCU" | "HKEY_CURRENT_USER" => HKEY_CURRENT_USER,
        "HKU" | "HKEY_USERS" => HKEY_USERS,
        "HKCR" | "HKEY_CLASSES_ROOT" => HKEY_CLASSES_ROOT,
        _ => return None,
    };
    let w = |s: &str| s.encode_utf16().chain([0]).collect::<Vec<u16>>();
    let sub = w(sub);
    unsafe {
        let mut k = std::ptr::null_mut();
        if RegOpenKeyExW(root, sub.as_ptr(), 0, KEY_READ, &mut k) != 0 {
            return None;
        }
        let r = match name {
            None => Some((0, vec![])),
            Some(n) => {
                let n = w(n);
                let (mut ty, mut len) = (0u32, 0u32);
                if RegQueryValueExW(k, n.as_ptr(), std::ptr::null(), &mut ty, std::ptr::null_mut(), &mut len) != 0 {
                    None
                } else {
                    let mut b = vec![0u8; len as usize];
                    let ok = RegQueryValueExW(k, n.as_ptr(), std::ptr::null(), &mut ty, b.as_mut_ptr(), &mut len) == 0;
                    b.truncate(len as usize);
                    ok.then_some((ty, b))
                }
            }
        };
        RegCloseKey(k);
        r
    }
}

/// The value as a number when it is a REG_DWORD; None when missing or of another type.
pub fn dword(out: &str) -> Option<u32> {
    match value(out)? {
        ("REG_DWORD", d) => u32::from_str_radix(d.trim_start_matches("0x"), 16).ok(),
        _ => None,
    }
}

/// Set by the installer for its children when the signed-in user is not the elevated account (UAC was answered with another
/// administrator): the SID of the signed-in user. Per-user keys then go to `HKU\<sid>`, not to the elevated account's HKCU.
pub const SID_ENV: &str = "WINRUST_SID";

pub fn valid_sid(s: &str) -> bool {
    s.starts_with("S-1-") && s.len() > 4 && s.chars().all(|c| c.is_ascii_digit() || c == '-' || c == 'S')
}

/// A reg.exe key argument aimed at the user `sid`: `HKCU\x` becomes `HKU\<sid>\x`. Anything else (HKLM, values, flags) is unchanged.
pub fn for_user(arg: &str, sid: &str) -> String {
    let rest = ["HKCU", "HKEY_CURRENT_USER"].iter().find_map(|p| arg.strip_prefix(p).filter(|r| r.is_empty() || r.starts_with('\\')));
    rest.map_or_else(|| arg.to_string(), |r| format!("HKU\\{sid}{r}"))
}

/// `for_user` for the SID the installer passed in the environment; the arguments as they are when it passed none.
pub fn user_args(args: &[&str]) -> Vec<String> {
    let sid = std::env::var(SID_ENV).ok().filter(|s| valid_sid(s));
    args.iter().map(|a| sid.as_deref().map_or_else(|| a.to_string(), |s| for_user(a, s))).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aims_hkcu_at_the_given_user() {
        assert_eq!(for_user(r"HKCU\Software\X", "S-1-5-21-1-2-3-1001"), r"HKU\S-1-5-21-1-2-3-1001\Software\X");
        assert_eq!(for_user("HKEY_CURRENT_USER", "S-1-5-21-9"), r"HKU\S-1-5-21-9");
        assert_eq!(for_user(r"HKLM\SOFTWARE\X", "S-1-5-21-9"), r"HKLM\SOFTWARE\X");
        assert_eq!(for_user(r"HKCUX\a", "S-1-5-21-9"), r"HKCUX\a");
        assert_eq!(for_user("/v", "S-1-5-21-9"), "/v");
        assert!(valid_sid("S-1-5-21-1-2-3-1001") && !valid_sid("S-1-5-x; calc") && !valid_sid(""));
    }

    #[test]
    fn reads_reg_query_output() {
        let key = "\r\nHKEY_LOCAL_MACHINE\\SOFTWARE\\REG_X\r\n";
        assert_eq!(dword(&format!("{key}    AllowTelemetry    REG_DWORD    0x1a\r\n\r\n")), Some(26));
        assert_eq!(value(&format!("{key}    Debugger    REG_SZ    \"C:\\Program Files\\x.exe\" /e\r\n")), Some(("REG_SZ", "\"C:\\Program Files\\x.exe\" /e")));
        assert_eq!(value("    (Default)    REG_SZ    \r\n"), Some(("REG_SZ", "")));
        assert_eq!(dword("    V    REG_SZ    0x1\r\n"), None);
        assert_eq!(value("ERROR: The system was unable to find the specified registry key or value."), None);
    }

    #[test]
    fn native_read() {
        assert!(exists(r"HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion", Some("ProductName")));
        assert!(exists(r"HKLM\SOFTWARE\Microsoft", None));
        assert!(!exists(r"HKLM\SOFTWARE\no-such-key-winrust", None));
        assert!(!exists(r"BAD\x", None));
        assert!(get_dword(r"HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion", "CurrentMajorVersionNumber").is_some_and(|v| v >= 10));
        assert_eq!(get_dword(r"HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion", "ProductName"), None, "REG_SZ is not a DWORD");
    }
}
