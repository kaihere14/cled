//! This installation's device ID, stored in the app's config directory.

use std::fs;

use cled_sync::DeviceId;
use tauri::{AppHandle, Manager};

const FILE_NAME: &str = "device-id";

/// Loads the device ID, creating and saving a new one on first run. If it can't be saved, the
/// app still works with a fresh ID for this run.
pub fn load_or_create(app: &AppHandle) -> DeviceId {
    let Ok(dir) = app.path().app_config_dir() else {
        eprintln!("no config directory; using a temporary device ID");
        return DeviceId::new_random();
    };
    let path = dir.join(FILE_NAME);

    match fs::read_to_string(&path) {
        Ok(text) => match text.parse() {
            Ok(id) => return id,
            Err(err) => eprintln!(
                "invalid device ID in {}: {err}; replacing it",
                path.display()
            ),
        },
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => eprintln!("could not read {}: {err}", path.display()),
    }

    let id = DeviceId::new_random();
    if let Err(err) = fs::create_dir_all(&dir).and_then(|()| fs::write(&path, format!("{id}\n"))) {
        eprintln!("could not save device ID to {}: {err}", path.display());
    }
    id
}
