//! Host API: the services the host exposes to plugins.
//!
//! The trait here is the *logical* contract. Both runtimes translate it:
//! - Native plugins receive a C ABI function table (see `native.rs`).
//! - WASM plugins receive host functions registered in the linker (see `wasm.rs`).
//!
//! Keeping one logical contract means a plugin written against it behaves the
//! same on desktop and (later) on Android.

use crate::error::PluginResult;
use serde::{Deserialize, Serialize};

/// Log levels a plugin can emit through the host.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PluginLogLevel {
    Error,
    Warn,
    Info,
    Debug,
    Trace,
}

/// Snapshot of the live audio stream state, returned by `HostApi::audio_state`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioStateSnapshot {
    /// Whether a transport session is currently streaming audio.
    pub streaming: bool,
    /// Input sample rate in Hz (0 when idle).
    pub sample_rate: u32,
    /// Channel count of the incoming stream.
    pub channels: u32,
    /// Raw input level (RMS, 0..1).
    pub input_level: f32,
    /// Level after the DSP chain (RMS, 0..1).
    pub processed_level: f32,
    /// Output queue latency in milliseconds.
    pub queued_ms: f64,
    /// Whether the mute button is engaged.
    pub muted: bool,
}

/// Snapshot of a connected device (phone / web client).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceSnapshot {
    /// Connection mode: wifi | usb | web.
    pub mode: String,
    /// Peer address / device label.
    pub label: String,
    /// Whether the device audio session is active.
    pub audio_active: bool,
}

/// Target of a cross-device plugin message.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MessageTarget {
    /// A plugin on the same host.
    Local { plugin_id: String },
    /// A plugin on the connected remote device.
    Remote { plugin_id: String },
    /// Broadcast to all hosts (local + remote) subscribed to the topic.
    Broadcast,
}

/// The services plugins can call. Implemented by the host and handed to each
/// plugin instance; methods must be cheap and never block the real-time audio
/// thread unless explicitly documented.
pub trait HostApi: Send + Sync {
    /// Emit a structured log line attributed to the plugin.
    fn log(&self, level: PluginLogLevel, message: &str);

    /// Read a plugin-scoped configuration value (merged defaults + overrides).
    fn get_config(&self, key: &str) -> Option<serde_json::Value>;

    /// Write a plugin-scoped configuration value (persisted by the host).
    fn set_config(&self, key: &str, value: serde_json::Value) -> PluginResult<()>;

    /// Publish an event on the plugin bus; local subscribers receive it.
    fn emit_event(&self, topic: &str, payload: serde_json::Value) -> PluginResult<()>;

    /// Send a binary message to a local or remote plugin.
    fn send_message(&self, target: MessageTarget, payload: Vec<u8>) -> PluginResult<()>;

    /// Live audio stream state (requires `audio.state` capability).
    fn audio_state(&self) -> AudioStateSnapshot;

    /// Play a WAV file through the host audio output (requires `audio.play`
    /// capability). Returns once the file is queued; playback is asynchronous
    /// on a host-owned thread and never real-time safe.
    fn play_sound(&self, path: &str) -> PluginResult<()>;

    /// Connected devices (requires `device.list` capability).
    fn connected_devices(&self) -> Vec<DeviceSnapshot>;
}
