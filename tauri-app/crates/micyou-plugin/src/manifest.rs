//! Plugin manifest: a unified, platform-independent description shared by the
//! desktop (Tauri), CLI/TUI and the future Android runtime.
//!
//! A plugin directory contains a `plugin.json` plus the entry artifact
//! (native cdylib or WASM module). The manifest is the single source of truth
//! for identity, runtime type, capabilities, DSP wiring and UI registration.

use crate::error::{PluginError, PluginResult};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::{Path, PathBuf};

/// Host API version this plugin system speaks. Plugins declare the version
/// they were built against; the host rejects incompatible ones.
pub const HOST_API_VERSION: u32 = 1;

/// Plugin directory layout: the manifest file name.
pub const MANIFEST_FILE_NAME: &str = "plugin.json";

/// Reverse-DNS plugin id (e.g. `dev.micyou.eq`). Allowed charset:
/// lowercase alphanumerics plus `.` and `-`, at least one dot.
pub fn validate_plugin_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id.contains('.')
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '.' || c == '-')
}

/// Runtime type. `Native` loads a platform cdylib (`.so` / `.dylib` / `.dll`),
/// `Wasm` loads a WebAssembly module into the sandboxed interpreter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RuntimeKind {
    Native,
    Wasm,
}

impl fmt::Display for RuntimeKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RuntimeKind::Native => write!(f, "native"),
            RuntimeKind::Wasm => write!(f, "wasm"),
        }
    }
}

/// Functional category of a plugin, used by the host to decide lifecycle and
/// scheduling policy (e.g. real-time DSP plugins never block the audio thread).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginKind {
    /// Background logic / automation / networking.
    #[default]
    Utility,
    /// Real-time audio processor inserted into the DSP chain.
    Dsp,
    /// Provides a frontend configuration panel.
    Ui,
    /// Dedicated to cross-device state synchronization.
    Bridge,
}

impl fmt::Display for PluginKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PluginKind::Dsp => write!(f, "dsp"),
            PluginKind::Utility => write!(f, "utility"),
            PluginKind::Ui => write!(f, "ui"),
            PluginKind::Bridge => write!(f, "bridge"),
        }
    }
}

/// Capability identifiers the host understands. Plugins must declare the
/// capabilities they need; the host grants them after policy checks.
pub mod capabilities {
    /// Insert a processing node into the real-time DSP chain.
    pub const DSP_NODE: &str = "dsp.node";
    /// Read host configuration (settings.json etc).
    pub const CONFIG_READ: &str = "config.read";
    /// Write host configuration.
    pub const CONFIG_WRITE: &str = "config.write";
    /// Emit events on the plugin bus (broadcast / subscribe model).
    pub const EVENT_EMIT: &str = "event.emit";
    /// Send messages to other plugins or to a remote device plugin.
    pub const MESSAGE_SEND: &str = "message.send";
    /// Query live audio stream state (levels, format, latency).
    pub const AUDIO_STATE: &str = "audio.state";
    /// Play an audio file (wav) through the host output device.
    pub const AUDIO_PLAY: &str = "audio.play";
    /// Enumerate connected devices (phones, web clients).
    pub const DEVICE_LIST: &str = "device.list";
    /// Open outbound network connections.
    pub const NETWORK_IO: &str = "network.io";
    /// Read plugin-local files (already inside the plugin sandbox).
    pub const FS_READ: &str = "fs.read";
}

/// All capability identifiers the host currently recognizes.
pub const KNOWN_CAPABILITIES: &[&str] = &[
    capabilities::DSP_NODE,
    capabilities::CONFIG_READ,
    capabilities::CONFIG_WRITE,
    capabilities::EVENT_EMIT,
    capabilities::MESSAGE_SEND,
    capabilities::AUDIO_STATE,
    capabilities::AUDIO_PLAY,
    capabilities::DEVICE_LIST,
    capabilities::NETWORK_IO,
    capabilities::FS_READ,
];

/// Native platform tags used in `PluginManifest.platforms`.
pub mod platforms {
    pub const WINDOWS: &str = "windows";
    pub const LINUX: &str = "linux";
    pub const MACOS: &str = "macos";
    pub const ANDROID: &str = "android";
}

fn default_api_version() -> u32 {
    HOST_API_VERSION
}

/// Optional UI registration: a Vue panel the frontend can lazy-load.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UiDescriptor {
    /// Frontend route / component identifier (e.g. `plugin-panel`).
    pub route: String,
    /// Display name of the panel.
    pub label: String,
    /// Relative path to a bundled JS entry (advanced; default: generic form).
    #[serde(default)]
    pub entry: Option<String>,
}

/// Optional DSP registration: where the node is inserted in the chain.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DspDescriptor {
    /// Insert before or after a built-in node name (e.g. `Equalizer`).
    #[serde(default)]
    pub insert_after: Option<String>,
    /// Insert at the head of the chain (before AEC) when true.
    #[serde(default)]
    pub first: bool,
    /// Preferred processing block size in samples (native plugins only;
    /// the host may fall back to its own frame size).
    #[serde(default)]
    pub frame_size: Option<usize>,
    /// DSP plugins are granted an additional real-time slot check.
    #[serde(default)]
    pub realtime_safe: bool,
}

/// The unified plugin manifest. Field names use camelCase to mirror the
/// wire/protocol style used across MicYou.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginManifest {
    /// Reverse-DNS id, e.g. `dev.micyou.eq`.
    pub id: String,
    /// Human-readable name.
    pub name: String,
    /// Semver version.
    pub version: String,
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    /// native | wasm
    pub runtime: RuntimeKind,
    /// File name of the entry artifact relative to the plugin directory.
    pub entry: String,
    /// Supported platforms; empty means all. Tags: linux, windows, macos, android.
    #[serde(default)]
    pub platforms: Vec<String>,
    /// Host API version this plugin was built against.
    #[serde(default = "default_api_version")]
    pub api_version: u32,
    /// Capability identifiers requested from the host.
    #[serde(default)]
    pub capabilities: Vec<String>,
    /// Functional category.
    #[serde(default)]
    pub kind: PluginKind,
    #[serde(default)]
    pub ui: Option<UiDescriptor>,
    #[serde(default)]
    pub dsp: Option<DspDescriptor>,
    /// Default configuration (merged into plugin state on first enable).
    #[serde(default)]
    pub config: Option<serde_json::Value>,
}

impl PluginManifest {
    /// Parse + validate a manifest from raw JSON text.
    pub fn from_json(text: &str) -> PluginResult<Self> {
        let manifest: Self =
            serde_json::from_str(text).map_err(|e| PluginError::InvalidManifest(e.to_string()))?;
        manifest.validate()?;
        Ok(manifest)
    }

    /// Load + validate a manifest from `<plugin_dir>/plugin.json`.
    pub fn load_from_dir(dir: &Path) -> PluginResult<Self> {
        let path = dir.join(MANIFEST_FILE_NAME);
        let text = std::fs::read_to_string(&path)
            .map_err(|e| PluginError::InvalidManifest(format!("{}: {e}", path.display())))?;
        Self::from_json(&text)
    }

    /// Semantic validation of a parsed manifest.
    pub fn validate(&self) -> PluginResult<()> {
        if !validate_plugin_id(&self.id) {
            return Err(PluginError::Validation(format!(
                "invalid plugin id {:?}: expect reverse-DNS lowercase alphanumeric with a dot",
                self.id
            )));
        }
        if self.name.is_empty() {
            return Err(PluginError::Validation("name must not be empty".into()));
        }
        semver::Version::parse(&self.version).map_err(|e| {
            PluginError::Validation(format!("invalid semver version {:?}: {e}", self.version))
        })?;
        if self.entry.is_empty() {
            return Err(PluginError::Validation("entry must not be empty".into()));
        }
        if self.api_version != HOST_API_VERSION {
            return Err(PluginError::ApiVersionMismatch {
                plugin: self.api_version,
                host: HOST_API_VERSION,
            });
        }
        for cap in &self.capabilities {
            if !KNOWN_CAPABILITIES.contains(&cap.as_str()) {
                return Err(PluginError::Validation(format!(
                    "unknown capability {:?}",
                    cap
                )));
            }
        }
        if self.kind == PluginKind::Dsp && self.runtime == RuntimeKind::Wasm {
            // WASM DSP nodes are allowed but must declare realtime_safe, and the
            // host treats them as best-effort (interpreter latency is not
            // guaranteed real-time safe).
            if let Some(dsp) = &self.dsp {
                if dsp.realtime_safe {
                    return Err(PluginError::Validation(
                        "wasm dsp plugin must not claim realtime_safe; interpreter execution cannot guarantee real-time safety".into(),
                    ));
                }
            }
        }
        if self.kind == PluginKind::Ui && self.ui.is_none() {
            return Err(PluginError::Validation(
                "ui plugin must declare a ui descriptor".into(),
            ));
        }
        Ok(())
    }

    /// Entry artifact path resolved against the plugin directory.
    pub fn entry_path(&self, plugin_dir: &Path) -> PathBuf {
        plugin_dir.join(&self.entry)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD_JSON: &str = r#"{
        "id": "dev.micyou.eq",
        "name": "Bass Boost",
        "version": "1.2.0",
        "author": "LanRhyme",
        "description": "10-band bass EQ",
        "runtime": "native",
        "entry": "libmicyou_eq.so",
        "platforms": ["linux", "windows", "macos"],
        "apiVersion": 1,
        "capabilities": ["dsp.node", "config.read"],
        "kind": "dsp",
        "dsp": { "insertAfter": "Equalizer", "realtimeSafe": true }
    }"#;

    #[test]
    fn parses_valid_manifest() {
        let manifest = PluginManifest::from_json(GOOD_JSON).unwrap();
        assert_eq!(manifest.id, "dev.micyou.eq");
        assert_eq!(manifest.runtime, RuntimeKind::Native);
        assert_eq!(manifest.kind, PluginKind::Dsp);
        assert_eq!(manifest.api_version, HOST_API_VERSION);
        assert_eq!(manifest.capabilities, vec!["dsp.node", "config.read"]);
        assert_eq!(
            manifest.dsp.as_ref().unwrap().insert_after.as_deref(),
            Some("Equalizer")
        );
        assert!(manifest.dsp.as_ref().unwrap().realtime_safe);
    }

    #[test]
    fn defaults_apply_when_fields_missing() {
        let json = r#"{
            "id": "dev.micyou.util",
            "name": "Logger",
            "version": "0.1.0",
            "runtime": "wasm",
            "entry": "logger.wasm"
        }"#;
        let manifest = PluginManifest::from_json(json).unwrap();
        assert_eq!(manifest.api_version, HOST_API_VERSION);
        assert!(manifest.platforms.is_empty());
        assert!(manifest.capabilities.is_empty());
        assert_eq!(manifest.kind, PluginKind::Utility);
        assert!(manifest.ui.is_none());
        assert!(manifest.dsp.is_none());
    }

    #[test]
    fn rejects_invalid_plugin_id() {
        for bad in ["no-dot", "Uppercase.Id", "a/b", "", "sp ace", "中文.id"] {
            let json = format!(
                r#"{{"id":"{bad}","name":"x","version":"1.0.0","runtime":"wasm","entry":"x.wasm"}}"#
            );
            let result = PluginManifest::from_json(&json);
            assert!(result.is_err(), "id {bad:?} should be rejected");
        }
    }

    #[test]
    fn rejects_bad_semver() {
        let json = r#"{"id":"a.b","name":"x","version":"not-a-version","runtime":"wasm","entry":"x.wasm"}"#;
        assert!(PluginManifest::from_json(json).is_err());
    }

    #[test]
    fn rejects_api_version_mismatch() {
        let json = r#"{"id":"a.b","name":"x","version":"1.0.0","runtime":"wasm","entry":"x.wasm","apiVersion":99}"#;
        let result = PluginManifest::from_json(json).unwrap_err();
        assert!(matches!(
            result,
            PluginError::ApiVersionMismatch {
                plugin: 99,
                host: 1
            }
        ));
    }

    #[test]
    fn rejects_unknown_capability() {
        let json = r#"{"id":"a.b","name":"x","version":"1.0.0","runtime":"wasm","entry":"x.wasm","capabilities":["root"]}"#;
        let result = PluginManifest::from_json(json).unwrap_err();
        assert!(matches!(result, PluginError::Validation(_)));
    }

    #[test]
    fn rejects_wasm_dsp_claiming_realtime_safe() {
        let json = r#"{
            "id":"a.b.dsp","name":"x","version":"1.0.0","runtime":"wasm","entry":"x.wasm",
            "kind":"dsp","dsp":{"realtimeSafe":true}
        }"#;
        let result = PluginManifest::from_json(json).unwrap_err();
        assert!(matches!(result, PluginError::Validation(_)));
    }

    #[test]
    fn rejects_ui_plugin_without_ui_descriptor() {
        let json = r#"{"id":"a.b.ui","name":"x","version":"1.0.0","runtime":"wasm","entry":"x.wasm","kind":"ui"}"#;
        let result = PluginManifest::from_json(json).unwrap_err();
        assert!(matches!(result, PluginError::Validation(_)));
    }

    #[test]
    fn entry_path_resolves_relative_to_dir() {
        let manifest = PluginManifest::from_json(GOOD_JSON).unwrap();
        assert_eq!(
            manifest.entry_path(Path::new("/opt/micyou/plugins/dev.micyou.eq")),
            PathBuf::from("/opt/micyou/plugins/dev.micyou.eq/libmicyou_eq.so")
        );
    }

    #[test]
    fn serialize_roundtrip_preserves_camel_case() {
        let manifest = PluginManifest::from_json(GOOD_JSON).unwrap();
        let json = serde_json::to_value(&manifest).unwrap();
        assert!(json.get("apiVersion").is_some());
        assert!(json.get("insertAfter").is_none()); // nested struct not flattened
        assert_eq!(json["kind"], "dsp");
    }
}
