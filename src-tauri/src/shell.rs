use std::collections::HashSet;
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::Command;
use std::str::FromStr;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tauri::image::Image;
use tauri::menu::{Menu, MenuBuilder, MenuItem, MenuItemBuilder, SubmenuBuilder};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, Theme, WebviewUrl, WebviewWindowBuilder, Wry};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Modifiers, Shortcut};
use tauri_plugin_notification::NotificationExt;

use crate::{room, Core, NO_WINDOW, VERSION};

pub const DEFAULT_HOTKEY: &str = "Ctrl+Alt+M";
const REPO: &str = "https://github.com/hrs23/ez-hanseikai";
const TRAY_POLL: Duration = Duration::from_secs(1);
const DEBUG_SIZE: (f64, f64) = (1100.0, 720.0);
const JA: [(&str, &str); 52] = [
    ("File", "ファイル"),
    ("Room", "ルーム"),
    ("Session", "セッション"),
    ("View", "表示"),
    ("Help", "ヘルプ"),
    ("Open data folder", "データフォルダを開く"),
    ("Storage…", "ストレージ…"),
    ("Export identity…", "IDを書き出す…"),
    ("Import identity…", "IDを読み込む…"),
    ("Settings…", "設定…"),
    ("Exit", "終了"),
    ("Create room", "ルームを作成"),
    ("Join room…", "ルームに参加…"),
    ("Copy invite code", "招待コードをコピー"),
    ("Cancel invite", "招待を取り消す"),
    ("Rename room…", "ルーム名を変更…"),
    ("Check connections", "接続を確認"),
    ("Leave room…", "ルームから退出…"),
    ("Start session", "セッション開始"),
    ("End session", "セッション終了"),
    ("Mark this moment", "この瞬間をマーク"),
    ("Add note", "メモを追加"),
    ("Previous session", "前のセッション"),
    ("Next session", "次のセッション"),
    ("Export video…", "動画を書き出す…"),
    ("Delete from this PC…", "このPCから削除…"),
    ("Now", "いま"),
    ("Capture", "キャプチャ"),
    ("Sidebar", "サイドバー"),
    ("Events panel", "イベント欄"),
    ("Full screen", "全画面"),
    ("All players", "全員"),
    ("Next player", "次のプレイヤー"),
    ("Previous player", "前のプレイヤー"),
    ("Keyboard shortcuts", "キーボードショートカット"),
    ("Diagnostics", "診断"),
    ("Check for updates", "更新を確認"),
    ("Report a problem", "問題を報告"),
    ("Release notes", "リリースノート"),
    ("About", "バージョン情報"),
    ("Show EZ Hanseikai", "EZ Hanseikai を表示"),
    ("End the session for everyone and exit?", "全員のセッションを終了してアプリを終了しますか?"),
    ("End session and exit", "セッションを終了して終了"),
    ("Cancel", "キャンセル"),
    ("Identity saved. Keep this file private: anyone with it can act as you.", "IDを保存しました。このファイルは誰にも渡さないでください。持っている人はあなたとして振る舞えます。"),
    ("That file is not an identity key.", "このファイルはIDの鍵ではありません。"),
    ("Importing replaces your identity, so your ID changes. Your current key is kept as key.bin.old.", "読み込むとIDが置き換わり、IDが変わります。今の鍵は key.bin.old として残ります。"),
    ("Import", "読み込む"),
    ("Version", "バージョン"),
    ("EZ Hanseikai is still running", "EZ Hanseikai はバックグラウンドで動作中です"),
    ("Exit from the tray icon.", "終了はトレイアイコンから。"),
    ("Moment marked", "マークしました"),
];
const JA_WANTS: (&str, &str) = ("{name} wants to join", "{name} が参加を希望しています");

struct Items {
    toggles: Vec<MenuItem<Wry>>,
    marks: Vec<MenuItem<Wry>>,
    status: Option<MenuItem<Wry>>,
}

static ITEMS: Mutex<Items> = Mutex::new(Items { toggles: Vec::new(), marks: Vec::new(), status: None });

pub fn tr(lang: &str, en: &'static str) -> &'static str {
    if lang != "ja" {
        return en;
    }
    JA.iter().find(|(e, _)| *e == en).map_or(en, |(_, ja)| ja)
}

fn lang(core: &Core) -> String {
    crate::language(core)
}

pub fn ask(app: &AppHandle, text: &str, ok: &str, cancel: &str) -> bool {
    app.dialog().message(text).title("EZ Hanseikai").kind(MessageDialogKind::Warning).buttons(MessageDialogButtons::OkCancelCustom(ok.into(), cancel.into())).blocking_show()
}

pub fn tell(app: &AppHandle, text: &str) {
    app.dialog().message(text).title("EZ Hanseikai").kind(MessageDialogKind::Info).buttons(MessageDialogButtons::Ok).blocking_show();
}

fn documents(app: &AppHandle) -> Option<PathBuf> {
    app.path().document_dir().ok()
}

pub fn save_file(app: &AppHandle, name: &str, kind: &str, ext: &str) -> Option<PathBuf> {
    let mut dialog = app.dialog().file().set_file_name(name).add_filter(kind, &[ext]);
    if let Some(dir) = documents(app) {
        dialog = dialog.set_directory(dir);
    }
    dialog.blocking_save_file().and_then(|p| p.into_path().ok())
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

fn open_url(url: &str) {
    let _ = Command::new("cmd").args(["/c", "start", "", url]).creation_flags(NO_WINDOW).spawn();
}

fn show_window(app: &AppHandle) {
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

pub fn open_debug(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("debug") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
        return;
    }
    let built = WebviewWindowBuilder::new(app, "debug", WebviewUrl::App("debug.html".into()))
        .title("EZ Hanseikai Diagnostics")
        .inner_size(DEBUG_SIZE.0, DEBUG_SIZE.1)
        .center()
        .theme(Some(Theme::Dark))
        .build();
    if let Ok(window) = built {
        let _ = window.remove_menu();
        plain_keys(&window);
    }
}

pub fn keep_running(recording: bool, online: bool) -> bool {
    recording || online
}

pub fn close_main(app: &AppHandle, core: &Core) {
    let recording = core.active_set.lock().unwrap().is_some();
    let online = core.peers.lock().unwrap().iter().any(|p| p.conn.is_some());
    if !keep_running(recording, online) {
        app.exit(0);
        return;
    }
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.hide();
    }
    if !core.hid_once.swap(true, Ordering::Relaxed) {
        core.save_settings();
        let lang = lang(core);
        notify(app, tr(&lang, "EZ Hanseikai is still running"), tr(&lang, "Exit from the tray icon."));
    }
}

fn exit_app(app: &AppHandle, core: &Arc<Core>) {
    if core.active_set.lock().unwrap().is_some() {
        show_window(app);
        let lang = lang(core);
        if !ask(app, tr(&lang, "End the session for everyone and exit?"), tr(&lang, "End session and exit"), tr(&lang, "Cancel")) {
            return;
        }
        room::stop_set(core);
    }
    app.exit(0);
}

fn export_identity(app: &AppHandle, core: &Arc<Core>) {
    let Some(path) = save_file(app, "ez-hanseikai-identity.key", "Key", "key") else { return };
    if std::fs::copy(core.dir.join("key.bin"), &path).is_ok() {
        tell(app, tr(&lang(core), "Identity saved. Keep this file private: anyone with it can act as you."));
    }
}

fn import_identity(app: &AppHandle, core: &Arc<Core>) {
    let Some(path) = open_file(app, "Key", "key") else { return };
    let lang = lang(core);
    if !std::fs::read(&path).is_ok_and(|b| b.len() == 32) {
        tell(app, tr(&lang, "That file is not an identity key."));
        return;
    }
    if !ask(app, tr(&lang, "Importing replaces your identity, so your ID changes. Your current key is kept as key.bin.old."), tr(&lang, "Import"), tr(&lang, "Cancel")) {
        return;
    }
    let _ = std::fs::copy(core.dir.join("key.bin"), core.dir.join("key.bin.old"));
    if std::fs::copy(&path, core.dir.join("key.bin")).is_ok() {
        app.restart();
    }
}

pub fn toggle_recording(core: &Arc<Core>) {
    if core.active_set.lock().unwrap().is_some() {
        room::stop_set(core);
    } else if let Err(e) = room::start_set(core) {
        let _ = core.app.emit("flash", e);
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
    if crate::mark_moment(core).is_ok() && !window_shown(&core.app) {
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

fn toggle_text(recording: bool) -> &'static str {
    if recording { "End session" } else { "Start session" }
}

fn build_menu(app: &AppHandle, lang: &str, toggle: &MenuItem<Wry>, mark: &MenuItem<Wry>) -> tauri::Result<Menu<Wry>> {
    let it = |id: &str, text: &'static str, accel: Option<&str>| item(app, lang, id, text, accel);
    let file = SubmenuBuilder::new(app, tr(lang, "File"))
        .item(&it("open-folder", "Open data folder", None)?)
        .item(&it("storage", "Storage…", None)?)
        .separator()
        .item(&it("export-identity", "Export identity…", None)?)
        .item(&it("import-identity", "Import identity…", None)?)
        .separator()
        .item(&it("settings", "Settings…", Some("CmdOrCtrl+,"))?)
        .separator()
        .item(&it("exit", "Exit", None)?)
        .build()?;
    let room = SubmenuBuilder::new(app, tr(lang, "Room"))
        .item(&it("create-room", "Create room", None)?)
        .item(&it("join-room", "Join room…", None)?)
        .separator()
        .item(&it("copy-invite", "Copy invite code", None)?)
        .item(&it("cancel-invite", "Cancel invite", None)?)
        .item(&it("rename-room", "Rename room…", None)?)
        .item(&it("check-room", "Check connections", None)?)
        .separator()
        .item(&it("leave-room", "Leave room…", None)?)
        .build()?;
    let session = SubmenuBuilder::new(app, tr(lang, "Session"))
        .item(toggle)
        .item(mark)
        .item(&it("add-note", "Add note", None)?)
        .separator()
        .item(&it("prev-session", "Previous session", None)?)
        .item(&it("next-session", "Next session", None)?)
        .separator()
        .item(&it("export-session", "Export video…", None)?)
        .item(&it("delete-session", "Delete from this PC…", None)?)
        .build()?;
    let view = SubmenuBuilder::new(app, tr(lang, "View"))
        .item(&it("page-room", "Room", Some("CmdOrCtrl+1"))?)
        .item(&it("page-now", "Now", Some("CmdOrCtrl+2"))?)
        .item(&it("page-capture", "Capture", Some("CmdOrCtrl+3"))?)
        .separator()
        .item(&it("sidebar", "Sidebar", Some("CmdOrCtrl+B"))?)
        .item(&it("events-panel", "Events panel", None)?)
        .item(&it("fullscreen", "Full screen", Some("F11"))?)
        .separator()
        .item(&it("all-players", "All players", None)?)
        .item(&it("next-player", "Next player", None)?)
        .item(&it("prev-player", "Previous player", None)?)
        .separator()
        .item(&it("shortcuts", "Keyboard shortcuts", None)?)
        .build()?;
    let help = SubmenuBuilder::new(app, tr(lang, "Help"))
        .item(&it("debug", "Diagnostics", Some("CmdOrCtrl+Shift+I"))?)
        .item(&it("update", "Check for updates", None)?)
        .separator()
        .item(&it("github", "GitHub", None)?)
        .item(&it("issues", "Report a problem", None)?)
        .item(&it("releases", "Release notes", None)?)
        .separator()
        .item(&it("about", "About", None)?)
        .build()?;
    MenuBuilder::new(app).items(&[&file, &room, &session, &view, &help]).build()
}

fn tray_menu(app: &AppHandle, lang: &str, status: &MenuItem<Wry>, toggle: &MenuItem<Wry>, mark: &MenuItem<Wry>) -> tauri::Result<Menu<Wry>> {
    MenuBuilder::new(app)
        .item(status)
        .separator()
        .item(&item(app, lang, "show", "Show EZ Hanseikai", None)?)
        .item(mark)
        .item(toggle)
        .separator()
        .item(&item(app, lang, "exit", "Exit", None)?)
        .build()
}

fn install_menus(app: &AppHandle, core: &Core) -> tauri::Result<()> {
    let lang = lang(core);
    let recording = core.active_set.lock().unwrap().is_some();
    let text = toggle_text(recording);
    let keys = core.hotkey.lock().unwrap().clone();
    let bar_toggle = item(app, &lang, "toggle-recording", text, Some("CmdOrCtrl+R"))?;
    let tray_toggle = item(app, &lang, "toggle-recording", text, None)?;
    let bar_mark = item(app, &lang, "mark", "Mark this moment", hotkey(&keys).map(|_| keys.as_str()))?;
    let tray_mark = item(app, &lang, "mark", "Mark this moment", None)?;
    let status = MenuItemBuilder::with_id("status", status_text(core)).enabled(false).build(app)?;
    for m in [&bar_mark, &tray_mark] {
        m.set_enabled(recording)?;
    }
    app.set_menu(build_menu(app, &lang, &bar_toggle, &bar_mark)?)?;
    if let Some(tray) = app.tray_by_id("main") {
        tray.set_menu(Some(tray_menu(app, &lang, &status, &tray_toggle, &tray_mark)?))?;
    }
    *ITEMS.lock().unwrap() = Items { toggles: vec![bar_toggle, tray_toggle], marks: vec![bar_mark, tray_mark], status: Some(status) };
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

fn status_text(core: &Core) -> String {
    let room = core.active_room().filter(|r| r.invite.is_none()).map(|r| r.name);
    let timer = core.active_set.lock().unwrap().as_ref().map(|s| clock((crate::now_us() - s.start_us) / 1_000_000));
    match (room, timer) {
        (Some(room), Some(timer)) => format!("{room} · {timer}"),
        (Some(room), None) => room,
        _ => "EZ Hanseikai".into(),
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
    let source = app.default_window_icon().unwrap();
    let idle_icon = Image::new_owned(source.rgba().to_vec(), source.width(), source.height());
    let rec_icon = Image::from_bytes(include_bytes!("../icons/tray-rec.png"))?;
    let tray = TrayIconBuilder::with_id("main")
        .icon(idle_icon.clone())
        .tooltip("EZ Hanseikai")
        .show_menu_on_left_click(false)
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                show_window(tray.app_handle());
            }
        })
        .build(app)?;
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
            "show" => show_window(app),
            "toggle-recording" => toggle_recording(core),
            "mark" => marked(core),
            "debug" => open_debug(app),
            "github" => open_url(REPO),
            "issues" => open_url(&format!("{REPO}/issues/new")),
            "releases" => open_url(&format!("{REPO}/releases")),
            "about" => dialog(|app, core| tell(app, &format!("EZ Hanseikai\n{} {VERSION}\nMIT or Apache-2.0\n{REPO}", tr(&lang(core), "Version")))),
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
            let working = recording && core.engine.working();
            let lang = lang(&core);
            let hotkey = core.hotkey.lock().unwrap().clone();
            if last.as_ref() != Some(&(recording, working, lang.clone(), hotkey.clone())) {
                last = Some((recording, working, lang.clone(), hotkey.clone()));
                arm_hotkey(&app, &hotkey, recording);
                let items = ITEMS.lock().unwrap();
                for toggle in &items.toggles {
                    let _ = toggle.set_text(tr(&lang, toggle_text(recording)));
                }
                for mark in &items.marks {
                    let _ = mark.set_enabled(recording);
                }
                let _ = tray.set_icon(Some(if working { rec_icon.clone() } else { idle_icon.clone() }));
            }
            let status = status_text(&core);
            if let Some(item) = ITEMS.lock().unwrap().status.as_ref() {
                let _ = item.set_text(&status);
            }
            let _ = tray.set_tooltip(Some(if recording { status.as_str() } else { "EZ Hanseikai" }));
            let asks = ask_names(&core);
            let hidden = !window_shown(&app);
            for (id, name) in &asks {
                if asked.insert(id.clone()) && hidden {
                    notify(&app, &wants_text(&lang, name), &status);
                }
            }
            asked.retain(|id| asks.iter().any(|(a, _)| a == id));
            std::thread::sleep(TRAY_POLL);
        }
    });
    Ok(())
}

#[cfg(test)]
mod tests {
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
    fn the_hotkey_needs_ctrl_or_alt() {
        assert!(hotkey(DEFAULT_HOTKEY).is_some());
        assert!(hotkey("Alt+F9").is_some());
        assert!(hotkey("M").is_none());
        assert!(hotkey("Shift+M").is_none());
        assert!(hotkey("Ctrl+").is_none());
        assert!(hotkey("Ctrl+Nonsense").is_none());
    }

    #[test]
    fn the_window_hides_only_while_needed() {
        assert!(keep_running(true, false));
        assert!(keep_running(false, true));
        assert!(!keep_running(false, false));
    }

    #[test]
    fn clocks_read_like_the_player() {
        assert_eq!(clock(0), "0:00");
        assert_eq!(clock(2832), "47:12");
        assert_eq!(clock(4325), "1:12:05");
    }
}
