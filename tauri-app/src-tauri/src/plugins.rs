//! Plugin host wiring: owns the plugin manager, the DSP node registry and the
//! cross-device message bus, shared by the audio thread (via
//! `DspProcessor::set_external_hook`), the TCP server (plugin message relay)
//! and the frontend commands (`commands/plugins.rs`).

use micyou_plugin::bus::{PluginBus, PluginMessage, PluginSyncTransport};
use micyou_plugin::PluginRuntime;
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
        self.sender
            .lock()
            .map(|g| g.is_some())
            .unwrap_or(false)
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
                if let Ok(mut instance) = manager.instance_mut(&id) {
                    let result =
                        instance.handle_message(&msg.source, &msg.topic, &msg.payload);
                    manager.return_instance(&id, instance);
                    result?;
                }
            }
            Ok(())
        });

        let bus = Arc::new(PluginBus::new(sync.clone(), dispatcher));

        Self {
            manager,
            dsp_registry,
            sync,
            bus,
        }
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
