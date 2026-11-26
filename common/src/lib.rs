//! Code shared by the WinRust programs: the language switch (`t!`, `tf!`, `lang!`, `en!`), the NTFS MFT reader, Win+D.
//! Use it with `#[macro_use] extern crate winrust_common;` at the crate root.
//!
//! English by default; Russian, Japanese or Portuguese (Brazil) when %ProgramData%\WinRust\lang.txt contains "ru", "ja" or "pt"
//! (the installer's language button writes it).
//! Programs read the file once at start; the one that flips the switch updates its own texts at once.
use std::{
    path::PathBuf,
    sync::atomic::{AtomicU8, Ordering::Relaxed},
};

pub mod desktop;
pub mod dpi;
pub mod fg;
pub mod journal;
pub mod mem;
pub mod mft;
pub mod proc;
pub mod reg;
pub mod settings;
pub mod style;
pub mod wallpaper;

/// "Show in folder": opens `path`'s folder with the file selected in the installed rust-explorer, else in Windows Explorer.
pub fn reveal(path: &str) {
    use std::{os::windows::process::CommandExt, process::Command};
    let rx = std::env::var_os("ProgramFiles").map(|pf| PathBuf::from(pf).join(r"rust-explorer\rust-explorer.exe")).filter(|p| p.is_file());
    let _ = match rx {
        Some(rx) => Command::new(rx).raw_arg(format!("/select,\"{path}\"")).spawn(),
        None => Command::new("explorer.exe").raw_arg(format!("/select,\"{path}\"")).spawn(),
    };
}

/// The UI languages. English by default; the choice lives in %ProgramData%\WinRust\lang.txt.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Lang {
    En,
    Ru,
    Ja,
    Pt,
}

impl Lang {
    /// The name of the language in itself (for the switch button).
    pub fn name(self) -> &'static str {
        match self {
            Lang::En => "English",
            Lang::Ru => "Русский",
            Lang::Ja => "日本語",
            Lang::Pt => "Português",
        }
    }
    /// The language after this one: the switch button goes round them.
    pub fn next(self) -> Lang {
        match self {
            Lang::En => Lang::Ru,
            Lang::Ru => Lang::Ja,
            Lang::Ja => Lang::Pt,
            Lang::Pt => Lang::En,
        }
    }
    fn code(self) -> &'static str {
        ["en", "ru", "ja", "pt"][self as usize]
    }
    fn parse(s: &str) -> Lang {
        let s = s.trim().to_ascii_lowercase();
        match s.get(..2) {
            Some("ru") => Lang::Ru,
            Some("ja") => Lang::Ja,
            Some("pt") => Lang::Pt,
            _ => Lang::En,
        }
    }
}

static LANG: AtomicU8 = AtomicU8::new(u8::MAX); // MAX = not read yet, else a `Lang`

fn file() -> Option<PathBuf> {
    std::env::var_os("ProgramData").map(|p| PathBuf::from(p).join("WinRust").join("lang.txt"))
}

/// The current language at run time. In code that has unit tests use `lang!()` instead: it is always Russian under `cargo test`.
pub fn lang() -> Lang {
    match LANG.load(Relaxed) {
        0 => Lang::En,
        1 => Lang::Ru,
        2 => Lang::Ja,
        3 => Lang::Pt,
        _ => {
            let l = file().and_then(|f| std::fs::read_to_string(f).ok()).map_or(Lang::En, |s| Lang::parse(&s));
            LANG.store(l as u8, Relaxed);
            l
        }
    }
}

/// Switches this process and writes the choice for the other programs (they pick it up when they start).
pub fn set_lang(l: Lang) {
    LANG.store(l as u8, Relaxed);
    if let Some(f) = file() {
        let _ = std::fs::create_dir_all(f.parent().unwrap());
        let _ = std::fs::write(f, l.code());
    }
}

/// `lang!()`: the current `Lang`. `cfg!(test)` is checked in the calling crate, so its unit tests keep their Russian expectations.
#[macro_export]
macro_rules! lang {
    () => {
        if cfg!(test) { $crate::Lang::Ru } else { $crate::lang() }
    };
}

/// `en!()`: true for English only.
#[macro_export]
macro_rules! en {
    () => {
        $crate::lang!() == $crate::Lang::En
    };
}

/// `dot!()`: true where the decimal separator is a dot (English, Japanese); Russian and Portuguese use a comma.
#[macro_export]
macro_rules! dot {
    () => {
        matches!($crate::lang!(), $crate::Lang::En | $crate::Lang::Ja)
    };
}

/// `t!("русский", "english", ja: "日本語", pt: "português")`: a text in the current language.
/// Without `ja:`/`pt:` Japanese and Portuguese show the English text; `t!(ru, en, pt: ..)` has Portuguese only.
#[macro_export]
macro_rules! t {
    ($ru:expr, $en:expr, ja: $ja:expr, pt: $pt:expr $(,)?) => {
        match $crate::lang!() {
            $crate::Lang::Ru => $ru,
            $crate::Lang::En => $en,
            $crate::Lang::Ja => $ja,
            $crate::Lang::Pt => $pt,
        }
    };
    ($ru:expr, $en:expr, pt: $pt:expr $(,)?) => {
        match $crate::lang!() {
            $crate::Lang::Ru => $ru,
            $crate::Lang::Pt => $pt,
            _ => $en,
        }
    };
    ($ru:expr, $en:expr) => {
        if $crate::lang!() == $crate::Lang::Ru { $ru } else { $en }
    };
}

/// `tf!("русский {}", "english {}", ja: "日本語 {}", pt: "português {}", args...)`: format! in the current language
/// (all templates are checked at compile time). Without `ja:`/`pt:` they use the English one; `pt:` alone is allowed too.
#[macro_export]
macro_rules! tf {
    ($ru:literal, $en:literal, ja: $ja:literal, pt: $pt:literal $(, $a:expr)* $(,)?) => {
        match $crate::lang!() {
            $crate::Lang::Ru => format!($ru $(, $a)*),
            $crate::Lang::En => format!($en $(, $a)*),
            $crate::Lang::Ja => format!($ja $(, $a)*),
            $crate::Lang::Pt => format!($pt $(, $a)*),
        }
    };
    ($ru:literal, $en:literal, pt: $pt:literal $(, $a:expr)* $(,)?) => {
        match $crate::lang!() {
            $crate::Lang::Ru => format!($ru $(, $a)*),
            $crate::Lang::Pt => format!($pt $(, $a)*),
            _ => format!($en $(, $a)*),
        }
    };
    ($ru:literal, $en:literal $(, $a:expr)* $(,)?) => {
        if $crate::lang!() == $crate::Lang::Ru { format!($ru $(, $a)*) } else { format!($en $(, $a)*) }
    };
}

#[cfg(test)]
mod tests {
    #[test]
    fn tests_run_in_russian() {
        assert_eq!(t!("да", "yes"), "да");
        assert_eq!(tf!("{} шт", "{} pcs", 3), "3 шт");
        assert_eq!(t!("да", "yes", ja: "はい", pt: "sim"), "да");
        assert_eq!(tf!("{} шт", "{} pcs", ja: "{} 個", pt: "{} un.", 3), "3 шт");
    }

    #[test]
    fn lang_codes() {
        use super::Lang;
        assert_eq!(Lang::parse(" pt-BR "), Lang::Pt);
        assert_eq!(Lang::parse("JA"), Lang::Ja);
        assert_eq!(Lang::parse("ru"), Lang::Ru);
        assert_eq!(Lang::parse("x"), Lang::En);
        assert_eq!(Lang::Pt.next(), Lang::En);
    }
}
