//! Running in the background: tray icon, close-to-tray, single instance, start on login.

use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{App, AppHandle, Manager, Runtime, Window, WindowEvent};
use tauri_plugin_autostart::{MacosLauncher, ManagerExt};

const MAIN_WINDOW: &str = "main";

/// Passed by the login item so Cled starts in the tray without opening its window.
const HIDDEN_ARG: &str = "--hidden";

/// Must be registered before other plugins so a second launch exits before doing any work.
pub fn single_instance_plugin<R: Runtime>() -> tauri::plugin::TauriPlugin<R> {
    tauri_plugin_single_instance::init(|app, _args, _cwd| show_main_window(app))
}

pub fn autostart_plugin<R: Runtime>() -> tauri::plugin::TauriPlugin<R> {
    tauri_plugin_autostart::init(MacosLauncher::LaunchAgent, Some(vec![HIDDEN_ARG]))
}

pub fn setup(app: &App) -> tauri::Result<()> {
    create_tray(app)?;
    // The window starts hidden (see tauri.conf.json) to avoid a flash when launched at login.
    if !std::env::args().any(|arg| arg == HIDDEN_ARG) {
        show_main_window(app.handle());
    }
    Ok(())
}

/// Closing the window hides it; Cled keeps watching the clipboard until "Quit" is chosen.
pub fn on_window_event(window: &Window, event: &WindowEvent) {
    if let WindowEvent::CloseRequested { api, .. } = event
        && window.label() == MAIN_WINDOW
    {
        api.prevent_close();
        let _ = window.hide();
    }
}

fn create_tray(app: &App) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, "show", "Show Cled", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit Cled", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &quit])?;

    let mut tray = TrayIconBuilder::with_id("main")
        .tooltip("Cled")
        .menu(&menu)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => show_main_window(app),
            "quit" => app.exit(0),
            _ => {}
        });
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.build(app)?;
    Ok(())
}

pub fn show_main_window<R: Runtime>(app: &AppHandle<R>) {
    if let Some(window) = app.get_webview_window(MAIN_WINDOW) {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

#[tauri::command(async)]
pub fn get_autostart(app: AppHandle) -> Result<bool, String> {
    app.autolaunch().is_enabled().map_err(|err| err.to_string())
}

#[tauri::command(async)]
pub fn set_autostart(app: AppHandle, enabled: bool) -> Result<(), String> {
    let autolaunch = app.autolaunch();
    if enabled {
        autolaunch.enable()
    } else {
        autolaunch.disable()
    }
    .map_err(|err| err.to_string())
}

#[tauri::command]
pub fn quit(app: AppHandle) {
    app.exit(0);
}
