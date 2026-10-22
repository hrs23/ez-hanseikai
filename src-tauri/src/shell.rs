use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value};
use tauri::menu::{Menu, MenuBuilder, MenuItem, MenuItemBuilder, SubmenuBuilder};
use tauri::{AppHandle, Emitter, Manager, Wry};
use tauri_plugin_dialog::{DialogExt, FileDialogBuilder, MessageDialogButtons, MessageDialogKind};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Modifiers, Shortcut};
use tauri_plugin_notification::NotificationExt;

use crate::{language, Core, REPO, VERSION};

type Action = fn(&Arc<Core>) -> Option<&'static str>;
pub const HOTKEYS: [(&str, &str, Action); 3] = [("mark", "Ctrl+Alt+M", mark), ("mic", "Ctrl+Alt+N", toggle_mic), ("screen", "Ctrl+Alt+V", toggle_screen)];
pub type Hotkeys = [String; 3];
const BOARD: &str = "board";
const STATUS_POLL: Duration = Duration::from_secs(1);
const JA: [(&str, &str); 44] = [
    ("Already running with this data folder.", "このデータフォルダですでに動いています。"),
    ("EZ Hanseikai", "EZ反省会"),
    ("File", "ファイル"),
    ("Room", "ルーム"),
    ("Recording", "録画"),
    ("View", "表示"),
    ("Help", "ヘルプ"),
    ("Open data folder", "データフォルダを開く"),
    ("Settings…", "設定…"),
    ("Exit", "終了"),
    ("Copy invite code", "招待コードをコピー"),
    ("Cancel invite", "招待を取り消す"),
    ("Room settings…", "ルームの設定…"),
    ("Leave room", "ルームを離れる"),
    ("Members", "メンバー"),
    ("Extensions", "拡張機能"),
    ("Start recording", "録画開始"),
    ("Stop recording", "録画終了"),
    ("Record debrief", "反省会を録画"),
    ("Mark this moment", "この瞬間をマーク"),
    ("Add note", "メモを追加"),
    ("Export to folder…", "フォルダにエクスポート…"),
    ("Delete…", "削除…"),
    ("Live", "ライブ"),
    ("Game plan", "作戦ボード"),
    ("Sidebar", "サイドバー"),
    ("Full screen", "全画面"),
    ("Keyboard shortcuts", "キーボードショートカット"),
    ("Diagnostics", "診断"),
    ("Check for updates", "アップデートを確認"),
    ("Reset app…", "アプリをリセット…"),
    ("Report a problem", "問題を報告"),
    ("Release notes", "リリースノート"),
    ("About", "バージョン情報"),
    ("Cancel the export and exit?", "エクスポートを中止してアプリを終了しますか?"),
    ("Files are still arriving. Exit anyway?", "まだファイルを受け取っています。それでも終了しますか?"),
    ("Cancel", "キャンセル"),
    ("Version", "バージョン"),
    ("Moment marked", "マークしました"),
    ("Microphone on", "マイクオン"),
    ("Microphone off", "マイクオフ"),
    ("Screen on", "画面オン"),
    ("Screen off", "画面オフ"),
    ("{name} wants to join", "{name} が参加を希望しています"),
];

struct Items {
    toggle: Option<MenuItem<Wry>>,
    mark: Option<MenuItem<Wry>>,
}

static ITEMS: Mutex<Items> = Mutex::new(Items { toggle: None, mark: None });

pub fn tr<'a>(lang: &str, en: &'a str) -> &'a str {
    if lang != "ja" {
        return en;
    }
    JA.iter().find(|(e, _)| *e == en).map_or(en, |(_, ja)| ja)
}

pub fn app_name(app: &AppHandle, lang: &str) -> String {
    tr(lang, &app.package_info().name).to_string()
}

pub fn ask(app: &AppHandle, lang: &str, text: &str, ok: &str, cancel: &str) -> bool {
    app.dialog().message(text).title(app_name(app, lang)).kind(MessageDialogKind::Warning).buttons(MessageDialogButtons::OkCancelCustom(ok.into(), cancel.into())).blocking_show()
}

pub fn tell(app: &AppHandle, lang: &str, text: &str) {
    app.dialog().message(text).title(app_name(app, lang)).kind(MessageDialogKind::Info).buttons(MessageDialogButtons::Ok).blocking_show();
}

fn file_dialog(app: &AppHandle) -> FileDialogBuilder<Wry> {
    let dialog = app.dialog().file();
    match app.path().document_dir() {
        Ok(dir) => dialog.set_directory(dir),
        Err(_) => dialog,
    }
}

pub fn save_file(app: &AppHandle, name: &str) -> Option<PathBuf> {
    let ext = Path::new(name).extension().and_then(|e| e.to_str()).unwrap_or_default();
    file_dialog(app).set_file_name(name).add_filter(ext.to_uppercase(), &[ext]).blocking_save_file().and_then(|p| p.into_path().ok())
}

pub fn pick_folder(app: &AppHandle) -> Option<PathBuf> {
    file_dialog(app).blocking_pick_folder().and_then(|p| p.into_path().ok())
}

pub fn notify(app: &AppHandle, title: &str, body: &str) {
    let _ = app.notification().builder().title(title).body(body).show();
}

const LINK_MAX: usize = 2048;

pub fn link_ok(url: &str) -> bool {
    (url.starts_with("https://") || url.starts_with("http://")) && url.len() <= LINK_MAX && url.chars().all(|c| c.is_ascii_graphic() && c != '"')
}

pub fn open_link(url: &str) {
    use windows::core::{w, HSTRING, PCWSTR};
    use windows::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE};
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    if link_ok(url) {
        let url = HSTRING::from(url);
        std::thread::spawn(move || unsafe {
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE);
            ShellExecuteW(None, w!("open"), &url, PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL);
        });
    }
}

pub fn already_running(app: &AppHandle) {
    use windows::core::HSTRING;
    use windows::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONINFORMATION, MB_OK};
    let text = tr(crate::os_language(), "Already running with this data folder.");
    unsafe { MessageBoxW(None, &HSTRING::from(text), &HSTRING::from(app_name(app, crate::os_language())), MB_OK | MB_ICONINFORMATION) };
}

pub fn set_fullscreen(app: &AppHandle, label: &str, on: bool) -> Result<(), String> {
    let window = app.get_webview_window(label).ok_or("No such window.")?;
    window.set_fullscreen(on).map_err(|e| e.to_string())?;
    (if on { window.hide_menu() } else { window.show_menu() }).map_err(|e| e.to_string())
}

pub fn open_board(app: &AppHandle, dir: &Path) {
    if let Some(window) = app.get_webview_window(BOARD) {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
        return;
    }
    let (handle, dir) = (app.clone(), dir.to_path_buf());
    let _ = app.run_on_main_thread(move || {
        let builder = tauri::WebviewWindowBuilder::new(&handle, BOARD, tauri::WebviewUrl::App("index.html?board".into())).title(app_name(&handle, "")).decorations(false).maximized(true).theme(Some(tauri::Theme::Dark)).background_color(tauri::window::Color(12, 14, 18, 255));
        match crate::profile(builder, &dir).build() {
            Ok(window) => crate::shell::plain_keys(&window),
            Err(e) => tracing::warn!("the board window did not open: {e}"),
        }
    });
}

pub fn show_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
        let _ = app.emit("shown", ());
    }
}

fn window_shown(app: &AppHandle) -> bool {
    app.get_webview_window("main").is_some_and(|w| w.is_visible().unwrap_or(false) && !w.is_minimized().unwrap_or(false))
}

fn window_watched(app: &AppHandle) -> bool {
    window_shown(app) && app.get_webview_window("main").is_some_and(|w| w.is_focused().unwrap_or(false))
}

const CAPTION: u32 = 0x0f1115;
const CAPTION_TEXT: u32 = 0xe7e9ee;

fn colorref(rgb: u32) -> u32 {
    ((rgb & 0xff) << 16) | (rgb & 0xff00) | (rgb >> 16)
}

const MENU_HOT: u32 = 0x262b35;
const MENU_DIM: u32 = 0x8a92a3;
const MENU_SUBCLASS: usize = 0x4747;

fn fill(hdc: ::windows::Win32::Graphics::Gdi::HDC, rect: &::windows::Win32::Foundation::RECT, rgb: u32) {
    use ::windows::Win32::Graphics::Gdi::{CreateSolidBrush, DeleteObject, FillRect};
    unsafe {
        let brush = CreateSolidBrush(::windows::Win32::Foundation::COLORREF(colorref(rgb)));
        FillRect(hdc, rect, brush);
        let _ = DeleteObject(brush.into());
    }
}

#[repr(C)]
struct MenuDraw {
    hmenu: ::windows::Win32::UI::WindowsAndMessaging::HMENU,
    hdc: ::windows::Win32::Graphics::Gdi::HDC,
    flags: u32,
}

#[repr(C)]
struct MenuItemDraw {
    item: ::windows::Win32::UI::Controls::DRAWITEMSTRUCT,
    menu: MenuDraw,
    position: u32,
}

unsafe extern "system" fn menu_bar_proc(hwnd: ::windows::Win32::Foundation::HWND, msg: u32, wparam: ::windows::Win32::Foundation::WPARAM, lparam: ::windows::Win32::Foundation::LPARAM, _id: usize, _data: usize) -> ::windows::Win32::Foundation::LRESULT {
    use ::windows::Win32::Foundation::{LRESULT, RECT};
    use ::windows::Win32::Graphics::Gdi::{DrawTextW, GetWindowDC, ReleaseDC, SetBkMode, SetTextColor, DT_CENTER, DT_HIDEPREFIX, DT_SINGLELINE, DT_VCENTER, TRANSPARENT};
    use ::windows::Win32::UI::Shell::DefSubclassProc;
    use ::windows::Win32::UI::WindowsAndMessaging::{GetMenuBarInfo, GetMenuItemInfoW, GetWindowRect, MENUBARINFO, MENUITEMINFOW, MIIM_STRING, OBJID_MENU, WM_NCACTIVATE, WM_NCPAINT};
    const DRAW_MENU: u32 = 0x91;
    const DRAW_ITEM: u32 = 0x92;
    const ODS_SELECTED: u32 = 1;
    const ODS_GRAYED: u32 = 2;
    const ODS_DISABLED: u32 = 4;
    const ODS_NOACCEL: u32 = 0x100;
    const ODS_HOTLIGHT: u32 = 0x40;
    let bar = || unsafe {
        let mut info = MENUBARINFO { cbSize: std::mem::size_of::<MENUBARINFO>() as u32, ..Default::default() };
        let _ = GetMenuBarInfo(hwnd, OBJID_MENU, 0, &mut info);
        let mut window = RECT::default();
        let _ = GetWindowRect(hwnd, &mut window);
        RECT { left: info.rcBar.left - window.left, right: info.rcBar.right - window.left, top: info.rcBar.top - window.top - 1, bottom: info.rcBar.bottom - window.top }
    };
    match msg {
        DRAW_MENU => {
            let hdc = unsafe { (*(lparam.0 as *const MenuDraw)).hdc };
            fill(hdc, &bar(), CAPTION);
            LRESULT(0)
        }
        DRAW_ITEM => unsafe {
            let draw = &mut *(lparam.0 as *mut MenuItemDraw);
            let hdc = draw.menu.hdc;
            let mut info = MENUITEMINFOW { cbSize: std::mem::size_of::<MENUITEMINFOW>() as u32, fMask: MIIM_STRING, ..Default::default() };
            if GetMenuItemInfoW(draw.menu.hmenu, draw.position, true, &mut info).is_err() {
                return LRESULT(0);
            }
            let mut label = vec![0u16; info.cch as usize + 1];
            info.dwTypeData = ::windows::core::PWSTR(label.as_mut_ptr());
            info.cch += 1;
            if GetMenuItemInfoW(draw.menu.hmenu, draw.position, true, &mut info).is_err() {
                return LRESULT(0);
            }
            let lit = draw.item.itemState.0 & (ODS_HOTLIGHT | ODS_SELECTED) != 0;
            let dim = draw.item.itemState.0 & (ODS_GRAYED | ODS_DISABLED) != 0;
            fill(hdc, &draw.item.rcItem, if lit { MENU_HOT } else { CAPTION });
            SetBkMode(hdc, TRANSPARENT);
            SetTextColor(hdc, ::windows::Win32::Foundation::COLORREF(colorref(if dim { MENU_DIM } else { CAPTION_TEXT })));
            let mut flags = DT_CENTER | DT_SINGLELINE | DT_VCENTER;
            if draw.item.itemState.0 & ODS_NOACCEL != 0 {
                flags |= DT_HIDEPREFIX;
            }
            DrawTextW(hdc, &mut label[..info.cch as usize], &mut draw.item.rcItem, flags);
            LRESULT(0)
        },
        WM_NCPAINT | WM_NCACTIVATE => unsafe {
            let result = DefSubclassProc(hwnd, msg, wparam, lparam);
            let mut line = bar();
            line.top = line.bottom;
            line.bottom += 1;
            let hdc = GetWindowDC(Some(hwnd));
            fill(hdc, &line, CAPTION);
            ReleaseDC(Some(hwnd), hdc);
            result
        },
        _ => unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) },
    }
}

fn dark_popups() {
    use ::windows::core::w;
    use ::windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
    const SET_PREFERRED_APP_MODE: usize = 135;
    const FLUSH_MENU_THEMES: usize = 136;
    const FORCE_DARK: i32 = 2;
    unsafe {
        let Ok(uxtheme) = LoadLibraryW(w!("uxtheme.dll")) else { return };
        let ordinal = |n: usize| GetProcAddress(uxtheme, ::windows::core::PCSTR(n as *const u8));
        if let (Some(set), Some(flush)) = (ordinal(SET_PREFERRED_APP_MODE), ordinal(FLUSH_MENU_THEMES)) {
            let set: unsafe extern "system" fn(i32) -> i32 = std::mem::transmute(set);
            let flush: unsafe extern "system" fn() = std::mem::transmute(flush);
            set(FORCE_DARK);
            flush();
        }
    }
}

pub fn dark_menu(window: &tauri::WebviewWindow) {
    use ::windows::Win32::UI::Shell::{RemoveWindowSubclass, SetWindowSubclass};
    let Ok(hwnd) = window.hwnd() else { return };
    let hwnd = ::windows::Win32::Foundation::HWND(hwnd.0);
    dark_popups();
    let _ = unsafe { RemoveWindowSubclass(hwnd, Some(menu_bar_proc), MENU_SUBCLASS) };
    let _ = unsafe { SetWindowSubclass(hwnd, Some(menu_bar_proc), MENU_SUBCLASS, 0) };
}

pub fn match_caption(window: &tauri::WebviewWindow) {
    use ::windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_BORDER_COLOR, DWMWA_CAPTION_COLOR, DWMWA_TEXT_COLOR};
    let Ok(hwnd) = window.hwnd() else { return };
    let hwnd = ::windows::Win32::Foundation::HWND(hwnd.0);
    for (attribute, rgb) in [(DWMWA_CAPTION_COLOR, CAPTION), (DWMWA_BORDER_COLOR, CAPTION), (DWMWA_TEXT_COLOR, CAPTION_TEXT)] {
        let color = colorref(rgb);
        let _ = unsafe { DwmSetWindowAttribute(hwnd, attribute, &color as *const u32 as *const _, std::mem::size_of::<u32>() as u32) };
    }
}

pub fn plain_keys(window: &tauri::WebviewWindow) {
    let _ = window.with_webview(|webview| {
        use webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2Settings3;
        use windows::core::Interface;
        let settings = unsafe { webview.controller().CoreWebView2().and_then(|core| core.Settings()) };
        if let Ok(settings) = settings.and_then(|s| s.cast::<ICoreWebView2Settings3>()) {
            let _ = unsafe { settings.SetAreBrowserAcceleratorKeysEnabled(false) };
        }
    });
}

pub fn exit_question(exporting: bool, receiving: bool) -> Option<&'static str> {
    if exporting {
        Some("Cancel the export and exit?")
    } else {
        receiving.then_some("Files are still arriving. Exit anyway?")
    }
}

pub fn exit_app(app: &AppHandle, core: &Arc<Core>) {
    let exporting = core.export.lock().unwrap().is_some();
    if let Some(question) = exit_question(exporting, crate::net::receiving(core)) {
        show_window(app);
        let lang = language(core);
        if !ask(app, &lang, tr(&lang, question), tr(&lang, "Exit"), tr(&lang, "Cancel")) {
            return;
        }
    }
    crate::data::release(core);
    app.exit(0);
}

pub fn hotkey(text: &str) -> Option<Shortcut> {
    let shortcut = Shortcut::from_str(text).ok()?;
    shortcut.mods.intersects(Modifiers::CONTROL | Modifiers::ALT | Modifiers::SUPER).then_some(shortcut)
}

fn check_hotkey(keys: &Hotkeys, index: usize, text: &str) -> Result<Shortcut, &'static str> {
    let shortcut = hotkey(text).ok_or("Use Ctrl or Alt with a key.")?;
    if keys.iter().enumerate().any(|(i, k)| i != index && hotkey(k) == Some(shortcut)) {
        return Err("Another action uses that shortcut.");
    }
    Ok(shortcut)
}

pub fn load_hotkeys(settings: &Value) -> Hotkeys {
    let mut keys = Hotkeys::default();
    for (i, (id, default, _)) in HOTKEYS.iter().enumerate() {
        keys[i] = [settings["hotkeys"][id].as_str(), Some(*default)].into_iter().flatten().find(|k| check_hotkey(&keys, i, k).is_ok()).unwrap_or_default().to_string();
    }
    keys
}

pub fn hotkeys_json(keys: &Hotkeys) -> Value {
    json!(HOTKEYS.iter().zip(keys).map(|((id, default, _), keys)| json!({"action": id, "keys": keys, "default": default})).collect::<Vec<_>>())
}

fn arm_hotkeys<S: AsRef<str>>(app: &AppHandle, keys: &[S], on: bool) -> bool {
    let shortcuts = app.global_shortcut();
    let _ = shortcuts.unregister_all();
    !on || keys.iter().filter_map(|k| hotkey(k.as_ref())).fold(true, |ok, s| shortcuts.register(s).is_ok() && ok)
}

pub fn set_hotkey(core: &Core, action: &str, text: &str) -> Result<(), String> {
    let index = HOTKEYS.iter().position(|(id, _, _)| *id == action).ok_or("Unknown shortcut.")?;
    let mut keys = core.hotkeys.lock().unwrap().clone();
    check_hotkey(&keys, index, text)?;
    let free = arm_hotkeys(&core.app, &[text], true);
    if free {
        keys[index] = text.to_string();
    }
    arm_hotkeys(&core.app, &keys, core.any_recording());
    if !free {
        return Err("Another app uses that shortcut.".into());
    }
    *core.hotkeys.lock().unwrap() = keys;
    core.save_settings();
    let _ = install_menus(&core.app, core);
    Ok(())
}

fn mark(core: &Arc<Core>) -> Option<&'static str> {
    crate::ops::mark(core, Value::Null).ok().map(|_| "Moment marked")
}

fn toggle_mic(core: &Arc<Core>) -> Option<&'static str> {
    let mic = core.engine.mix.lock().unwrap()["mic"].clone();
    let muted = mic["muted"] != true;
    crate::ops::find("set_mix")?(core, json!({"kind": "mic", "volume": mic["volume"], "muted": muted})).ok()?;
    Some(if muted { "Microphone off" } else { "Microphone on" })
}

fn toggle_screen(core: &Arc<Core>) -> Option<&'static str> {
    let on = !core.engine.source.lock().unwrap().is_null();
    let next = if on { Value::Null } else { core.engine.last_source.lock().unwrap().clone() };
    if !on && next.is_null() {
        return None;
    }
    crate::ops::find("set_source")?(core, json!({"source": next})).ok()?;
    Some(if on { "Screen off" } else { "Screen on" })
}

pub fn run_hotkey(core: &Arc<Core>, index: usize) {
    if let Some(done) = HOTKEYS[index].2(core) {
        if !window_watched(&core.app) {
            notify(&core.app, tr(&language(core), done), "");
        }
    }
}

pub fn pressed(core: &Arc<Core>, shortcut: &Shortcut) {
    let index = core.hotkeys.lock().unwrap().iter().position(|k| hotkey(k).as_ref() == Some(shortcut));
    if let Some(index) = index {
        run_hotkey(core, index);
    }
}

fn item(app: &AppHandle, lang: &str, id: &str, text: &'static str, accel: Option<&str>) -> tauri::Result<MenuItem<Wry>> {
    let builder = || MenuItemBuilder::with_id(id, tr(lang, text));
    match accel {
        Some(a) => builder().accelerator(a).build(app).or_else(|_| builder().build(app)),
        None => builder().build(app),
    }
}

fn toggle_text(recording: bool, reviewing: bool) -> &'static str {
    match (recording, reviewing) {
        (true, _) => "Stop recording",
        (false, true) => "Record debrief",
        (false, false) => "Start recording",
    }
}

fn build_menu(app: &AppHandle, lang: &str, toggle: &MenuItem<Wry>, mark: &MenuItem<Wry>) -> tauri::Result<Menu<Wry>> {
    let it = |id: &str, text: &'static str, accel: Option<&str>| item(app, lang, id, text, accel);
    let file = SubmenuBuilder::new(app, tr(lang, "File"))
        .item(&it("open-folder", "Open data folder", None)?)
        .separator()
        .item(&it("settings", "Settings…", Some("CmdOrCtrl+,"))?)
        .separator()
        .item(&it("exit", "Exit", None)?)
        .build()?;
    let room = SubmenuBuilder::new(app, tr(lang, "Room"))
        .item(&it("rename-room", "Room settings…", None)?)
        .item(&it("copy-invite", "Copy invite code", None)?)
        .item(&it("cancel-invite", "Cancel invite", None)?)
        .separator()
        .item(&it("leave-room", "Leave room", None)?)
        .build()?;
    let session = SubmenuBuilder::new(app, tr(lang, "Recording"))
        .item(toggle)
        .item(mark)
        .item(&it("add-note", "Add note", None)?)
        .separator()
        .item(&it("export-folder", "Export to folder…", None)?)
        .item(&it("delete-session", "Delete…", None)?)
        .build()?;
    let view = SubmenuBuilder::new(app, tr(lang, "View"))
        .item(&it("page-live", "Live", Some("CmdOrCtrl+1"))?)
        .item(&it("page-board", "Game plan", Some("CmdOrCtrl+2"))?)
        .item(&it("page-members", "Members", Some("CmdOrCtrl+3"))?)
        .item(&it("page-extensions", "Extensions", Some("CmdOrCtrl+4"))?)
        .separator()
        .item(&it("sidebar", "Sidebar", Some("CmdOrCtrl+B"))?)
        .item(&it("fullscreen", "Full screen", Some("F11"))?)
        .separator()
        .item(&it("shortcuts", "Keyboard shortcuts", None)?)
        .build()?;
    let help = SubmenuBuilder::new(app, tr(lang, "Help"))
        .item(&it("diagnostics", "Diagnostics", None)?)
        .item(&it("update", "Check for updates", None)?)
        .separator()
        .item(&it("reset", "Reset app…", None)?)
        .separator()
        .item(&it("issues", "Report a problem", None)?)
        .item(&it("releases", "Release notes", None)?)
        .separator()
        .item(&it("about", "About", None)?)
        .build()?;
    MenuBuilder::new(app).items(&[&file, &room, &session, &view, &help]).build()
}

pub fn install_menus(app: &AppHandle, core: &Core) -> tauri::Result<()> {
    let lang = language(core);
    let recording = core.any_recording();
    let text = toggle_text(recording, core.party.lock().unwrap().reviewing());
    let keys = core.hotkeys.lock().unwrap()[0].clone();
    let toggle = item(app, &lang, "toggle-recording", text, None)?;
    let mark = item(app, &lang, "mark", "Mark this moment", hotkey(&keys).map(|_| keys.as_str()))?;
    mark.set_enabled(recording)?;
    let menu = build_menu(app, &lang, &toggle, &mark)?;
    if let Some(window) = app.get_webview_window("main") {
        window.set_theme(Some(tauri::Theme::Dark))?;
        window.set_menu(menu)?;
        let styled = window.clone();
        window.run_on_main_thread(move || dark_menu(&styled))?;
    }
    *ITEMS.lock().unwrap() = Items { toggle: Some(toggle), mark: Some(mark) };
    Ok(())
}

pub fn clock(seconds: i64) -> String {
    let s = seconds.max(0);
    match s / 3600 {
        0 => format!("{}:{:02}", s / 60, s % 60),
        h => format!("{h}:{:02}:{:02}", s % 3600 / 60, s % 60),
    }
}

fn status_text(core: &Core) -> String {
    let room = core.joined_room().ok().and(core.active_room()).map(|r| r.name);
    let timer = core.active_set.lock().unwrap().as_ref().map(|s| clock((crate::now_us() - s.start_us) / 1_000_000));
    match (room, timer) {
        (Some(room), Some(timer)) => format!("{room} · {timer}"),
        (Some(room), None) => room,
        _ => String::new(),
    }
}

fn ask_names(core: &Core) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = core.asks.lock().unwrap().iter().filter(|a| a.answer.is_none()).map(|a| (a.id.clone(), a.name.clone())).collect();
    for p in core.peers.lock().unwrap().iter().filter(|p| p.conn.is_some()) {
        out.extend(p.asks.iter().filter_map(|a| Some((a["id"].as_str()?.to_string(), a["name"].as_str().unwrap_or_default().to_string()))));
    }
    out
}

fn wants_text(lang: &str, name: &str) -> String {
    tr(lang, "{name} wants to join").replace("{name}", name)
}

pub fn setup(app: &tauri::App, core: Arc<Core>) -> tauri::Result<()> {
    install_menus(app.handle(), &core)?;

    let menu_core = core.clone();
    app.on_menu_event(move |app, event| {
        let core = &menu_core;
        let dialog = |run: fn(&AppHandle, &Arc<Core>)| {
            let (app, core) = (app.clone(), core.clone());
            std::thread::spawn(move || run(&app, &core));
        };
        match event.id().as_ref() {
            "open-folder" => crate::data::open_folder(&core.dir),
            "exit" => dialog(exit_app),
            "mark" => run_hotkey(core, 0),
            "issues" => open_link(&format!("{REPO}/issues/new")),
            "releases" => open_link(&format!("{REPO}/releases")),
            "about" => dialog(|app, core| { let lang = language(core); tell(app, &lang, &format!("{} {VERSION}\nMIT or Apache-2.0\n{REPO}", tr(&lang, "Version"))) }),
            other => {
                show_window(app);
                let _ = app.emit("menu", other.to_string());
            }
        }
    });

    let app = app.handle().clone();
    std::thread::spawn(move || {
        let mut last = None;
        let mut asked: HashSet<String> = HashSet::new();
        loop {
            let recording = core.any_recording();
            let reviewing = core.party.lock().unwrap().reviewing();
            let lang = language(&core);
            let hotkeys = core.hotkeys.lock().unwrap().clone();
            if last.as_ref() != Some(&(recording, reviewing, lang.clone(), hotkeys.clone())) {
                last = Some((recording, reviewing, lang.clone(), hotkeys.clone()));
                arm_hotkeys(&app, &hotkeys, recording);
                let items = ITEMS.lock().unwrap();
                if let Some(toggle) = &items.toggle {
                    let _ = toggle.set_text(tr(&lang, toggle_text(recording, reviewing)));
                }
                if let Some(mark) = &items.mark {
                    let _ = mark.set_enabled(recording);
                }
            }
            let status = status_text(&core);
            let asks = ask_names(&core);
            let hidden = !window_shown(&app);
            for (id, name) in &asks {
                if asked.insert(id.clone()) && hidden {
                    notify(&app, &wants_text(&lang, name), &status);
                }
            }
            asked.retain(|id| asks.iter().any(|(a, _)| a == id));
            std::thread::sleep(STATUS_POLL);
        }
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn colors_are_written_blue_first_for_windows() {
        assert_eq!(super::colorref(0x0f1115), 0x0015110f);
        assert_eq!(super::colorref(0xff0000), 0x0000ff);
    }

    use super::*;

    #[test]
    fn translations_are_unique_and_fall_back_to_english() {
        for (i, (en, _)) in JA.iter().enumerate() {
            assert!(!JA[..i].iter().any(|(other, _)| other == en), "{en}");
        }
        assert_eq!(tr("ja", "Room"), "ルーム");
        assert_eq!(tr("en", "Room"), "Room");
        assert_eq!(tr("ja", "Not translated"), "Not translated");
        assert_eq!(wants_text("ja", "Sora"), "Sora が参加を希望しています");
    }

    #[test]
    fn the_start_command_says_what_it_records() {
        assert_eq!(toggle_text(false, false), "Start recording");
        assert_eq!(toggle_text(false, true), "Record debrief");
        assert_eq!(toggle_text(true, true), "Stop recording");
    }

    #[test]
    fn only_plain_web_links_open() {
        assert!(link_ok("https://example.com/a?b=1&c=2"));
        assert!(link_ok("http://example.com"));
        assert!(link_ok("https://www.google.com/maps/@48.85,2.35,15z"));
        assert!(link_ok("https://ja.wikipedia.org/wiki/%E6%88%A6%E8%A1%93"));
        assert!(!link_ok("https://ja.wikipedia.org/wiki/戦術"));
        assert!(!link_ok("file:///C:/Windows/notepad.exe"));
        assert!(!link_ok("javascript:alert(1)"));
        assert!(!link_ok("ezh://moment/x/1/all"));
        assert!(!link_ok("https://example.com/a b"));
        assert!(!link_ok("https://example.com/\"x"));
        assert!(!link_ok(&format!("https://example.com/{}", "a".repeat(LINK_MAX))));
    }

    #[test]
    fn hotkeys_need_ctrl_or_alt_and_differ() {
        for (_, default, _) in HOTKEYS {
            assert!(hotkey(default).is_some());
        }
        assert!(hotkey("Alt+F9").is_some());
        assert!(hotkey("M").is_none());
        assert!(hotkey("Shift+M").is_none());
        assert!(hotkey("Ctrl+").is_none());
        assert!(hotkey("Ctrl+Nonsense").is_none());
        let keys = load_hotkeys(&Value::Null);
        assert_eq!(keys, HOTKEYS.map(|(_, default, _)| default.to_string()));
        assert_eq!(check_hotkey(&keys, 1, "ctrl+alt+m"), Err("Another action uses that shortcut."));
        assert_eq!(check_hotkey(&keys, 1, "Shift+M"), Err("Use Ctrl or Alt with a key."));
        assert!(check_hotkey(&keys, 0, "Ctrl+Alt+M").is_ok());
        assert!(check_hotkey(&keys, 2, "Super+F9").is_ok());
    }

    #[test]
    fn saved_hotkeys_load_and_never_repeat() {
        let keys = load_hotkeys(&json!({"hotkeys": {"mark": "Alt+F9", "screen": "Alt+F9"}}));
        assert_eq!(keys, ["Alt+F9", "Ctrl+Alt+N", "Ctrl+Alt+V"]);
        let keys = load_hotkeys(&json!({"hotkeys": {"mark": "Ctrl+Alt+N", "mic": "M"}}));
        assert_eq!(keys, ["Ctrl+Alt+N", "", "Ctrl+Alt+V"]);
    }

    #[test]
    fn closing_asks_only_while_work_would_be_lost() {
        assert_eq!(exit_question(false, false), None);
        assert_eq!(exit_question(true, true), Some("Cancel the export and exit?"));
        assert_eq!(exit_question(false, true), Some("Files are still arriving. Exit anyway?"));
    }

    #[test]
    fn clocks_read_like_the_player() {
        assert_eq!(clock(0), "0:00");
        assert_eq!(clock(2832), "47:12");
        assert_eq!(clock(4325), "1:12:05");
    }
}
