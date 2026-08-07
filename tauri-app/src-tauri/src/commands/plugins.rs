//! Plugin management commands for the frontend.

use crate::server::ServerState;
use micyou_plugin::PluginSyncTransport;
use micyou_plugin::manifest::UiDescriptor;
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ui: Option<UiDescriptor>,
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
                ui: m.ui.clone(),
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
pub fn get_plugin_logs(state: State<'_, ServerState>, id: String) -> Result<Vec<String>, String> {
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

/// Read a plugin-authored settings page (self-contained HTML file inside
/// the plugin directory, rendered by the frontend in a sandboxed iframe).
#[tauri::command]
pub fn get_plugin_panel(
    state: State<'_, ServerState>,
    pluginId: String,
    panelId: String,
) -> Result<String, String> {
    let manager = state
        .plugins
        .manager
        .lock()
        .map_err(|_| "plugin manager lock poisoned".to_string())?;
    let entry = manager
        .entry(&pluginId)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("unknown plugin {pluginId}"))?;
    let panel = entry
        .manifest
        .ui
        .as_ref()
        .and_then(|u| u.panels.iter().find(|p| p.id == panelId))
        .ok_or_else(|| format!("unknown panel {panelId}"))?;
    let path = entry.dir.join(&panel.entry);
    std::fs::read_to_string(&path)
        .map_err(|e| format!("read {}: {e}", path.display()))
}

/// Deliver a UI action to a plugin instance (soundpad buttons etc).
/// The plugin receives `{ action, payload }` through its message entry.
#[tauri::command]
pub fn plugin_trigger(
    state: State<'_, ServerState>,
    pluginId: String,
    action: String,
    payload: Option<String>,
) -> Result<(), String> {
    let bytes = payload.unwrap_or_default().into_bytes();
    state
        .plugins
        .trigger(&pluginId, &action, &bytes)
        .map_err(|e| e.to_string())
}

/// Import a plugin from a `.zip` file or a plugin directory.
///
/// The source manifest is validated first; the payload is then copied into
/// the plugins dir under the plugin id. Returns the imported plugin id.
#[tauri::command]
pub fn import_plugin(state: State<'_, ServerState>, source: String) -> Result<String, String> {
    let src = std::path::PathBuf::from(source);
    if !src.exists() {
        return Err(format!("source not found: {}", src.display()));
    }
    let plugins_dir = state
        .plugins
        .manager
        .lock()
        .map_err(|_| "plugin manager lock poisoned".to_string())?
        .plugins_dir()
        .to_path_buf();
    std::fs::create_dir_all(&plugins_dir).map_err(|e| e.to_string())?;

    let id = if src.is_dir() {
        import_plugin_dir(&src, &plugins_dir)
    } else if src
        .extension()
        .map(|e| e.eq_ignore_ascii_case("zip"))
        .unwrap_or(false)
    {
        import_plugin_zip(&src, &plugins_dir)
    } else {
        return Err("unsupported source: expected a directory or a .zip file".into());
    }
    .map_err(|e| e.to_string())?;

    // Register the new entry so it appears immediately without a rescan.
    let mut manager = state
        .plugins
        .manager
        .lock()
        .map_err(|_| "plugin manager lock poisoned".to_string())?;
    manager
        .discover_plugin(plugins_dir.join(&id))
        .map_err(|e| e.to_string())?;
    Ok(id)
}

/// Copy a plugin directory (validated) into the plugins dir.
fn import_plugin_dir(src: &std::path::Path, dest_root: &std::path::Path) -> Result<String, String> {
    let manifest = micyou_plugin::PluginManifest::load_from_dir(src)
        .map_err(|e| format!("invalid plugin: {e}"))?;
    let id = manifest.id.clone();
    let dest = dest_root.join(&id);
    if dest.exists() {
        return Err(format!("plugin {id} already installed"));
    }
    copy_dir_recursive(src, &dest).map_err(|e| format!("copy failed: {e}"))?;
    Ok(id)
}

/// Import a `.zip` plugin: peek the manifest for validation + id, then extract
/// with path-traversal protection into `dest_root/<id>/`.
fn import_plugin_zip(
    zip_path: &std::path::Path,
    dest_root: &std::path::Path,
) -> Result<String, String> {
    let file = std::fs::File::open(zip_path).map_err(|e| format!("open zip: {e}"))?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| format!("read zip: {e}"))?;

    // Locate plugin.json (may live in a nested folder) and validate it first.
    let mut manifest_name: Option<String> = None;
    for i in 0..archive.len() {
        let name = archive
            .by_index(i)
            .map_err(|e| format!("zip entry: {e}"))?
            .name()
            .to_string();
        if name == "plugin.json" || name.ends_with("/plugin.json") {
            manifest_name = Some(name);
            break;
        }
    }
    let manifest_name = manifest_name.ok_or("zip contains no plugin.json")?;
    let manifest_text = {
        let mut entry = archive
            .by_name(&manifest_name)
            .map_err(|e| format!("read manifest: {e}"))?;
        let mut text = String::new();
        std::io::Read::read_to_string(&mut entry, &mut text)
            .map_err(|e| format!("read manifest: {e}"))?;
        text
    };
    let manifest = micyou_plugin::PluginManifest::from_json(&manifest_text)
        .map_err(|e| format!("invalid plugin: {e}"))?;
    let id = manifest.id.clone();
    let dest = dest_root.join(&id);
    if dest.exists() {
        return Err(format!("plugin {id} already installed"));
    }
    std::fs::create_dir_all(&dest).map_err(|e| format!("create dir: {e}"))?;

    // Strip the folder prefix that contains plugin.json (e.g. "my-plugin/")
    let prefix = std::path::Path::new(&manifest_name)
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_default();

    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(|e| format!("zip entry: {e}"))?;
        // `enclosed_name` rejects absolute paths and `..` traversal
        let Some(rel) = entry.enclosed_name() else {
            continue;
        };
        let rel = if rel.starts_with(&prefix) {
            rel.strip_prefix(&prefix).unwrap_or(&rel).to_path_buf()
        } else {
            rel
        };
        let target = dest.join(&rel);
        if entry.is_dir() {
            std::fs::create_dir_all(&target).map_err(|e| format!("mkdir: {e}"))?;
        } else {
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent).map_err(|e| format!("mkdir: {e}"))?;
            }
            let mut out =
                std::fs::File::create(&target).map_err(|e| format!("create file: {e}"))?;
            std::io::copy(&mut entry, &mut out).map_err(|e| format!("extract: {e}"))?;
        }
    }
    Ok(id)
}

/// Recursive directory copy (no symlink following).
fn copy_dir_recursive(src: &std::path::Path, dest: &std::path::Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dest)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let from = entry.path();
        let to = dest.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir_recursive(&from, &to)?;
        } else {
            std::fs::copy(&from, &to)?;
        }
    }
    Ok(())
}
