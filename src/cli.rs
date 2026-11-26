//! Command line: list / install / update / uninstall / rollback (see README).
use crate::*;

// ---------- command line ----------
//
//   plugin-launcher list
//   plugin-launcher install <plugin>... --yes   (several names, or `all` for the defaults; without --yes it only prints warnings)
//   plugin-launcher uninstall <plugin>
//   plugin-launcher rollback --yes
//   plugin-launcher autoupdate               (what the WinRust-update task runs as SYSTEM: src/update)
// Exit code 0 = done, 1 = failed, 2 = bad arguments or --yes missing. Run it from an elevated terminal: the manifest
// demands admin, and from a normal one Windows opens a separate elevated console whose output is lost.

pub(crate) fn cli(args: &[String]) -> i32 {
    // A windows-subsystem exe has no console of its own: use stdout if the caller redirected it (pipe, file),
    // else attach to the parent's console and write to it directly.
    let redirected = unsafe { !GetStdHandle(STD_OUTPUT_HANDLE).is_null() && GetStdHandle(STD_OUTPUT_HANDLE) != INVALID_HANDLE_VALUE };
    let mut out: Box<dyn Write> = if redirected {
        Box::new(std::io::stdout())
    } else {
        unsafe { AttachConsole(ATTACH_PARENT_PROCESS) };
        winrust_common::utf8_console();
        fs::OpenOptions::new().write(true).open("CONOUT$").map_or_else(|_| Box::new(std::io::sink()) as Box<dyn Write>, |f| Box::new(f))
    };
    let mut say = |s: &str| {
        let _ = writeln!(out, "{s}");
    };
    let yes = args.iter().any(|a| a == "--yes");
    // `--lang zh` (or `--lang=zh`) also switches the interface language, so one elevated command installs and enables
    // it: the components restarted below read lang.txt and start in it.
    let lang_arg = args.iter().enumerate().find_map(|(i, a)| match a.strip_prefix("--lang=") {
        Some(v) => Some(v.to_string()),
        None if a == "--lang" => args.get(i + 1).cloned(),
        None => None,
    });
    let mut words: Vec<&str> = Vec::new();
    let mut skip = false;
    for a in args {
        if skip {
            skip = false;
        } else if a == "--yes" || a.starts_with("--lang=") {
            continue;
        } else if a == "--lang" {
            skip = true;
        } else {
            words.push(a);
        }
    }
    let find = |n: &str| PLUGINS.iter().position(|p| p.name == n);
    if let Some(code) = &lang_arg {
        let l = i18n::Lang::parse(code);
        i18n::set_lang(l);
        say(&tf!("Язык: {}", "Language: {}", ja: "言語: {}", pt: "Idioma: {}", zh: "语言：{}", l.name()));
    }
    let usage = t!("использование: plugin-launcher list | install <плагин>...|all --yes | update <плагин>|all | uninstall <плагин> | rollback --yes [--lang en|ru|ja|pt|zh]", "usage: plugin-launcher list | install <plugin>...|all --yes | update <plugin>|all | uninstall <plugin> | rollback --yes [--lang en|ru|ja|pt|zh]", ja: "使い方: plugin-launcher list | install <プラグイン>...|all --yes | update <プラグイン>|all | uninstall <プラグイン> | rollback --yes [--lang en|ru|ja|pt|zh]", pt: "uso: plugin-launcher list | install <plugin>...|all --yes | update <plugin>|all | uninstall <plugin> | rollback --yes [--lang en|ru|ja|pt|zh]", zh: "用法：plugin-launcher list | install <插件>...|all --yes | update <插件>|all | uninstall <插件> | rollback --yes [--lang en|ru|ja|pt|zh]");
    let result = match words.as_slice() {
        ["list"] => {
            for p in PLUGINS {
                say(&format!("{:<16}{}", p.name, if p.actions.is_none() { t!("заглушка", "stub", ja: "未実装", pt: "esboço", zh: "占位") } else if (p.installed)() { t!("установлен", "installed", ja: "インストール済み", pt: "instalado", zh: "已安装") } else { t!("не установлен", "not installed", ja: "未インストール", pt: "não instalado", zh: "未安装") }));
            }
            return 0;
        }
        ["install", names @ ..] if !names.is_empty() => {
            // Order of the table, so topbar comes before rust-explorer if both are requested.
            let mut idx = Vec::new();
            for n in names {
                match *n {
                    "all" => idx.extend(ready().filter(|&i| selected_by_default(PLUGINS[i].name, false))),
                    n => match find(n) {
                        Some(i) => idx.push(i),
                        None => return { say(&tf!("неизвестный плагин: {n}", "unknown plugin: {n}", ja: "不明なプラグイン: {n}", pt: "plugin desconhecido: {n}", zh: "未知插件：{n}")); 2 },
                    },
                }
            }
            idx.sort_unstable();
            idx.dedup();
            if !yes {
                for &i in &idx {
                    say(&format!("{}: {}", PLUGINS[i].name, PLUGINS[i].warn.get()));
                }
                return { say(t!("Ничего не изменено. Повторите с --yes, чтобы установить.", "Nothing changed. Repeat with --yes to install.", ja: "何も変更していません。--yes を付けてやり直してください。", pt: "Nada foi alterado. Repita com --yes para instalar.", zh: "没有任何更改。请加上 --yes 重新执行以安装。")); 2 };
            }
            // one that fails or is already there does not stop the rest
            let mut failed = false;
            for i in idx {
                if (PLUGINS[i].installed)() {
                    say(&tf!("{} уже установлен", "{} is already installed", ja: "{} はインストール済みです", pt: "{} já está instalado", zh: "{} 已经安装", PLUGINS[i].name));
                    continue;
                }
                match install_plugin(i) {
                    Ok(m) => say(&m),
                    Err(e) => {
                        failed = true;
                        say(&tf!("ошибка: {0}: {1}", "error: {0}: {1}", ja: "エラー: {0}: {1}", pt: "erro: {0}: {1}", zh: "错误：{0}：{1}", PLUGINS[i].name, e));
                    }
                }
            }
            return if failed { 1 } else { 0 };
        }
        // every installed plugin in table order (one UAC prompt for all); one that fails does not stop the rest
        ["update", "all"] => {
            let mut failed = false;
            for i in (0..PLUGINS.len()).filter(|&i| PLUGINS[i].actions.is_some() && (PLUGINS[i].installed)()) {
                match install_plugin(i) {
                    Ok(m) => say(&m),
                    Err(e) => {
                        failed = true;
                        say(&tf!("ошибка: {0}: {1}", "error: {0}: {1}", ja: "エラー: {0}: {1}", pt: "erro: {0}: {1}", zh: "错误：{0}：{1}", PLUGINS[i].name, e));
                    }
                }
            }
            return if failed { 1 } else { 0 };
        }
        ["update", n] => match find(n) {
            None => return { say(&tf!("неизвестный плагин: {n}", "unknown plugin: {n}", ja: "不明なプラグイン: {n}", pt: "plugin desconhecido: {n}", zh: "未知插件：{n}")); 2 },
            Some(i) if !(PLUGINS[i].installed)() => return { say(&tf!("{n} не установлен (используйте install)", "{n} is not installed (use install)", ja: "{n} はインストールされていません (install を使用してください)", pt: "{n} não está instalado (use install)", zh: "{n} 未安装（请使用 install）")); 2 },
            Some(i) => install_plugin(i),
        },
        ["uninstall", n] => match find(n) {
            None => return { say(&tf!("неизвестный плагин: {n}", "unknown plugin: {n}", ja: "不明なプラグイン: {n}", pt: "plugin desconhecido: {n}", zh: "未知插件：{n}")); 2 },
            Some(i) if !(PLUGINS[i].installed)() => return { say(&tf!("{n} не установлен", "{n} is not installed", ja: "{n} はインストールされていません", pt: "{n} não está instalado", zh: "{n} 未安装")); 0 },
            Some(i) => uninstall_plugin(i),
        },
        ["autoupdate"] => autoupdate(),
        ["rollback"] if yes => rollback(),
        ["rollback"] => return { say(t!("Удаляет все установленные плагины. Повторите с --yes.", "Removes every installed plugin. Repeat with --yes.", ja: "インストール済みのプラグインをすべて削除します。--yes を付けてやり直してください。", pt: "Remove todos os plugins instalados. Repita com --yes.", zh: "会移除所有已安装的插件。请加上 --yes 重新执行。")); 2 },
        _ => return { say(usage); 2 },
    };
    match result {
        Ok(m) => { say(&m); 0 }
        Err(e) => { say(&tf!("ошибка: {e}", "error: {e}", ja: "エラー: {e}", pt: "erro: {e}", zh: "错误：{e}")); 1 }
    }
}
