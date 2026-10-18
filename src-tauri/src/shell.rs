use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tauri::image::Image;
use tauri::menu::{Menu, MenuBuilder, MenuItem, MenuItemBuilder, SubmenuBuilder};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, Theme, WebviewUrl, WebviewWindowBuilder, Wry};

use crate::{room, Core, NO_WINDOW, VERSION};

const REPO: &str = "https://github.com/hrs23/ez-hanseikai";
const TRAY_POLL: Duration = Duration::from_secs(1);
const DEBUG_SIZE: (f64, f64) = (1100.0, 720.0);
const JA: [(&str, &str); 45] = [
    ("File", "ファイル"),
    ("Room", "部屋"),
    ("Recording", "録画"),
    ("View", "表示"),
    ("Help", "ヘルプ"),
    ("Open data folder", "データフォルダを開く"),
    ("Storage...", "ストレージ..."),
    ("Export set as one video...", "セットを1本の動画に書き出す..."),
    ("Export identity...", "IDを書き出す..."),
    ("Import identity...", "IDを読み込む..."),
    ("Settings...", "設定..."),
    ("Exit", "終了"),
    ("Create room", "部屋を作る"),
    ("Join room...", "部屋に参加..."),
    ("Copy invite code", "招待コードをコピー"),
    ("Cancel invite", "招待を取り消す"),
    ("Rename room...", "部屋の名前を変更..."),
    ("Check connections...", "接続を確認..."),
    ("Plugins...", "プラグイン..."),
    ("Leave room", "部屋を抜ける"),
    ("Start recording", "録画を開始"),
    ("Stop recording", "録画を停止"),
    ("Record", "録画"),
    ("Player", "プレイヤー"),
    ("Full screen", "全画面"),
    ("Fill window", "ウィンドウに合わせる"),
    ("All players", "全員"),
    ("Next player", "次のプレイヤー"),
    ("Previous player", "前のプレイヤー"),
    ("Keyboard shortcuts", "キーボードショートカット"),
    ("Debug", "デバッグ"),
    ("Check for updates", "更新を確認"),
    ("Report a problem", "問題を報告"),
    ("Release notes", "リリースノート"),
    ("About", "バージョン情報"),
    ("Open", "開く"),
    ("EZ Hanseikai - recording", "EZ Hanseikai - 録画中"),
    ("Recording is running. Stop recording and exit?", "録画中です。録画を止めて終了しますか?"),
    ("Identity saved. Keep this file private: anyone with it can act as you.", "IDを保存しました。このファイルは誰にも渡さないでください。持っている人はあなたとして振る舞えます。"),
    ("That file is not an identity key.", "このファイルはIDの鍵ではありません。"),
    ("Importing replaces your identity, so your ID changes. Your current key is kept as key.bin.old. Continue?", "読み込むとIDが置き換わり、IDが変わります。今の鍵は key.bin.old として残ります。続けますか?"),
    ("Version", "バージョン"),
    ("Next tab", "次のタブ"),
    ("Previous tab", "前のタブ"),
    ("GitHub", "GitHub"),
];

static TOGGLES: Mutex<Vec<MenuItem<Wry>>> = Mutex::new(Vec::new());

pub fn tr(lang: &str, en: &'static str) -> &'static str {
    if lang != "ja" {
        return en;
    }
    JA.iter().find(|(e, _)| *e == en).map_or(en, |(_, ja)| ja)
}

fn powershell(script: &str) -> String {
    Command::new("powershell")
        .args(["-NoProfile", "-STA", "-Command", script])
        .creation_flags(NO_WINDOW)
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}

fn quote(text: &str) -> String {
    text.replace('\'', "''")
}

pub fn message(text: &str, buttons: &str) -> bool {
    let script = format!(
        "Add-Type -AssemblyName System.Windows.Forms; [int][System.Windows.Forms.MessageBox]::Show('{}', 'EZ Hanseikai', '{buttons}')",
        quote(text)
    );
    powershell(&script) == "1"
}

pub fn pick_file(save: bool, name: &str) -> Option<PathBuf> {
    let kind = if save { "SaveFileDialog" } else { "OpenFileDialog" };
    let script = format!(
        "Add-Type -AssemblyName System.Windows.Forms; $d = New-Object System.Windows.Forms.{kind}; $d.FileName = '{}'; if ($d.ShowDialog() -eq 'OK') {{ $d.FileName }}",
        quote(name)
    );
    let path = powershell(&script);
    (!path.is_empty()).then(|| PathBuf::from(path))
}

fn open_url(url: &str) {
    let _ = Command::new("cmd").args(["/c", "start", "", url]).creation_flags(NO_WINDOW).spawn();
}

fn show_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
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

pub fn open_debug(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("debug") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
        return;
    }
    let built = WebviewWindowBuilder::new(app, "debug", WebviewUrl::App("debug.html".into()))
        .title("EZ Hanseikai Debug")
        .inner_size(DEBUG_SIZE.0, DEBUG_SIZE.1)
        .center()
        .theme(Some(Theme::Dark))
        .build();
    if let Ok(window) = built {
        let _ = window.remove_menu();
        plain_keys(&window);
    }
}

fn exit_app(app: &AppHandle, core: &Arc<Core>) {
    if core.active_set.lock().unwrap().is_some() {
        show_window(app);
        if !message(tr(&crate::language(core), "Recording is running. Stop recording and exit?"), "OKCancel") {
            return;
        }
        room::stop_set(core);
    }
    app.exit(0);
}

fn export_identity(core: &Core) {
    let Some(path) = pick_file(true, "ez-hanseikai-identity.key") else { return };
    if std::fs::copy(core.dir.join("key.bin"), &path).is_ok() {
        message(tr(&crate::language(core), "Identity saved. Keep this file private: anyone with it can act as you."), "OK");
    }
}

fn import_identity(app: &AppHandle, core: &Arc<Core>) {
    let Some(path) = pick_file(false, "") else { return };
    let valid = std::fs::read(&path).map(|b| b.len() == 32).unwrap_or(false);
    if !valid {
        message(tr(&crate::language(core), "That file is not an identity key."), "OK");
        return;
    }
    if !message(tr(&crate::language(core), "Importing replaces your identity, so your ID changes. Your current key is kept as key.bin.old. Continue?"), "OKCancel") {
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
    } else {
        let _ = room::start_set(core);
    }
}

fn item(app: &AppHandle, lang: &str, id: &str, text: &'static str, accel: Option<&str>) -> tauri::Result<MenuItem<Wry>> {
    let builder = MenuItemBuilder::with_id(id, tr(lang, text));
    match accel {
        Some(a) => builder.accelerator(a).build(app),
        None => builder.build(app),
    }
}

fn toggle_text(recording: bool) -> &'static str {
    if recording { "Stop recording" } else { "Start recording" }
}

fn build_menu(app: &AppHandle, lang: &str, toggle: &MenuItem<Wry>) -> tauri::Result<Menu<Wry>> {
    let it = |id: &str, text: &'static str, accel: Option<&str>| item(app, lang, id, text, accel);
    let file = SubmenuBuilder::new(app, tr(lang, "File"))
        .item(&it("open-folder", "Open data folder", None)?)
        .item(&it("storage", "Storage...", None)?)
        .item(&it("export-set", "Export set as one video...", None)?)
        .separator()
        .item(&it("export-identity", "Export identity...", None)?)
        .item(&it("import-identity", "Import identity...", None)?)
        .separator()
        .item(&it("settings", "Settings...", Some("CmdOrCtrl+,"))?)
        .separator()
        .item(&it("exit", "Exit", None)?)
        .build()?;
    let room = SubmenuBuilder::new(app, tr(lang, "Room"))
        .item(&it("create-room", "Create room", None)?)
        .item(&it("join-room", "Join room...", None)?)
        .separator()
        .item(&it("copy-invite", "Copy invite code", None)?)
        .item(&it("cancel-invite", "Cancel invite", None)?)
        .item(&it("rename-room", "Rename room...", None)?)
        .item(&it("check-room", "Check connections...", None)?)
        .item(&it("plugins", "Plugins...", None)?)
        .separator()
        .item(&it("leave-room", "Leave room", None)?)
        .build()?;
    let recording = SubmenuBuilder::new(app, tr(lang, "Recording")).item(toggle).build()?;
    let view = SubmenuBuilder::new(app, tr(lang, "View"))
        .item(&it("tab-room", "Room", Some("CmdOrCtrl+1"))?)
        .item(&it("tab-record", "Record", Some("CmdOrCtrl+2"))?)
        .item(&it("tab-player", "Player", Some("CmdOrCtrl+3"))?)
        .separator()
        .item(&it("fullscreen", "Full screen", Some("F11"))?)
        .item(&it("fill", "Fill window", None)?)
        .item(&it("all-players", "All players", None)?)
        .item(&it("next-player", "Next player", None)?)
        .item(&it("prev-player", "Previous player", None)?)
        .separator()
        .item(&it("shortcuts", "Keyboard shortcuts", None)?)
        .build()?;
    let help = SubmenuBuilder::new(app, tr(lang, "Help"))
        .item(&it("debug", "Debug", Some("CmdOrCtrl+Shift+I"))?)
        .item(&it("update", "Check for updates", None)?)
        .separator()
        .item(&it("github", "GitHub", None)?)
        .item(&it("issues", "Report a problem", None)?)
        .item(&it("releases", "Release notes", None)?)
        .separator()
        .item(&it("about", "About", None)?)
        .build()?;
    MenuBuilder::new(app).items(&[&file, &room, &recording, &view, &help]).build()
}

fn tray_menu(app: &AppHandle, lang: &str, toggle: &MenuItem<Wry>) -> tauri::Result<Menu<Wry>> {
    MenuBuilder::new(app).item(toggle).item(&item(app, lang, "open", "Open", None)?).separator().item(&item(app, lang, "exit", "Exit", None)?).build()
}

fn install_menus(app: &AppHandle, core: &Core) -> tauri::Result<()> {
    let lang = crate::language(core);
    let text = toggle_text(core.active_set.lock().unwrap().is_some());
    let bar_toggle = item(app, &lang, "toggle-recording", text, Some("CmdOrCtrl+R"))?;
    let tray_toggle = item(app, &lang, "toggle-recording", text, None)?;
    app.set_menu(build_menu(app, &lang, &bar_toggle)?)?;
    if let Some(tray) = app.tray_by_id("main") {
        tray.set_menu(Some(tray_menu(app, &lang, &tray_toggle)?))?;
    }
    *TOGGLES.lock().unwrap() = vec![bar_toggle, tray_toggle];
    Ok(())
}

pub fn rebuild_menus(app: &AppHandle, core: &Core) {
    let _ = install_menus(app, core);
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
            "export-identity" => dialog(|_, core| export_identity(core)),
            "import-identity" => dialog(import_identity),
            "exit" => dialog(exit_app),
            "open" => show_window(app),
            "toggle-recording" => toggle_recording(core),
            "debug" => open_debug(app),
            "github" => open_url(REPO),
            "issues" => open_url(&format!("{REPO}/issues/new")),
            "releases" => open_url(&format!("{REPO}/releases")),
            "about" => dialog(|_, core| {
                message(&format!("EZ Hanseikai\n{} {VERSION}\nMIT or Apache-2.0\n{REPO}", tr(&crate::language(core), "Version")), "OK");
            }),
            other => {
                show_window(app);
                let _ = app.emit("menu", other.to_string());
            }
        }
    });

    std::thread::spawn(move || {
        let mut last = None;
        loop {
            let recording = core.active_set.lock().unwrap().is_some();
            let working = recording && core.engine.working();
            let lang = crate::language(&core);
            if last.as_ref() != Some(&(recording, working, lang.clone())) {
                last = Some((recording, working, lang.clone()));
                for toggle in TOGGLES.lock().unwrap().iter() {
                    let _ = toggle.set_text(tr(&lang, toggle_text(recording)));
                }
                let _ = tray.set_tooltip(Some(if working { tr(&lang, "EZ Hanseikai - recording") } else { "EZ Hanseikai" }));
                let _ = tray.set_icon(Some(if working { rec_icon.clone() } else { idle_icon.clone() }));
            }
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
        assert_eq!(tr("ja", "Room"), "部屋");
        assert_eq!(tr("en", "Room"), "Room");
        assert_eq!(tr("ja", "Not translated"), "Not translated");
    }
}
