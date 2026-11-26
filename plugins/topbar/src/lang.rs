//! topbar: lang (split out of main.rs; shares its items through `super::*`).
use super::*;

/// Next keyboard layout for the app window. Posted to its focused control (that is where Chrome, Office... look for it);
/// timer 7 checks it took and otherwise presses Win+Space, the system switcher (games, terminals ignore the message).
/// The installed keyboard layouts and the one the app in front uses.
pub(crate) unsafe fn layouts() -> (Vec<HKL>, HKL) {
    let mut l: [HKL; 32] = [null_mut(); 32];
    let n = GetKeyboardLayoutList(32, l.as_mut_ptr()).clamp(0, 32) as usize;
    (l[..n].to_vec(), GetKeyboardLayout(GetWindowThreadProcessId(app_window(), null_mut())))
}

/// A layout's language as Windows names it in the user's language ("Русский (Россия)"), and its short code ("RUS").
pub(crate) unsafe fn layout_names(k: HKL) -> (String, String) {
    let id = k as usize as u32 & 0xffff;
    let get = |what: u32| {
        let mut b = [0u16; 96];
        let n = GetLocaleInfoW(id, what, b.as_mut_ptr(), 96).max(1) as usize - 1;
        String::from_utf16_lossy(&b[..n])
    };
    let short = get(3).to_uppercase(); // LOCALE_SABBREVLANGNAME
    (get(2), if short == "ENU" { "ENG".into() } else { short }) // LOCALE_SLOCALIZEDDISPLAYNAME
}

/// The input language picker above the language button.
pub(crate) unsafe fn lang_menu(bar: HWND) {
    let (list, cur) = layouts();
    let mut items: Vec<fly::MItem> = list.iter().map(|&k| {
        let (name, short) = layout_names(k);
        fly::MItem::new(name).on(k == cur).hint(Box::leak(short.into_boxed_str())) // a handful of short strings, kept for the bar's lifetime
    }).collect();
    items.push(fly::MItem::new(t!("Языковые параметры", "Language settings", ja: "言語設定", pt: "Configurações de idioma", zh: "语言设置")).sep());
    let head = (null_mut(), "\u{E775}", t!("Язык ввода", "Input language", ja: "入力言語", pt: "Idioma de entrada", zh: "输入语言").to_string(), t!("Win+Пробел — следующий", "Win+Space — next one", ja: "Win+Space — 次の言語", pt: "Win+Espaço — o próximo", zh: "Win+Space — 下一个").to_string());
    fly::menu(tip_cx(&bar_layout(bar), LANG), Some(head), items, move |i| match list.get(i) {
        Some(&k) => set_lang(bar, k),
        None => open(&wide("ms-settings:regionlanguage")),
    });
}

/// The window in front is one of the bar's own (Start, a flyout).
pub(crate) unsafe fn own_foreground() -> bool {
    let mut pid = 0u32;
    GetWindowThreadProcessId(GetForegroundWindow(), &mut pid);
    pid == std::process::id()
}

/// The next layout (or the previous one with `back`, Win+Shift+Space) for the app in front.
pub(crate) unsafe fn next_lang(bar: HWND, back: bool) {
    let own = own_foreground(); // our Start in front: switch this thread (app_window() is the app behind it)
    let (l, cur) = if own { (layouts().0, GetKeyboardLayout(0)) } else { layouts() };
    if l.len() < 2 {
        return; // one layout: nothing to switch to
    }
    let i = l.iter().position(|&k| k == cur);
    let n = l.len();
    let next = l[match (i, back) { (Some(i), false) => (i + 1) % n, (Some(i), true) => (i + n - 1) % n, (None, _) => 0 }];
    if own {
        ActivateKeyboardLayout(next, 0);
        InvalidateRect(bar, null(), 0);
    } else {
        set_lang(bar, next);
    }
}

/// Asks the app in front to switch to layout `next` (the bar's own thread cannot change another app's language).
pub(crate) unsafe fn set_lang(bar: HWND, next: HKL) {
    let w = app_window();
    let tid = GetWindowThreadProcessId(w, null_mut());
    let cur = GetKeyboardLayout(tid);
    let mut gi: GUITHREADINFO = zeroed();
    gi.cbSize = size_of::<GUITHREADINFO>() as u32;
    let target = if GetGUIThreadInfo(tid, &mut gi) != 0 && !gi.hwndFocus.is_null() { gi.hwndFocus } else { w };
    PostMessageW(target, WM_INPUTLANGCHANGEREQUEST, 0, next as isize);
    LANG_TRY.set((tid, cur as usize, next as usize));
    SetTimer(bar, 7, 250, None);
}

pub(crate) unsafe fn lang_name() -> String {
    let tid = GetWindowThreadProcessId(app_window(), null_mut());
    let id = GetKeyboardLayout(tid) as usize as u32 & 0xffff;
    let mut b = [0u16; 16];
    let n = GetLocaleInfoW(id, 3, b.as_mut_ptr(), 16).max(1) as usize - 1; // LOCALE_SABBREVLANGNAME
    let s = String::from_utf16_lossy(&b[..n]).to_uppercase();
    if s == "ENU" { "ENG".into() } else { s }
}

thread_local! {
    pub(crate) static SIG: Cell<(u64, u32)> = const { Cell::new((0, 0)) }; // (hash of the last painted state, ticks since the last repaint)
}
