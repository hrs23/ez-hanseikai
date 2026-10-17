use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use tauri::image::Image;
use tauri::menu::{Menu, MenuBuilder, MenuItem, MenuItemBuilder, SubmenuBuilder};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{App, AppHandle, Emitter, Manager, Theme, WebviewUrl, WebviewWindowBuilder};

use crate::{room, Core, VERSION};

const REPO: &str = "https://github.com/hrs23/ez-hanseikai";
const TRAY_POLL: Duration = Duration::from_secs(1);
const NO_WINDOW: u32 = 0x08000000;
const DEBUG_SIZE: (f64, f64) = (760.0, 640.0);

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

fn open_debug(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("debug") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
        return;
    }
    let built = WebviewWindowBuilder::new(app, "debug", WebviewUrl::App("debug.html".into()))
        .title("EZ Hanseikai Debug")
        .inner_size(DEBUG_SIZE.0, DEBUG_SIZE.1)
        .theme(Some(Theme::Dark))
        .build();
    if let Ok(window) = built {
        let _ = window.remove_menu();
    }
}

fn exit_app(app: &AppHandle, core: &Arc<Core>) {
    if core.active_set.lock().unwrap().is_some() {
        show_window(app);
        if !message("Recording is running. Stop recording and exit?", "OKCancel") {
            return;
        }
        room::stop_set(core);
    }
    app.exit(0);
}

fn export_identity(core: &Core) {
    let Some(path) = pick_file(true, "ez-hanseikai-identity.key") else { return };
    if std::fs::copy(core.dir.join("key.bin"), &path).is_ok() {
        message("Identity saved. Keep this file private: anyone with it can act as you.", "OK");
    }
}

fn import_identity(app: &AppHandle, core: &Core) {
    let Some(path) = pick_file(false, "") else { return };
    let valid = std::fs::read(&path).map(|b| b.len() == 32).unwrap_or(false);
    if !valid {
        message("That file is not an identity key.", "OK");
        return;
    }
    if !message("Importing replaces your identity, so your ID changes. Your current key is kept as key.bin.old. Continue?", "OKCancel") {
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

fn item(app: &App, id: &str, text: &str, accel: Option<&str>) -> tauri::Result<MenuItem<tauri::Wry>> {
    let builder = MenuItemBuilder::with_id(id, text);
    match accel {
        Some(a) => builder.accelerator(a).build(app),
        None => builder.build(app),
    }
}

fn build_menu(app: &App, toggle: &MenuItem<tauri::Wry>) -> tauri::Result<Menu<tauri::Wry>> {
    let file = SubmenuBuilder::new(app, "File")
        .item(&item(app, "open-folder", "Open data folder", None)?)
        .item(&item(app, "export-set", "Export set as one video...", None)?)
        .separator()
        .item(&item(app, "export-identity", "Export identity...", None)?)
        .item(&item(app, "import-identity", "Import identity...", None)?)
        .separator()
        .item(&item(app, "settings", "Settings...", Some("CmdOrCtrl+,"))?)
        .separator()
        .item(&item(app, "exit", "Exit", None)?)
        .build()?;
    let room = SubmenuBuilder::new(app, "Room")
        .item(&item(app, "create-room", "Create room", None)?)
        .item(&item(app, "join-room", "Join room...", None)?)
        .item(&item(app, "copy-invite", "Copy invite code", None)?)
        .item(&item(app, "cancel-invite", "Cancel invite", None)?)
        .separator()
        .item(&item(app, "leave-room", "Leave room", None)?)
        .build()?;
    let recording = SubmenuBuilder::new(app, "Recording")
        .item(toggle)
        .build()?;
    let view = SubmenuBuilder::new(app, "View")
        .item(&item(app, "fullscreen", "Full screen", Some("F11"))?)
        .item(&item(app, "all-players", "All players", None)?)
        .item(&item(app, "next-player", "Next player", None)?)
        .item(&item(app, "prev-player", "Previous player", None)?)
        .separator()
        .item(&item(app, "shortcuts", "Keyboard shortcuts", None)?)
        .build()?;
    let help = SubmenuBuilder::new(app, "Help")
        .item(&item(app, "debug", "Debug", None)?)
        .item(&item(app, "update", "Check for updates", None)?)
        .separator()
        .item(&item(app, "github", "GitHub", None)?)
        .item(&item(app, "issues", "Report a problem", None)?)
        .item(&item(app, "releases", "Release notes", None)?)
        .separator()
        .item(&item(app, "about", "About", None)?)
        .build()?;
    MenuBuilder::new(app).items(&[&file, &room, &recording, &view, &help]).build()
}

pub fn setup(app: &App, core: Arc<Core>) -> tauri::Result<()> {
    let bar_toggle = item(app, "toggle-recording", "Start recording", Some("CmdOrCtrl+R"))?;
    app.set_menu(build_menu(app, &bar_toggle)?)?;
    let tray_toggle = item(app, "toggle-recording", "Start recording", None)?;
    let tray_menu = MenuBuilder::new(app)
        .item(&tray_toggle)
        .item(&item(app, "open", "Open", None)?)
        .separator()
        .item(&item(app, "exit", "Exit", None)?)
        .build()?;
    let source = app.default_window_icon().unwrap();
    let idle_icon = Image::new_owned(source.rgba().to_vec(), source.width(), source.height());
    let rec_icon = Image::from_bytes(include_bytes!("../icons/tray-rec.png"))?;
    let tray = TrayIconBuilder::with_id("main")
        .icon(idle_icon.clone())
        .tooltip("EZ Hanseikai")
        .menu(&tray_menu)
        .show_menu_on_left_click(false)
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                show_window(tray.app_handle());
            }
        })
        .build(app)?;

    let menu_core = core.clone();
    app.on_menu_event(move |app, event| {
        let core = &menu_core;
        match event.id().as_ref() {
            "open-folder" => crate::data::open_folder(&core.dir),
            "export-identity" => export_identity(core),
            "import-identity" => import_identity(app, core),
            "exit" => exit_app(app, core),
            "open" => show_window(app),
            "toggle-recording" => toggle_recording(core),
            "debug" => open_debug(app),
            "github" => open_url(REPO),
            "issues" => open_url(&format!("{REPO}/issues/new")),
            "releases" => open_url(&format!("{REPO}/releases")),
            "about" => {
                message(&format!("EZ Hanseikai\nVersion {VERSION}\nMIT or Apache-2.0\n{REPO}"), "OK");
            }
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
            if last != Some(recording) {
                last = Some(recording);
                let text = if recording { "Stop recording" } else { "Start recording" };
                let _ = tray_toggle.set_text(text);
                let _ = bar_toggle.set_text(text);
                let _ = tray.set_tooltip(Some(if recording { "EZ Hanseikai - recording" } else { "EZ Hanseikai" }));
                let _ = tray.set_icon(Some(if recording { rec_icon.clone() } else { idle_icon.clone() }));
            }
            std::thread::sleep(TRAY_POLL);
        }
    });
    Ok(())
}
