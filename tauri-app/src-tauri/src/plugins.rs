//! Plugin host wiring: owns the plugin manager and the DSP node registry,
//! shared by the audio thread (via `DspProcessor::set_external_hook`) and the
//! frontend commands (`commands/plugins.rs`).

use std::sync::{Arc, RwLock};

/// Runtime plugin host. One instance per process, managed Tauri state.
pub struct PluginHost {
    pub manager: micyou_plugin::PluginManager,
    pub dsp_registry: Arc<micyou_plugin::PluginDspRegistry>,
}

/// Default chain position for the synthetic plugin node: right after AEC,
/// so plugin processing runs on echo-cancelled audio.
pub const PLUGIN_NODE_AFTER: &str = "AEC";

impl PluginHost {
    pub fn new() -> Self {
        let config = crate::app_config::config_dir();
        Self {
            manager: micyou_plugin::PluginManager::new(
                config.join("plugins"),
                config.join("plugin-state.json"),
            ),
            dsp_registry: Arc::new(micyou_plugin::PluginDspRegistry::new()),
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
