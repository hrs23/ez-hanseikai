use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::Value;
use tauri::menu::{Menu, MenuBuilder, MenuItem, MenuItemBuilder, SubmenuBuilder};
use tauri::{AppHandle, Emitter, Manager, Wry};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Modifiers, Shortcut};
use tauri_plugin_notification::NotificationExt;

use crate::{room, Core, REPO, VERSION};

pub const DEFAULT_HOTKEY: &str = "Ctrl+Alt+M";
const STATUS_POLL: Duration = Duration::from_secs(1);
const JA: [(&str, &str); 51] = [
    ("GGI Hanseikai is already running with this data folder.", "GGI反省会はこのデータフォルダですでに動いています。"),
    ("GGI Hanseikai", "GGI反省会"),
    ("File", "ファイル"),
    ("Room", "ルーム"),
    ("Recording", "録画"),
    ("View", "表示"),
    ("Help", "ヘルプ"),
    ("Open data folder", "データフォルダを開く"),
    ("Export identity…", "IDをエクスポート…"),
    ("Import identity…", "IDをインポート…"),
    ("Settings…", "設定…"),
    ("Exit", "終了"),
    ("Copy invite code", "招待コードをコピー"),
    ("Cancel invite", "招待を取り消す"),
    ("Room settings…", "ルームの設定…"),
    ("Leave room", "ルームを離れる"),
    ("Members", "メンバー"),
    ("Plugins", "プラグイン"),
    ("Start recording", "録画開始"),
    ("Stop recording", "録画終了"),
    ("Record debrief", "反省会を録画"),
    ("Mark this moment", "この瞬間をマーク"),
    ("Add note", "メモを追加"),
    ("Previous recording", "前の録画"),
    ("Next recording", "次の録画"),
    ("Export to folder…", "フォルダにエクスポート…"),
    ("Delete…", "削除…"),
    ("Live", "ライブ"),
    ("Game plan", "作戦ボード"),
    ("Sidebar", "サイドバー"),
    ("Side panel", "サイドパネル"),
    ("Full screen", "全画面"),
    ("All players", "全員"),
    ("Next player", "次のプレイヤー"),
    ("Previous player", "前のプレイヤー"),
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
    ("Identity saved. Keep this file private: anyone with it can act as you.", "IDを保存しました。このファイルは誰にも渡さないでください。持っている人はあなたとして振る舞えます。"),
    ("That file is not an identity key.", "このファイルはIDの鍵ではありません。"),
    ("Importing replaces your identity, so your ID changes. Your current key is kept as key.bin.old.", "インポートするとIDが置き換わり、IDが変わります。今の鍵は key.bin.old として残ります。"),
    ("Import", "インポート"),
    ("Version", "バージョン"),
    ("Moment marked", "マークしました"),
];
const JA_WANTS: (&str, &str) = ("{name} wants to join", "{name} が参加を希望しています");

struct Items {
    toggle: Option<MenuItem<Wry>>,
    mark: Option<MenuItem<Wry>>,
}

static ITEMS: Mutex<Items> = Mutex::new(Items { toggle: None, mark: None });

pub fn tr(lang: &str, en: &'static str) -> &'static str {
    if lang != "ja" {
        return en;
    }
    JA.iter().find(|(e, _)| *e == en).map_or(en, |(_, ja)| ja)
}

fn lang(core: &Core) -> String {
    crate::language(core)
}

pub fn ask(app: &AppHandle, lang: &str, text: &str, ok: &str, cancel: &str) -> bool {
    app.dialog().message(text).title(tr(lang, "GGI Hanseikai")).kind(MessageDialogKind::Warning).buttons(MessageDialogButtons::OkCancelCustom(ok.into(), cancel.into())).blocking_show()
}

pub fn tell(app: &AppHandle, lang: &str, text: &str) {
    app.dialog().message(text).title(tr(lang, "GGI Hanseikai")).kind(MessageDialogKind::Info).buttons(MessageDialogButtons::Ok).blocking_show();
}

fn documents(app: &AppHandle) -> Option<PathBuf> {
    app.path().document_dir().ok()
}

pub fn save_file(app: &AppHandle, name: &str) -> Option<PathBuf> {
    let ext = Path::new(name).extension().and_then(|e| e.to_str()).unwrap_or_default();
    let mut dialog = app.dialog().file().set_file_name(name).add_filter(ext.to_uppercase(), &[ext]);
    if let Some(dir) = documents(app) {
        dialog = dialog.set_directory(dir);
    }
    dialog.blocking_save_file().and_then(|p| p.into_path().ok())
}

pub fn pick_folder(app: &AppHandle) -> Option<PathBuf> {
    let mut dialog = app.dialog().file();
    if let Some(dir) = documents(app) {
        dialog = dialog.set_directory(dir);
    }
    dialog.blocking_pick_folder().and_then(|p| p.into_path().ok())
}

fn open_file(app: &AppHandle, kind: &str, ext: &str) -> Option<PathBuf> {
    let mut dialog = app.dialog().file().add_filter(kind, &[ext]);
    if let Some(dir) = documents(app) {
        dialog = dialog.set_directory(dir);
    }
    dialog.blocking_pick_file().and_then(|p| p.into_path().ok())
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

pub fn already_running() {
    use windows::core::HSTRING;
    use windows::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONINFORMATION, MB_OK};
    let text = tr(crate::os_language(), "GGI Hanseikai is already running with this data folder.");
    unsafe { MessageBoxW(None, &HSTRING::from(text), &HSTRING::from(tr(crate::os_language(), "GGI Hanseikai")), MB_OK | MB_ICONINFORMATION) };
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

const CAPTION: u32 = 0x0f1115;
const CAPTION_TEXT: u32 = 0xe7e9ee;

fn colorref(rgb: u32) -> u32 {
    ((rgb & 0xff) << 16) | (rgb & 0xff00) | (rgb >> 16)
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
        let lang = lang(core);
        if !ask(app, &lang, tr(&lang, question), tr(&lang, "Exit"), tr(&lang, "Cancel")) {
            return;
        }
        crate::export::cancel(core);
    }
    room::end_set(core, false, None);
    app.exit(0);
}

pub fn export_identity(app: &AppHandle, core: &Arc<Core>) {
    let Some(path) = save_file(app, "ggi-hanseikai-identity.key") else { return };
    if std::fs::copy(core.dir.join("key.bin"), &path).is_ok() {
        tell(app, &lang(core), tr(&lang(core), "Identity saved. Keep this file private: anyone with it can act as you."));
    }
}

pub fn import_identity(app: &AppHandle, core: &Arc<Core>) {
    let Some(path) = open_file(app, "Key", "key") else { return };
    let lang = lang(core);
    if !std::fs::read(&path).is_ok_and(|b| b.len() == 32) {
        tell(app, &lang, tr(&lang, "That file is not an identity key."));
        return;
    }
    if !ask(app, &lang, tr(&lang, "Importing replaces your identity, so your ID changes. Your current key is kept as key.bin.old."), tr(&lang, "Import"), tr(&lang, "Cancel")) {
        return;
    }
    let _ = std::fs::copy(core.dir.join("key.bin"), core.dir.join("key.bin.old"));
    if std::fs::copy(&path, core.dir.join("key.bin")).is_ok() {
        app.restart();
    }
}

pub fn hotkey(text: &str) -> Option<Shortcut> {
    let shortcut = Shortcut::from_str(text).ok()?;
    shortcut.mods.intersects(Modifiers::CONTROL | Modifiers::ALT | Modifiers::SUPER).then_some(shortcut)
}

fn arm_hotkey(app: &AppHandle, text: &str, on: bool) -> bool {
    let shortcuts = app.global_shortcut();
    let _ = shortcuts.unregister_all();
    !on || hotkey(text).is_some_and(|s| shortcuts.register(s).is_ok())
}

pub fn set_hotkey(core: &Core, text: &str) -> Result<(), String> {
    hotkey(text).ok_or("Use Ctrl or Alt with a key.")?;
    let recording = core.active_set.lock().unwrap().is_some();
    if !arm_hotkey(&core.app, text, true) {
        let old = core.hotkey.lock().unwrap().clone();
        arm_hotkey(&core.app, &old, recording);
        return Err("Another app uses that shortcut.".into());
    }
    arm_hotkey(&core.app, text, recording);
    *core.hotkey.lock().unwrap() = text.to_string();
    core.save_settings();
    rebuild_menus(&core.app, core);
    Ok(())
}

pub fn marked(core: &Arc<Core>) {
    if crate::ops::mark(core, Value::Null).is_ok() && !window_shown(&core.app) {
        notify(&core.app, tr(&lang(core), "Moment marked"), "");
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
        .item(&it("export-identity", "Export identity…", None)?)
        .item(&it("import-identity", "Import identity…", None)?)
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
        .item(&it("prev-session", "Previous recording", None)?)
        .item(&it("next-session", "Next recording", None)?)
        .separator()
        .item(&it("export-folder", "Export to folder…", None)?)
        .item(&it("delete-session", "Delete…", None)?)
        .build()?;
    let view = SubmenuBuilder::new(app, tr(lang, "View"))
        .item(&it("page-live", "Live", Some("CmdOrCtrl+1"))?)
        .item(&it("page-board", "Game plan", Some("CmdOrCtrl+2"))?)
        .item(&it("page-members", "Members", Some("CmdOrCtrl+3"))?)
        .item(&it("page-plugins", "Plugins", Some("CmdOrCtrl+4"))?)
        .separator()
        .item(&it("sidebar", "Sidebar", Some("CmdOrCtrl+B"))?)
        .item(&it("events-panel", "Side panel", None)?)
        .item(&it("fullscreen", "Full screen", Some("F11"))?)
        .separator()
        .item(&it("all-players", "All players", None)?)
        .item(&it("next-player", "Next player", None)?)
        .item(&it("prev-player", "Previous player", None)?)
        .separator()
        .item(&it("shortcuts", "Keyboard shortcuts", None)?)
        .build()?;
    let help = SubmenuBuilder::new(app, tr(lang, "Help"))
        .item(&it("diagnostics", "Diagnostics", None)?)
        .item(&it("update", "Check for updates", None)?)
        .separator()
        .item(&it("reset", "Reset app…", None)?)
        .separator()
        .item(&it("github", "GitHub", None)?)
        .item(&it("issues", "Report a problem", None)?)
        .item(&it("releases", "Release notes", None)?)
        .separator()
        .item(&it("about", "About", None)?)
        .build()?;
    MenuBuilder::new(app).items(&[&file, &room, &session, &view, &help]).build()
}

fn install_menus(app: &AppHandle, core: &Core) -> tauri::Result<()> {
    let lang = lang(core);
    let recording = core.active_set.lock().unwrap().is_some();
    let text = toggle_text(recording, core.party.lock().unwrap().reviewing());
    let keys = core.hotkey.lock().unwrap().clone();
    let toggle = item(app, &lang, "toggle-recording", text, None)?;
    let mark = item(app, &lang, "mark", "Mark this moment", hotkey(&keys).map(|_| keys.as_str()))?;
    mark.set_enabled(recording)?;
    app.set_menu(build_menu(app, &lang, &toggle, &mark)?)?;
    *ITEMS.lock().unwrap() = Items { toggle: Some(toggle), mark: Some(mark) };
    Ok(())
}

pub fn rebuild_menus(app: &AppHandle, core: &Core) {
    let _ = install_menus(app, core);
}

pub fn clock(seconds: i64) -> String {
    let s = seconds.max(0);
    match s / 3600 {
        0 => format!("{}:{:02}", s / 60, s % 60),
        h => format!("{h}:{:02}:{:02}", s % 3600 / 60, s % 60),
    }
}

fn status_text(core: &Core, lang: &str) -> String {
    let room = core.active_room().filter(|r| r.invite.is_none()).map(|r| r.name);
    let timer = core.active_set.lock().unwrap().as_ref().map(|s| clock((crate::now_us() - s.start_us) / 1_000_000));
    match (room, timer) {
        (Some(room), Some(timer)) => format!("{room} · {timer}"),
        (Some(room), None) => room,
        _ => tr(lang, "GGI Hanseikai").into(),
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
    if lang == "ja" { JA_WANTS.1 } else { JA_WANTS.0 }.replace("{name}", name)
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
            "export-identity" => dialog(export_identity),
            "import-identity" => dialog(import_identity),
            "exit" => dialog(exit_app),
            "mark" => marked(core),
            "github" => open_link(REPO),
            "issues" => open_link(&format!("{REPO}/issues/new")),
            "releases" => open_link(&format!("{REPO}/releases")),
            "about" => dialog(|app, core| { let lang = lang(core); tell(app, &lang, &format!("{}\n{} {VERSION}\nMIT or Apache-2.0\n{REPO}", tr(&lang, "GGI Hanseikai"), tr(&lang, "Version"))) }),
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
            let recording = core.active_set.lock().unwrap().is_some();
            let reviewing = core.party.lock().unwrap().reviewing();
            let lang = lang(&core);
            let hotkey = core.hotkey.lock().unwrap().clone();
            if last.as_ref() != Some(&(recording, reviewing, lang.clone(), hotkey.clone())) {
                last = Some((recording, reviewing, lang.clone(), hotkey.clone()));
                arm_hotkey(&app, &hotkey, recording);
                let items = ITEMS.lock().unwrap();
                if let Some(toggle) = &items.toggle {
                    let _ = toggle.set_text(tr(&lang, toggle_text(recording, reviewing)));
                }
                if let Some(mark) = &items.mark {
                    let _ = mark.set_enabled(recording);
                }
            }
            let status = status_text(&core, &lang);
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
    fn the_hotkey_needs_ctrl_or_alt() {
        assert!(hotkey(DEFAULT_HOTKEY).is_some());
        assert!(hotkey("Alt+F9").is_some());
        assert!(hotkey("M").is_none());
        assert!(hotkey("Shift+M").is_none());
        assert!(hotkey("Ctrl+").is_none());
        assert!(hotkey("Ctrl+Nonsense").is_none());
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
