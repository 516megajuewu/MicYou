//! MicYou plugin framework.
//!
//! Unified plugin abstraction with a dual runtime:
//! - **Native** plugins: platform cdylibs (`.so` / `.dylib` / `.dll`) loaded
//!   through a versioned C ABI, for real-time DSP and deep system integration.
//! - **WASM** plugins: WebAssembly modules executed in a sandboxed interpreter,
//!   for utilities, UI logic and lightweight processing.
//!
//! The manifest, Host API contract and cross-device message protocol are
//! platform-neutral so the same plugin works on the desktop app, the CLI/TUI
//! and (in a future phase) Android with a different loading implementation.

pub mod error;
pub mod host;
pub mod manager;
pub mod manifest;
pub mod plugin;

pub use error::{PluginError, PluginResult};
pub use host::{AudioStateSnapshot, DeviceSnapshot, HostApi, MessageTarget, PluginLogLevel};
pub use manager::{PluginEntry, PluginManager, PluginPersistedState, ScanReport};
pub use manifest::{
    capabilities, platforms, DspDescriptor, PluginManifest, PluginKind, RuntimeKind,
    UiDescriptor, HOST_API_VERSION, KNOWN_CAPABILITIES, MANIFEST_FILE_NAME,
};
pub use plugin::{
    message_topic, require_capability, AudioFrameCtx, PluginEvent, PluginInstance,
    PluginRuntime, PluginState, ProcessStatus,
};
