//! WASM plugin runtime integration tests.
//!
//! A real WASM module (compiled from `fixtures/wasm/fixture.wat` via the `wat`
//! crate at test time — no external toolchain) is instantiated through wasmi:
//! host imports, linear-memory buffer exchange, DSP processing, event/message
//! delivery and fuel-metered sandboxing.

use micyou_plugin::host::{AudioStateSnapshot, DeviceSnapshot, HostApi, MessageTarget};
use micyou_plugin::manifest::{PluginKind, PluginManifest, RuntimeKind};
use micyou_plugin::plugin::{AudioFrameCtx, PluginEvent, PluginRuntime};
use micyou_plugin::{PluginLogLevel, PluginResult};
use std::path::Path;
use std::sync::{Arc, Mutex};

// ── Mock host ──────────────────────────────────────────────────────────────

#[derive(Default)]
struct MockHost {
    config: Mutex<std::collections::HashMap<String, serde_json::Value>>,
    log_lines: Mutex<Vec<String>>,
    emitted: Mutex<Vec<(String, serde_json::Value)>>,
    sent: Mutex<Vec<(MessageTarget, Vec<u8>)>>,
}

impl MockHost {
    fn new() -> Arc<Self> {
        let host = Arc::new(Self::default());
        host.config.lock().unwrap().insert(
            "fixture.key".into(),
            serde_json::json!({ "enabled": true, "gain": 2.0 }),
        );
        host
    }
}

impl HostApi for MockHost {
    fn log(&self, level: PluginLogLevel, message: &str) {
        self.log_lines
            .lock()
            .unwrap()
            .push(format!("{:?}: {message}", level));
    }
    fn get_config(&self, key: &str) -> Option<serde_json::Value> {
        self.config.lock().unwrap().get(key).cloned()
    }
    fn set_config(&self, key: &str, value: serde_json::Value) -> micyou_plugin::PluginResult<()> {
        self.config.lock().unwrap().insert(key.into(), value);
        Ok(())
    }
    fn emit_event(
        &self,
        topic: &str,
        payload: serde_json::Value,
    ) -> micyou_plugin::PluginResult<()> {
        self.emitted.lock().unwrap().push((topic.into(), payload));
        Ok(())
    }
    fn send_message(
        &self,
        target: MessageTarget,
        payload: Vec<u8>,
    ) -> micyou_plugin::PluginResult<()> {
        self.sent.lock().unwrap().push((target, payload));
        Ok(())
    }
    fn audio_state(&self) -> AudioStateSnapshot {
        AudioStateSnapshot::default()
    }
    fn play_sound(&self, _path: &str) -> PluginResult<()> { Ok(()) }
    fn plugin_dir(&self) -> String { "/tmp/plugin-dir".to_string() }
    fn connected_devices(&self) -> Vec<DeviceSnapshot> {
        Vec::new()
    }
}

fn fixture_wasm_bytes() -> Vec<u8> {
    let wat = include_str!("../fixtures/wasm/fixture.wat");
    wat::parse_str(wat).expect("fixture WAT must parse")
}

fn fixture_manifest() -> PluginManifest {
    PluginManifest {
        id: "test.wasm.fixture".to_string(),
        name: "Wasm Fixture".to_string(),
        version: "1.0.0".to_string(),
        author: None,
        description: None,
        runtime: RuntimeKind::Wasm,
        entry: "fixture.wasm".to_string(),
        platforms: Vec::new(),
        api_version: micyou_plugin::HOST_API_VERSION,
        capabilities: vec![
            micyou_plugin::capabilities::CONFIG_READ.to_string(),
            micyou_plugin::capabilities::CONFIG_WRITE.to_string(),
            micyou_plugin::capabilities::EVENT_EMIT.to_string(),
            micyou_plugin::capabilities::MESSAGE_SEND.to_string(),
            micyou_plugin::capabilities::AUDIO_STATE.to_string(),
            micyou_plugin::capabilities::DEVICE_LIST.to_string(),
        ],
        kind: PluginKind::Dsp,
        ui: None,
        dsp: None,
        config: None,
    }
}

fn load(host: &Arc<MockHost>) -> micyou_plugin::wasm::WasmPlugin {
    micyou_plugin::wasm::WasmPlugin::from_bytes(
        fixture_manifest(),
        fixture_wasm_bytes(),
        host.clone(),
    )
    .expect("fixture must instantiate")
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[test]
fn wasm_plugin_loads_and_processes_audio() {
    let host = MockHost::new();
    let mut plugin = load(&host);
    assert_eq!(plugin.manifest().id, "test.wasm.fixture");
    assert_eq!(plugin.manifest().runtime, RuntimeKind::Wasm);

    // set gain via the exported test helper through a raw func handle
    let set_gain: wasmi::TypedFunc<(f64,), ()> = plugin
        .export("test_set_gain")
        .expect("test_set_gain export");
    set_gain.call(plugin.store_mut(), (2.0,)).unwrap();

    let mut data = vec![0.1f32, -0.2, 0.3];
    let mut ctx = AudioFrameCtx {
        data: &mut data,
        channels: 1,
        sample_rate: 48000,
        queued_ms: 10.0,
    };
    let status = plugin.process_audio(&mut ctx).unwrap();
    assert_eq!(status, micyou_plugin::ProcessStatus::Ok);
    assert_eq!(data, vec![0.2, -0.4, 0.6]);

    // bypass path
    let set_gain: wasmi::TypedFunc<(f64,), ()> = plugin
        .export("test_set_gain")
        .expect("test_set_gain export");
    set_gain.call(plugin.store_mut(), (-1.0,)).unwrap();
    let mut data2 = vec![1.0f32, 2.0];
    let mut ctx2 = AudioFrameCtx {
        data: &mut data2,
        channels: 1,
        sample_rate: 48000,
        queued_ms: 10.0,
    };
    let status = plugin.process_audio(&mut ctx2).unwrap();
    assert_eq!(status, micyou_plugin::ProcessStatus::Bypass);
    assert_eq!(data2, vec![1.0, 2.0]);
}

#[test]
fn wasm_plugin_delivers_events_and_messages() {
    let host = MockHost::new();
    let mut plugin = load(&host);

    plugin
        .handle_event(&PluginEvent::MuteChanged { muted: true })
        .unwrap();
    plugin
        .handle_event(&PluginEvent::DeviceConnected {
            mode: "usb".into(),
            label: "device".into(),
        })
        .unwrap();
    plugin
        .handle_message("test.wasm.peer", "plugin:test.wasm.fixture", b"hello")
        .unwrap();

    let events: wasmi::TypedFunc<(), i32> =
        plugin.export("test_events").expect("test_events export");
    let messages: wasmi::TypedFunc<(), i32> = plugin
        .export("test_messages")
        .expect("test_messages export");
    assert_eq!(events.call(plugin.store_mut(), ()).unwrap(), 2);
    assert_eq!(messages.call(plugin.store_mut(), ()).unwrap(), 1);
}

#[test]
fn wasm_plugin_host_get_config_roundtrip() {
    let host = MockHost::new();
    let mut plugin = load(&host);

    let get_config: wasmi::TypedFunc<(), i32> = plugin
        .export("test_host_get_config")
        .expect("test_host_get_config export");
    let ptr = get_config.call(plugin.store_mut(), ()).unwrap();
    assert!(ptr > 0, "host must return an allocated JSON pointer");
    let value = plugin.read_str(ptr).expect("read string from memory");
    assert!(
        value.contains("enabled"),
        "config JSON missing key: {value}"
    );
}

#[test]
fn wasm_plugin_rejects_api_version_mismatch() {
    let host = MockHost::new();
    let mut manifest = fixture_manifest();
    manifest.api_version = 99;
    let result =
        micyou_plugin::wasm::WasmPlugin::from_bytes(manifest, fixture_wasm_bytes(), host.clone());
    assert!(matches!(
        result,
        Err(micyou_plugin::PluginError::ApiVersionMismatch { plugin: 99, .. })
    ));
}

#[test]
fn wasm_plugin_missing_module_reports_not_found() {
    let host = MockHost::new();
    let result = micyou_plugin::wasm::WasmPlugin::load(
        fixture_manifest(),
        Path::new("/nonexistent/plugin/dir"),
        host.clone(),
    );
    assert!(matches!(
        result,
        Err(micyou_plugin::PluginError::NotFound(_))
    ));
}

#[test]
fn wasm_plugin_fuel_exhaustion_is_trapped() {
    // A module with an infinite loop must be trapped by the fuel budget
    // instead of hanging the host.
    let infinite = wat::parse_str(
        r#"(module
              (memory (export "memory") 1)
              (func (export "alloc") (param i32) (result i32) (i32.const 16))
              (func (export "dealloc") (param i32) (param i32))
              (func (export "init") (result i32)
                (block $b
                  (loop $l
                    (br $l)))
                (i32.const 0))
            )"#,
    )
    .unwrap();
    let host = MockHost::new();
    let mut manifest = fixture_manifest();
    manifest.kind = PluginKind::Utility;
    let mut plugin = micyou_plugin::wasm::WasmPlugin::from_bytes(manifest, infinite, host.clone())
        .expect("module must instantiate");
    let result = plugin.init(&*host);
    assert!(
        matches!(result, Err(micyou_plugin::PluginError::Runtime(_))),
        "infinite loop must be trapped, got {result:?}"
    );
}

#[test]
fn wasm_plugin_missing_optional_exports_are_skipped() {
    // No process/event/message exports → those calls bypass/no-op.
    let minimal = wat::parse_str(
        r#"(module
              (memory (export "memory") 1)
              (func (export "alloc") (param i32) (result i32) (i32.const 16))
              (func (export "dealloc") (param i32) (param i32))
            )"#,
    )
    .unwrap();
    let host = MockHost::new();
    let mut manifest = fixture_manifest();
    manifest.kind = PluginKind::Utility;
    let mut plugin =
        micyou_plugin::wasm::WasmPlugin::from_bytes(manifest, minimal, host.clone()).unwrap();
    plugin.init(&*host).unwrap();
    let mut data = vec![1.0f32];
    let mut ctx = AudioFrameCtx {
        data: &mut data,
        channels: 1,
        sample_rate: 48000,
        queued_ms: 0.0,
    };
    assert_eq!(
        plugin.process_audio(&mut ctx).unwrap(),
        micyou_plugin::ProcessStatus::Bypass
    );
    plugin
        .handle_event(&PluginEvent::DspSettingsChanged)
        .unwrap();
    plugin.handle_message("src", "topic", b"data").unwrap();
}

// ── Example plugin validation ─────────────────────────────────────────────

/// The shipped example manifests must keep validating against the schema so
/// docs and CI stay honest.
#[test]
fn example_manifests_validate() {
    let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("workspace layout");
    // repo root is tauri-app/.. (two levels above crates/micyou-plugin)
    let repo_root = workspace_root
        .parent()
        .expect("repo layout");
    for (dir, id) in [
        ("plugins/examples/native-gain", "dev.micyou.example.gain"),
        ("plugins/examples/wasm-counter", "dev.micyou.example.counter"),
    ] {
        let path = repo_root.join(dir).join("plugin.json");
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("read {path:?}: {e}"));
        let manifest = micyou_plugin::manifest::PluginManifest::from_json(&text)
            .unwrap_or_else(|e| panic!("manifest {id} invalid: {e}"));
        assert_eq!(manifest.id, id);
    }

    // The example WAT must parse to valid wasm (toolchain-free check)
    let wat_path = repo_root
        .join("plugins/examples/wasm-counter/counter.wat");
    let wat = std::fs::read_to_string(&wat_path).unwrap();
    let bytes = wat::parse_str(&wat).expect("example counter.wat must parse");
    assert!(!bytes.is_empty());
}
