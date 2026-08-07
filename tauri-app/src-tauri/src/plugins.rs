//! Plugin host wiring: owns the plugin manager, the DSP node registry and the
//! cross-device message bus, shared by the audio thread (via
//! `DspProcessor::set_external_hook`), the TCP server (plugin message relay)
//! and the frontend commands (`commands/plugins.rs`).

use micyou_plugin::bus::{PluginBus, PluginMessage, PluginSyncTransport};
use micyou_plugin::host::{
    AudioStateSnapshot, DeviceSnapshot, HostApi, MessageTarget, PluginLogLevel,
};
use micyou_plugin::manifest::{PluginKind, RuntimeKind};
use micyou_plugin::{PluginError, PluginResult, PluginRuntime};
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, RwLock};

/// TCP control-channel transport for cross-device plugin messages.
/// The tcp_server registers the active client's message sender here; the bus
/// pushes wire messages through it. Only one device session is active at a
/// time in MicYou's model, so a single slot suffices.
pub struct TcpPluginSyncAdapter {
    sender: Mutex<Option<tokio::sync::mpsc::Sender<micyou_protocol::micyou::MessageWrapper>>>,
}

impl TcpPluginSyncAdapter {
    pub fn new() -> Self {
        Self {
            sender: Mutex::new(None),
        }
    }

    /// Register the active client's control sender (or clear on disconnect).
    pub fn set_sender(
        &self,
        sender: Option<tokio::sync::mpsc::Sender<micyou_protocol::micyou::MessageWrapper>>,
    ) {
        if let Ok(mut slot) = self.sender.lock() {
            *slot = sender;
        }
    }

    /// Clear the sender only when it is still ours (avoids nuking a newer
    /// client's slot during a takeover race).
    pub fn clear_if(
        &self,
        tx: &tokio::sync::mpsc::Sender<micyou_protocol::micyou::MessageWrapper>,
    ) {
        if let Ok(mut slot) = self.sender.lock() {
            if slot.as_ref().is_some_and(|s| s.same_channel(tx)) {
                *slot = None;
            }
        }
    }
}

impl Default for TcpPluginSyncAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl PluginSyncTransport for TcpPluginSyncAdapter {
    fn send(&self, msg: &PluginMessage) -> micyou_plugin::PluginResult<()> {
        let slot = self
            .sender
            .lock()
            .map_err(|_| micyou_plugin::PluginError::Runtime("sync sender poisoned".into()))?;
        let Some(tx) = slot.as_ref() else {
            return Err(micyou_plugin::PluginError::MessageDelivery(
                "no device connected".into(),
            ));
        };
        let wire = micyou_plugin::sync::to_wire(msg);
        let wrapper = micyou_protocol::micyou::MessageWrapper {
            audio_packet: None,
            connect: None,
            mute: None,
            ping: None,
            pong: None,
            plugin_message: Some(wire),
        };
        tx.try_send(wrapper)
            .map_err(|e| micyou_plugin::PluginError::MessageDelivery(e.to_string()))?;
        Ok(())
    }

    fn is_connected(&self) -> bool {
        self.sender.lock().map(|g| g.is_some()).unwrap_or(false)
    }
}

/// Runtime plugin host. One instance per process, managed Tauri state.
pub struct PluginHost {
    /// Plugin manager (scan/load/enable). Interior-mutable so the message-bus
    /// dispatcher and the commands can share it.
    pub manager: Arc<Mutex<micyou_plugin::PluginManager>>,
    pub dsp_registry: Arc<micyou_plugin::PluginDspRegistry>,
    pub sync: Arc<TcpPluginSyncAdapter>,
    /// Local + cross-device message bus (RPC / pub-sub).
    pub bus: Arc<PluginBus>,
    /// Bounded per-plugin log buffers (read by the frontend).
    pub logs: Arc<PluginLogs>,
    /// WAV playback for the `audio.play` capability (soundpads etc).
    pub sound: Arc<crate::sound_player::SoundPlayer>,
}

/// Default chain position for the synthetic plugin node: right after AEC,
/// so plugin processing runs on echo-cancelled audio.
pub const PLUGIN_NODE_AFTER: &str = "AEC";

impl PluginHost {
    pub fn new() -> Self {
        let config = crate::app_config::config_dir();
        let manager = Arc::new(Mutex::new(micyou_plugin::PluginManager::new(
            config.join("plugins"),
            config.join("plugin-state.json"),
        )));
        let dsp_registry = Arc::new(micyou_plugin::PluginDspRegistry::new());
        let sync = Arc::new(TcpPluginSyncAdapter::new());

        // Route incoming/request messages to local plugin instances.
        let manager_dispatch = manager.clone();
        let dispatcher: Arc<
            dyn Fn(&PluginMessage) -> micyou_plugin::PluginResult<()> + Send + Sync,
        > = Arc::new(move |msg: &PluginMessage| {
            let manager = manager_dispatch
                .lock()
                .map_err(|_| micyou_plugin::PluginError::Runtime("manager poisoned".into()))?;
            let targets: Vec<String> = if msg.target.is_empty() {
                manager.loaded_ids()
            } else {
                vec![msg.target.clone()]
            };
            for id in targets {
                manager.with_instance(&id, |instance| {
                    instance.handle_message(&msg.source, &msg.topic, &msg.payload)
                })?;
            }
            Ok(())
        });

        let bus = Arc::new(PluginBus::new(sync.clone(), dispatcher));
        let logs = Arc::new(PluginLogs::new());
        let sound = crate::sound_player::SoundPlayer::new();

        Self {
            manager,
            dsp_registry,
            sync,
            bus,
            logs,
            sound,
        }
    }

    /// Deliver a UI-triggered action to a plugin instance as a bus message on
    /// topic `ui:<action>` with the given payload (soundpad buttons etc).
    /// The plugin receives it through its `handle_message` entry.
    pub fn trigger(&self, plugin_id: &str, action: &str, payload: &[u8]) -> PluginResult<()> {
        let msg = PluginMessage::new(
            "ui",
            plugin_id,
            &format!("ui:{action}"),
            payload.to_vec(),
        );
        self.bus.handle_incoming(&msg);
        Ok(())
    }

    /// Load + start one plugin: instantiate the runtime, init it, register the
    /// instance and (for DSP plugins) its processing node.
    pub fn enable_plugin(&self, id: &str) -> PluginResult<()> {
        let entry = {
            let manager = self.manager.lock().map_err(lock_err)?;
            if manager.is_loaded(id) {
                return Ok(()); // already running
            }
            manager
                .entry(id)?
                .ok_or_else(|| PluginError::UnknownPlugin(id.to_string()))?
        };

        let host_api: Arc<dyn HostApi> = PluginHostApi::new(
            self.bus.clone(),
            self.manager.clone(),
            self.logs.clone(),
            self.sound.clone(),
            id.to_string(),
        );
        let mut instance = match entry.manifest.runtime {
            RuntimeKind::Native => micyou_plugin::native::load_native_instance(
                entry.manifest.clone(),
                &entry.dir,
                host_api.clone(),
            )?,
            RuntimeKind::Wasm => micyou_plugin::wasm::load_wasm_instance(
                entry.manifest.clone(),
                &entry.dir,
                host_api.clone(),
            )?,
        };
        instance.init(&*host_api)?;

        let dsp_handle = {
            let mut manager = self.manager.lock().map_err(lock_err)?;
            manager.set_enabled(id, true)?;
            manager.register_instance(instance)?;
            manager.instance_handle(id)?
        };

        if entry.manifest.kind == PluginKind::Dsp {
            let dsp = entry.manifest.dsp.clone().unwrap_or_default();
            let handle = dsp_handle.ok_or_else(|| PluginError::NotLoaded(id.to_string()))?;
            self.dsp_registry.register(micyou_plugin::DspNode {
                plugin_id: id.to_string(),
                first: dsp.first,
                insert_after: dsp.insert_after.clone(),
                instance: handle,
            })?;
        }
        log::info!("[plugins] enabled {id}");
        Ok(())
    }

    /// Stop + unload a plugin (deinit, remove DSP node, persist disabled).
    pub fn disable_plugin(&self, id: &str) -> PluginResult<()> {
        self.dsp_registry.unregister(id)?;
        let mut manager = self.manager.lock().map_err(lock_err)?;
        manager.unregister_instance(id)?;
        manager.set_enabled(id, false)?;
        log::info!("[plugins] disabled {id}");
        Ok(())
    }

    /// Uninstall: disable, remove from registry and delete the directory.
    pub fn uninstall_plugin(&self, id: &str) -> PluginResult<()> {
        self.dsp_registry.unregister(id)?;
        let mut manager = self.manager.lock().map_err(lock_err)?;
        manager.uninstall(id)?;
        log::info!("[plugins] uninstalled {id}");
        Ok(())
    }

    /// Ensure the synthetic `"Plugins"` node exists in the processing chain
    /// (right after AEC) when at least one DSP plugin is registered. This is
    /// an in-memory settings change; the user can reorder it in the GUI like
    /// any other chain node.
    pub fn ensure_plugin_chain_node(
        &self,
        dsp_settings: &Arc<RwLock<micyou_audio::dsp::AudioDspSettings>>,
    ) {
        if !self.dsp_registry.is_active() {
            return;
        }
        if let Ok(mut settings) = dsp_settings.write() {
            let chain = &mut settings.processing_chain;
            if chain
                .iter()
                .any(|n| n == micyou_audio::dsp::PLUGIN_CHAIN_NODE)
            {
                return;
            }
            match chain.iter().position(|n| n == PLUGIN_NODE_AFTER) {
                Some(idx) => {
                    chain.insert(idx + 1, micyou_audio::dsp::PLUGIN_CHAIN_NODE.to_string());
                }
                None => chain.push(micyou_audio::dsp::PLUGIN_CHAIN_NODE.to_string()),
            }
        }
    }

    /// Build the external DSP hook for `DspProcessor`. Cheap no-op when no
    /// DSP plugin is registered (see `PluginDspBridge::hook`).
    pub fn dsp_hook(&self) -> Option<Box<dyn FnMut(&mut Vec<f32>, usize, f64) + Send>> {
        let bridge = micyou_plugin::PluginDspBridge::new(self.dsp_registry.clone());
        Some(bridge.hook())
    }
}

impl Default for PluginHost {
    fn default() -> Self {
        Self::new()
    }
}

fn lock_err<T>(_: std::sync::PoisonError<T>) -> PluginError {
    PluginError::Runtime("plugin host lock poisoned".into())
}

// ── Per-plugin log buffers ─────────────────────────────────────────────────

/// Bounded ring of log lines per plugin, readable by the frontend.
pub struct PluginLogs {
    buffers: Mutex<HashMap<String, VecDeque<String>>>,
    cap: usize,
}

impl Default for PluginLogs {
    fn default() -> Self {
        Self::new()
    }
}

impl PluginLogs {
    pub fn new() -> Self {
        Self {
            buffers: Mutex::new(HashMap::new()),
            cap: 500,
        }
    }

    pub fn push(&self, plugin_id: &str, level: PluginLogLevel, message: &str) {
        let line = format!("[{}] {message}", level_label(level));
        if let Ok(mut buffers) = self.buffers.lock() {
            let queue = buffers.entry(plugin_id.to_string()).or_default();
            if queue.len() >= self.cap {
                queue.pop_front();
            }
            queue.push_back(line);
        }
    }

    pub fn lines(&self, plugin_id: &str) -> Vec<String> {
        self.buffers
            .lock()
            .map(|b| {
                b.get(plugin_id)
                    .map(|q| q.iter().cloned().collect())
                    .unwrap_or_default()
            })
            .unwrap_or_default()
    }

    pub fn clear(&self, plugin_id: &str) {
        if let Ok(mut buffers) = self.buffers.lock() {
            buffers.remove(plugin_id);
        }
    }
}

fn level_label(level: PluginLogLevel) -> &'static str {
    match level {
        PluginLogLevel::Error => "ERROR",
        PluginLogLevel::Warn => "WARN",
        PluginLogLevel::Info => "INFO",
        PluginLogLevel::Debug => "DEBUG",
        PluginLogLevel::Trace => "TRACE",
    }
}

// ── Real HostApi for plugin instances ──────────────────────────────────────

/// HostApi implementation backed by the plugin manager, the bus and the log
/// buffers. One instance per plugin; capabilities come from the manifest.
pub struct PluginHostApi {
    bus: Arc<PluginBus>,
    manager: Arc<Mutex<micyou_plugin::PluginManager>>,
    logs: Arc<PluginLogs>,
    sound: Arc<crate::sound_player::SoundPlayer>,
    plugin_id: String,
}

impl PluginHostApi {
    pub fn new(
        bus: Arc<PluginBus>,
        manager: Arc<Mutex<micyou_plugin::PluginManager>>,
        logs: Arc<PluginLogs>,
        sound: Arc<crate::sound_player::SoundPlayer>,
        plugin_id: String,
    ) -> Arc<Self> {
        Arc::new(Self {
            bus,
            manager,
            logs,
            sound,
            plugin_id,
        })
    }
}

impl HostApi for PluginHostApi {
    fn log(&self, level: PluginLogLevel, message: &str) {
        self.logs.push(&self.plugin_id, level, message);
        log::info!(target: "plugin", "[{}] {}", self.plugin_id, message);
    }

    fn get_config(&self, key: &str) -> Option<serde_json::Value> {
        let manager = self.manager.lock().ok()?;
        manager
            .plugin_config(&self.plugin_id)
            .ok()?
            .get(key)
            .cloned()
    }

    fn set_config(&self, key: &str, value: serde_json::Value) -> PluginResult<()> {
        let manager = self.manager.lock().map_err(lock_err)?;
        manager.set_plugin_config(&self.plugin_id, key, value)
    }

    fn emit_event(&self, topic: &str, payload: serde_json::Value) -> PluginResult<()> {
        let bytes = serde_json::to_vec(&payload)
            .map_err(|e| PluginError::Runtime(format!("event serialization: {e}")))?;
        self.bus.publish(topic, bytes)
    }

    fn send_message(&self, target: MessageTarget, payload: Vec<u8>) -> PluginResult<()> {
        match target {
            MessageTarget::Local { plugin_id } => {
                let msg = PluginMessage::new(&self.plugin_id, &plugin_id, &plugin_id, payload);
                self.bus.handle_incoming(&msg);
                Ok(())
            }
            MessageTarget::Remote { plugin_id } => {
                let msg = PluginMessage::new(&self.plugin_id, &plugin_id, &plugin_id, payload);
                self.bus.transport().send(&msg)
            }
            MessageTarget::Broadcast => {
                let msg = PluginMessage::new(&self.plugin_id, "", "broadcast", payload);
                self.bus.handle_incoming(&msg);
                if self.bus.transport().is_connected() {
                    self.bus.transport().send(&msg)?;
                }
                Ok(())
            }
        }
    }

    fn audio_state(&self) -> AudioStateSnapshot {
        // Real-time audio state is wired by the app through the bus topics;
        // the snapshot defaults are safe for plugins that only read config.
        AudioStateSnapshot::default()
    }

    fn play_sound(&self, path: &str) -> PluginResult<()> {
        self.sound.play_wav(path)
    }

    fn connected_devices(&self) -> Vec<DeviceSnapshot> {
        if self.bus.transport().is_connected() {
            vec![DeviceSnapshot {
                mode: "wifi".to_string(),
                label: "connected device".to_string(),
                audio_active: true,
            }]
        } else {
            Vec::new()
        }
    }
}
