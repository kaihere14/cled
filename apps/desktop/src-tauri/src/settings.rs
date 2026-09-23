//! User settings stored in `<config dir>/settings.json`: the connection mode and relay URL.
//! (Who is signed in isn't a setting; see `auth.rs`.)
//!
//! In relay mode the app connects to the relay (see `relay.rs`), but clipboard items don't go
//! through it yet: LAN sync runs as before whichever mode is selected.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, State};

use crate::relay::RelayState;

const FILE_NAME: &str = "settings.json";

/// The local relay started by `pnpm relay` (see `apps/relay/src/config.ts`).
pub const DEFAULT_RELAY_URL: &str = "http://127.0.0.1:8787";

/// Mirrors `Settings` in `src/lib/ipc.ts`. Missing fields take their defaults, so files written
/// by older versions (or no file at all) load without migration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub connection_mode: ConnectionMode,
    pub relay_url: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            connection_mode: ConnectionMode::Lan,
            relay_url: DEFAULT_RELAY_URL.into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ConnectionMode {
    Lan,
    Relay,
}

/// Checks that `input` is an `http://` or `https://` URL with a host, and returns it trimmed.
/// The error is shown to the user as is.
pub fn validate_relay_url(input: &str) -> Result<String, String> {
    let input = input.trim();
    if input.is_empty() {
        return Err("Enter a relay URL.".into());
    }
    let valid = url::Url::parse(input).is_ok_and(|url| {
        matches!(url.scheme(), "http" | "https") && url.host_str().is_some_and(|h| !h.is_empty())
    });
    if valid {
        Ok(input.to_owned())
    } else {
        Err(
            "Enter a full URL starting with http:// or https://, e.g. https://relay.example.com"
                .into(),
        )
    }
}

/// Loads settings from `path`. A missing, unreadable, or invalid file gives the defaults, and an
/// invalid relay URL (e.g. edited by hand) is replaced by the default.
fn load(path: &Path) -> Settings {
    let mut settings = match fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|err| {
            eprintln!(
                "invalid settings in {}: {err}; using defaults",
                path.display()
            );
            Settings::default()
        }),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Settings::default(),
        Err(err) => {
            eprintln!("could not read {}: {err}; using defaults", path.display());
            Settings::default()
        }
    };
    if validate_relay_url(&settings.relay_url).is_err() {
        settings.relay_url = DEFAULT_RELAY_URL.into();
    }
    settings
}

/// Writes atomically (temp file, then rename), so a crash never leaves a half-written file.
fn save(path: &Path, settings: &Settings) -> std::io::Result<()> {
    let json = serde_json::to_vec_pretty(settings).map_err(std::io::Error::other)?;
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, json)?;
    fs::rename(&tmp, path)
}

pub struct SettingsState {
    /// `None` without a config directory: settings then last only for this run.
    path: Option<PathBuf>,
    settings: Mutex<Settings>,
}

impl SettingsState {
    pub fn load(app: &AppHandle) -> Self {
        let path = match app.path().app_config_dir() {
            Ok(dir) => Some(dir.join(FILE_NAME)),
            Err(err) => {
                eprintln!("no config directory ({err}); settings won't be saved");
                None
            }
        };
        let settings = path.as_deref().map(load).unwrap_or_default();
        Self {
            path,
            settings: Mutex::new(settings),
        }
    }

    pub fn current(&self) -> Settings {
        self.lock().clone()
    }

    fn lock(&self) -> MutexGuard<'_, Settings> {
        self.settings
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Applies `change` and saves. If saving fails, nothing changes, in memory or on disk.
    fn update(&self, change: impl FnOnce(&mut Settings)) -> Result<Settings, String> {
        let mut current = self.lock();
        let mut next = current.clone();
        change(&mut next);
        if let Some(path) = &self.path
            && let Err(err) = save(path, &next)
        {
            eprintln!("could not save settings to {}: {err}", path.display());
            return Err("Couldn't save settings.".into());
        }
        *current = next.clone();
        Ok(next)
    }
}

#[tauri::command(async)]
pub fn get_settings(state: State<'_, SettingsState>) -> Settings {
    state.lock().clone()
}

// Each setter below also tells the relay connection, which connects, disconnects, or reconnects
// to match the saved settings.

#[tauri::command(async)]
pub fn set_connection_mode(
    state: State<'_, SettingsState>,
    relay: State<'_, RelayState>,
    mode: ConnectionMode,
) -> Result<Settings, String> {
    let settings = state.update(|settings| settings.connection_mode = mode)?;
    relay.configure(&settings);
    Ok(settings)
}

/// Validates and saves the relay URL. An invalid URL is rejected and the saved one kept.
#[tauri::command(async)]
pub fn set_relay_url(
    state: State<'_, SettingsState>,
    relay: State<'_, RelayState>,
    url: String,
) -> Result<Settings, String> {
    let url = validate_relay_url(&url)?;
    let settings = state.update(|settings| settings.relay_url = url)?;
    relay.configure(&settings);
    Ok(settings)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state_in(dir: &Path) -> SettingsState {
        let path = dir.join(FILE_NAME);
        SettingsState {
            settings: Mutex::new(load(&path)),
            path: Some(path),
        }
    }

    #[test]
    fn defaults_to_lan_and_local_relay() {
        let dir = tempfile::tempdir().unwrap();
        let settings = state_in(dir.path()).lock().clone();
        assert_eq!(settings.connection_mode, ConnectionMode::Lan);
        assert_eq!(settings.relay_url, DEFAULT_RELAY_URL);
    }

    #[test]
    fn default_relay_url_is_valid() {
        assert_eq!(
            validate_relay_url(DEFAULT_RELAY_URL).unwrap(),
            DEFAULT_RELAY_URL
        );
    }

    #[test]
    fn accepts_http_and_https_urls() {
        for url in [
            "http://localhost:8787",
            "http://192.168.1.10:8787",
            "https://relay.example.com",
            "http://[::1]:8787",
        ] {
            assert_eq!(validate_relay_url(url).unwrap(), url);
        }
        assert_eq!(
            validate_relay_url("  https://relay.example.com \n").unwrap(),
            "https://relay.example.com"
        );
    }

    #[test]
    fn rejects_everything_else() {
        for url in [
            "",
            "   ",
            "relay.example.com",
            "localhost:8787",
            "not a url",
            "ftp://relay.example.com",
            "ws://relay.example.com",
            "file:///etc/passwd",
            "http://",
        ] {
            assert!(validate_relay_url(url).is_err(), "accepted {url:?}");
        }
    }

    #[test]
    fn switches_modes_and_restores_them_after_restart() {
        let dir = tempfile::tempdir().unwrap();
        let state = state_in(dir.path());

        state
            .update(|s| s.connection_mode = ConnectionMode::Relay)
            .unwrap();
        assert_eq!(
            state_in(dir.path()).lock().connection_mode,
            ConnectionMode::Relay
        );

        state
            .update(|s| s.connection_mode = ConnectionMode::Lan)
            .unwrap();
        assert_eq!(
            state_in(dir.path()).lock().connection_mode,
            ConnectionMode::Lan
        );
    }

    #[test]
    fn saved_relay_url_is_restored_after_restart() {
        let dir = tempfile::tempdir().unwrap();
        let url = validate_relay_url("https://relay.example.com").unwrap();
        state_in(dir.path()).update(|s| s.relay_url = url).unwrap();

        assert_eq!(
            state_in(dir.path()).lock().relay_url,
            "https://relay.example.com"
        );
    }

    #[test]
    fn older_or_damaged_files_fall_back_to_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE_NAME);

        // A file from a version without these fields.
        fs::write(&path, "{}").unwrap();
        assert_eq!(load(&path), Settings::default());

        fs::write(&path, "not json").unwrap();
        assert_eq!(load(&path), Settings::default());

        fs::write(&path, r#"{"connectionMode":"relay","relayUrl":"nonsense"}"#).unwrap();
        let settings = load(&path);
        assert_eq!(settings.connection_mode, ConnectionMode::Relay);
        assert_eq!(settings.relay_url, DEFAULT_RELAY_URL);

        // From the version that had a manually entered relay user ID: that setting is dropped.
        fs::write(
            &path,
            r#"{"connectionMode":"relay","relayUserId":"someone"}"#,
        )
        .unwrap();
        let settings = load(&path);
        assert_eq!(settings.connection_mode, ConnectionMode::Relay);
        assert_eq!(
            serde_json::to_value(&settings).unwrap().get("relayUserId"),
            None
        );
    }

    #[test]
    fn file_uses_the_documented_field_names() {
        let json = serde_json::to_value(Settings::default()).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "connectionMode": "lan",
                "relayUrl": DEFAULT_RELAY_URL,
            })
        );
    }
}
