mod auth;
mod background;
mod clipboard;
mod device;
mod preview;
mod relay;
#[cfg(test)]
mod relay_e2e;
mod settings;
mod sync;
mod tunnel;

use tauri::{Manager, RunEvent};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // TLS for relays and the account service uses rustls with the `ring` provider (see
    // Cargo.toml). The HTTP client requires it to be installed before it is built.
    let _ = rustls::crypto::ring::default_provider().install_default();

    let app = tauri::Builder::default()
        .plugin(background::single_instance_plugin())
        .plugin(background::autostart_plugin())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let device = device::load_or_create(app.handle());
            let sync = sync::SyncState::start(app.handle(), device);
            let clipboard = clipboard::ClipboardState::start(
                app.handle().clone(),
                std::sync::Arc::clone(&sync.engine),
                sync.node().cloned(),
            );
            app.manage(clipboard);
            let settings = settings::SettingsState::load(app.handle());
            let auth = auth::AuthState::load();
            let relay = relay::RelayState::start(
                app.handle(),
                device,
                sync::device_name(),
                &settings.current(),
                std::sync::Arc::clone(&auth),
                sync.node().cloned(),
            );
            app.manage(settings);
            app.manage(auth);
            app.manage(relay);
            app.manage(sync);
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
            sync::sync_status,
            sync::start_pairing,
            sync::cancel_pairing,
            sync::pairable_devices,
            sync::pair_with,
            sync::remove_peer,
            settings::get_settings,
            settings::set_connection_mode,
            settings::set_relay_url,
            auth::auth_status,
            auth::sign_in,
            auth::cancel_sign_in,
            auth::sign_out,
            relay::relay_status,
            relay::send_relay_test,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application");

    app.run(|app, event| {
        if let RunEvent::Exit = event {
            clipboard::keep_content_after_exit(app);
        }
    });
}
