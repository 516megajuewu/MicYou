//! Plugin management commands for the frontend.

use crate::server::ServerState;
use micyou_plugin::PluginSyncTransport;
use serde::Serialize;
use tauri::State;

/// Frontend view of one plugin.
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct PluginView {
    pub id: String,
    pub name: String,
    pub version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub runtime: String,
    pub kind: String,
    pub platforms: Vec<String>,
    pub capabilities: Vec<String>,
    pub enabled: bool,
    pub loaded: bool,
    pub dsp_node: bool,
    /// Load/enable error surfaced to the user (e.g. artifact missing).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Cross-device sync status for the plugins page.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginSyncStatus {
    /// Whether a phone device session is connected.
    pub device_connected: bool,
    /// Plugins can currently reach the remote device.
    pub transport_ready: bool,
}

/// List all installed plugins (registry + load state).
#[tauri::command]
pub fn list_plugins(state: State<'_, ServerState>) -> Result<Vec<PluginView>, String> {
    let plugins = &state.plugins;
    let manager = plugins
        .manager
        .lock()
        .map_err(|_| "plugin manager lock poisoned".to_string())?;
    let dsp_ids = plugins.dsp_registry.plugin_ids();

    let mut views: Vec<PluginView> = manager
        .entries()
        .into_iter()
        .map(|entry| {
            let m = &entry.manifest;
            let id = m.id.clone();
            PluginView {
                dsp_node: dsp_ids.contains(&id),
                loaded: manager.is_loaded(&id),
                enabled: entry.state.is_enabled(),
                error: None,
                id: m.id.clone(),
                name: m.name.clone(),
                version: m.version.clone(),
                author: m.author.clone(),
                description: m.description.clone(),
                runtime: m.runtime.to_string(),
                kind: m.kind.to_string(),
                platforms: m.platforms.clone(),
                capabilities: m.capabilities.clone(),
            }
        })
        .collect();

    // Re-attempt loading enabled-but-failed plugins lazily and report errors.
    let ids: Vec<String> = views
        .iter()
        .filter(|v| v.enabled && !v.loaded)
        .map(|v| v.id.clone())
        .collect();
    drop(manager);
    for id in ids {
        if let Err(e) = plugins.enable_plugin(&id) {
            if let Some(view) = views.iter_mut().find(|v| v.id == id) {
                view.error = Some(e.to_string());
            }
        }
    }
    Ok(views)
}

/// Enable or disable a plugin (loads/unloads the runtime, updates DSP nodes).
#[tauri::command]
pub fn set_plugin_enabled(
    state: State<'_, ServerState>,
    id: String,
    enabled: bool,
) -> Result<(), String> {
    let result = if enabled {
        state.plugins.enable_plugin(&id)
    } else {
        state.plugins.disable_plugin(&id)
    };
    result.map_err(|e| e.to_string())
}

/// Uninstall a plugin (deletes its directory).
#[tauri::command]
pub fn uninstall_plugin(state: State<'_, ServerState>, id: String) -> Result<(), String> {
    state
        .plugins
        .uninstall_plugin(&id)
        .map_err(|e| e.to_string())
}

/// Read a plugin's persisted config.
#[tauri::command]
pub fn get_plugin_config(
    state: State<'_, ServerState>,
    id: String,
) -> Result<serde_json::Value, String> {
    let manager = state
        .plugins
        .manager
        .lock()
        .map_err(|_| "plugin manager lock poisoned".to_string())?;
    let map = manager.plugin_config(&id).map_err(|e| e.to_string())?;
    Ok(serde_json::Value::Object(map))
}

/// Write one plugin config value.
#[tauri::command]
pub fn set_plugin_config(
    state: State<'_, ServerState>,
    id: String,
    key: String,
    value: serde_json::Value,
) -> Result<(), String> {
    let manager = state
        .plugins
        .manager
        .lock()
        .map_err(|_| "plugin manager lock poisoned".to_string())?;
    manager
        .set_plugin_config(&id, &key, value)
        .map_err(|e| e.to_string())
}

/// Recent log lines emitted by a plugin.
#[tauri::command]
pub fn get_plugin_logs(
    state: State<'_, ServerState>,
    id: String,
) -> Result<Vec<String>, String> {
    Ok(state.plugins.logs.lines(&id))
}

/// Cross-device plugin sync status.
#[tauri::command]
pub fn get_plugin_sync_status(state: State<'_, ServerState>) -> Result<PluginSyncStatus, String> {
    let connected = state.plugins.sync.is_connected();
    Ok(PluginSyncStatus {
        device_connected: connected,
        transport_ready: connected,
    })
}

/// Open the plugin directory in the system file manager (helper for manual
/// installs: drop a plugin folder / .zip there).
#[tauri::command]
pub fn open_plugins_dir(state: State<'_, ServerState>) -> Result<String, String> {
    let dir = state
        .plugins
        .manager
        .lock()
        .map_err(|_| "plugin manager lock poisoned".to_string())?
        .plugins_dir()
        .to_path_buf();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir.display().to_string())
}
