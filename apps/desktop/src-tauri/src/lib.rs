mod background;
mod clipboard;
mod preview;

use tauri::{Manager, RunEvent};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app = tauri::Builder::default()
        .plugin(background::single_instance_plugin())
        .plugin(background::autostart_plugin())
        .setup(|app| {
            let state = clipboard::ClipboardState::start(app.handle().clone());
            app.manage(state);
            background::setup(app)?;
            Ok(())
        })
        .on_window_event(background::on_window_event)
        .invoke_handler(tauri::generate_handler![
            clipboard::read_clipboard,
            clipboard::write_clipboard,
            clipboard::clipboard_status,
            background::get_autostart,
            background::set_autostart,
            background::quit,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application");

    app.run(|app, event| {
        if let RunEvent::Exit = event {
            clipboard::keep_content_after_exit(app);
        }
    });
}
