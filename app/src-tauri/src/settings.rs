//! User settings, persisted as JSON in the OS config directory.
//!
//! Scope is deliberately small: an OpenRouter API key and the chosen
//! model. Everything else in the app is either derived from the scan or
//! remembered per-view in `localStorage`.
//!
//! **The API key is stored in plaintext.** That is a deliberate choice
//! for a local-only tool — anything running as this user can read the
//! file, which is the same trust boundary as the scan itself. If that
//! ever stops being acceptable, the swap is contained: the OS keychain
//! (Windows Credential Manager via the `keyring` crate) would replace the
//! body of `load`/`store` and nothing else in the app would change, since
//! the key never leaves this module except through the OpenRouter client.
//!
//! The key never reaches the webview either: `SettingsDto` reports
//! *whether* a key is set, never what it is.

use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tauri::Manager;

const FILE_NAME: &str = "settings.json";

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// OpenRouter key. Empty means "not configured".
    pub openrouter_api_key: String,
    /// Model slug, e.g. `qwen/qwen-2.5-72b-instruct:free`.
    pub model: String,
    /// Whether the chosen model can call tools. Cached from the model
    /// list so a scan-time decision doesn't need a network round trip.
    pub model_supports_tools: bool,
}

impl Settings {
    pub fn is_configured(&self) -> bool {
        !self.openrouter_api_key.trim().is_empty() && !self.model.trim().is_empty()
    }
}

fn path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_config_dir()
        .map_err(|e| format!("no config directory available: {e}"))?;
    Ok(dir.join(FILE_NAME))
}

/// Read settings, treating any problem as "not configured yet".
///
/// A missing file is the normal first-run case, and a corrupt one is not
/// worth blocking the whole app over — the user can just re-enter the key.
pub fn load(app: &tauri::AppHandle) -> Settings {
    let Ok(file) = path(app) else {
        return Settings::default();
    };
    let Ok(text) = fs::read_to_string(&file) else {
        return Settings::default();
    };
    serde_json::from_str(&text).unwrap_or_default()
}

pub fn store(app: &tauri::AppHandle, settings: &Settings) -> Result<(), String> {
    let file = path(app)?;
    if let Some(dir) = file.parent() {
        fs::create_dir_all(dir).map_err(|e| format!("could not create {}: {e}", dir.display()))?;
    }
    let text = serde_json::to_string_pretty(settings)
        .map_err(|e| format!("could not serialize settings: {e}"))?;
    fs::write(&file, text).map_err(|e| format!("could not write {}: {e}", file.display()))?;
    Ok(())
}

/// What the frontend is allowed to know. Note the absence of the key
/// itself — the UI shows whether one is set and lets you replace it, but
/// cannot read it back out.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsDto {
    pub has_api_key: bool,
    pub model: String,
    pub model_supports_tools: bool,
    /// Shown on the settings screen so it's obvious where the key lives.
    pub config_path: String,
}

impl SettingsDto {
    pub fn from(settings: &Settings, config_path: String) -> Self {
        Self {
            has_api_key: !settings.openrouter_api_key.trim().is_empty(),
            model: settings.model.clone(),
            model_supports_tools: settings.model_supports_tools,
            config_path,
        }
    }
}

pub fn config_path_string(app: &tauri::AppHandle) -> String {
    path(app)
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "(unavailable)".into())
}
