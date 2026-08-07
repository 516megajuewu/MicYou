//! WASM plugin runtime.
//!
//! WebAssembly modules run in the `wasmi` interpreter — a pure-Rust engine
//! with no native dependencies, so the same runtime can later be embedded on
//! Android. Plugins import host functions under the `micyou` module; the host
//! writes strings/buffers into the plugin's linear memory through the
//! exported `alloc`/`dealloc` pair.
//!
//! Sandboxing: every entry-point call runs under a fuel budget (`EngineConfig`
//! with `consume_fuel`) so a plugin stuck in an infinite loop is trapped
//! instead of hanging the host. WASM DSP nodes are explicitly best-effort:
//! interpreter latency cannot guarantee real-time safety.

use crate::abi::mpl_result_t;
use crate::error::{PluginError, PluginResult};
use crate::host::HostApi;
use crate::host::PluginLogLevel;
use crate::manifest::PluginManifest;
use crate::plugin::{AudioFrameCtx, PluginEvent, PluginInstance, PluginRuntime, ProcessStatus};
use std::path::Path;
use std::sync::Arc;
use wasmi::{
    Config, Engine, Instance, Linker, Memory, Module, Store, TypedFunc, WasmParams, WasmResults,
};

/// Fuel granted to a plugin call before the engine traps it.
const CALL_FUEL_BUDGET: u64 = 100_000;

/// Host functions a WASM plugin can import (module `micyou`).
pub const WASM_IMPORT_MODULE: &str = "micyou";

/// Host-side state stored inside the wasmi `Store`.
pub struct WasmHostCtx {
    pub host: Arc<dyn HostApi>,
    pub capabilities: Vec<String>,
}

impl WasmHostCtx {
    fn require(&self, capability: &str) -> Result<(), PluginError> {
        if self.capabilities.iter().any(|c| c == capability) {
            Ok(())
        } else {
            Err(PluginError::PermissionDenied(format!(
                "plugin lacks capability {capability}"
            )))
        }
    }
}

/// A loaded WASM plugin.
pub struct WasmPlugin {
    manifest: PluginManifest,
    /// Kept alive so `store`/`instance`/`memory` remain valid.
    #[allow(dead_code)]
    engine: Engine,
    store: Store<WasmHostCtx>,
    instance: Instance,
    memory: Memory,
    f_init: Option<TypedFunc<(), i32>>,
    f_deinit: Option<TypedFunc<(), ()>>,
    f_process: Option<TypedFunc<(i32, i32, i32, f64), i32>>,
    f_event: Option<TypedFunc<(i32,), i32>>,
    f_message: Option<TypedFunc<(i32, i32), i32>>,
    f_alloc: TypedFunc<(i32,), i32>,
    f_dealloc: TypedFunc<(i32, i32), ()>,
}

// wasmi Store<T> is Send when T is Send; our ctx is an Arc + Vec.
unsafe impl Send for WasmPlugin {}

impl WasmPlugin {
    /// Load + instantiate a WASM module from `<plugin_dir>/<manifest.entry>`.
    pub fn load(
        manifest: PluginManifest,
        plugin_dir: &Path,
        host: Arc<dyn HostApi>,
    ) -> PluginResult<Self> {
        let entry = manifest.entry_path(plugin_dir);
        let bytes = std::fs::read(&entry)
            .map_err(|e| PluginError::NotFound(format!("{}: {e}", entry.display())))?;
        Self::from_bytes(manifest, bytes, host)
    }

    /// Instantiate a WASM module from raw bytes (used by tests and by embedders).
    pub fn from_bytes(
        manifest: PluginManifest,
        wasm_bytes: Vec<u8>,
        host: Arc<dyn HostApi>,
    ) -> PluginResult<Self> {
        if manifest.api_version != crate::manifest::HOST_API_VERSION {
            return Err(PluginError::ApiVersionMismatch {
                plugin: manifest.api_version,
                host: crate::manifest::HOST_API_VERSION,
            });
        }

        let mut config = Config::default();
        config.consume_fuel(true);
        let engine = Engine::new(&config);
        let module = Module::new(&engine, &wasm_bytes[..])
            .map_err(|e| PluginError::LoadFailed(format!("module parse: {e}")))?;

        let ctx = WasmHostCtx {
            host,
            capabilities: manifest.capabilities.clone(),
        };
        let mut store = Store::new(&engine, ctx);

        let mut linker = Linker::new(&engine);
        register_host_functions(&mut linker);

        let instance = linker
            .instantiate_and_start(&mut store, &module)
            .map_err(|e| PluginError::LoadFailed(format!("instantiate: {e}")))?;

        let memory = instance
            .get_memory(&store, "memory")
            .ok_or_else(|| PluginError::LoadFailed("module must export linear memory".into()))?;

        let f_alloc: TypedFunc<(i32,), i32> = instance
            .get_typed_func(&store, "alloc")
            .map_err(|e| PluginError::LoadFailed(format!("missing alloc export: {e}")))?;
        let f_dealloc: TypedFunc<(i32, i32), ()> = instance
            .get_typed_func(&store, "dealloc")
            .map_err(|e| PluginError::LoadFailed(format!("missing dealloc export: {e}")))?;

        let f_init = optional_func(&instance, &store, "init")?;
        let f_deinit = optional_func(&instance, &store, "deinit")?;
        let f_process = optional_func(&instance, &store, "process")?;
        let f_event = optional_func(&instance, &store, "handle_event")?;
        let f_message = optional_func(&instance, &store, "handle_message")?;

        Ok(WasmPlugin {
            manifest,
            engine,
            store,
            instance,
            memory,
            f_init,
            f_deinit,
            f_process,
            f_event,
            f_message,
            f_alloc,
            f_dealloc,
        })
    }

    /// Run `f` with a fresh fuel budget, mapping fuel exhaustion to an error.
    fn with_fuel<T>(&mut self, f: impl FnOnce(&mut Self) -> PluginResult<T>) -> PluginResult<T> {
        self.store
            .set_fuel(CALL_FUEL_BUDGET)
            .map_err(|e| PluginError::Runtime(format!("set fuel: {e}")))?;
        let result = f(self);
        // Fuel < 0 means the budget was exhausted mid-call.
        if result.is_ok() {
            if let Ok(fuel) = self.store.get_fuel() {
                if fuel == 0 {
                    return Err(PluginError::Runtime(
                        "wasm fuel exhausted (plugin consumed its execution budget)".into(),
                    ));
                }
            }
        }
        result
    }

    /// Write a NUL-terminated string into plugin memory; returns its address.
    fn write_str(&mut self, text: &str) -> PluginResult<i32> {
        let size = text.len() as i32 + 1;
        let ptr = self
            .f_alloc
            .call(&mut self.store, (size,))
            .map_err(|e| PluginError::Runtime(format!("alloc: {e}")))?;
        let bytes = text.as_bytes();
        self.memory
            .write(&mut self.store, ptr as usize, bytes)
            .map_err(|e| PluginError::Runtime(format!("write string: {e}")))?;
        self.memory
            .write(&mut self.store, ptr as usize + bytes.len(), &[0u8])
            .map_err(|e| PluginError::Runtime(format!("write NUL: {e}")))?;
        Ok(ptr)
    }

    /// Read a NUL-terminated string from plugin memory (test/debug helper).
    pub fn read_str(&mut self, ptr: i32) -> PluginResult<String> {
        // 0 is a valid linear-memory address (plugin statics may live there);
        // only negative pointers mean "no string".
        if ptr < 0 {
            return Ok(String::new());
        }
        let mut bytes: Vec<u8> = Vec::new();
        let mut offset = ptr as usize;
        let mut one = [0u8; 1];
        loop {
            self.memory
                .read(&mut self.store, offset, &mut one)
                .map_err(|e| PluginError::Runtime(format!("read string: {e}")))?;
            if one[0] == 0 {
                break;
            }
            bytes.push(one[0]);
            offset += 1;
        }
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }

    /// Expose the wasmi store mutably (test/debug helper).
    pub fn store_mut(&mut self) -> &mut Store<WasmHostCtx> {
        &mut self.store
    }

    /// Expose the wasmi store immutably (test/debug helper).
    pub fn store_ref(&self) -> &Store<WasmHostCtx> {
        &self.store
    }

    /// Expose the wasmi instance (test/debug helper).
    pub fn instance_ref(&self) -> &Instance {
        &self.instance
    }

    /// Fetch a typed export without tripping the borrow checker (test/debug
    /// helper). Returns `None` when the export is missing or mis-typed.
    pub fn export<Params, Results>(&mut self, name: &str) -> Option<TypedFunc<Params, Results>>
    where
        Params: WasmParams,
        Results: WasmResults,
    {
        // Fresh fuel for the caller's subsequent direct call.
        let _ = self.store.set_fuel(CALL_FUEL_BUDGET);
        self.instance.get_typed_func(&mut self.store, name).ok()
    }

    /// Write raw bytes into plugin memory; returns their address.
    fn write_bytes(&mut self, data: &[u8]) -> PluginResult<i32> {
        let ptr = self
            .f_alloc
            .call(&mut self.store, (data.len() as i32,))
            .map_err(|e| PluginError::Runtime(format!("alloc bytes: {e}")))?;
        self.memory
            .write(&mut self.store, ptr as usize, data)
            .map_err(|e| PluginError::Runtime(format!("write bytes: {e}")))?;
        Ok(ptr)
    }

    fn read_bytes(&mut self, ptr: i32, len: usize) -> PluginResult<Vec<u8>> {
        let mut buf = vec![0u8; len];
        self.memory
            .read(&mut self.store, ptr as usize, &mut buf)
            .map_err(|e| PluginError::Runtime(format!("read bytes: {e}")))?;
        Ok(buf)
    }

    /// Serialize an event to JSON and deliver it.
    fn deliver_event(&mut self, event: &PluginEvent) -> PluginResult<()> {
        let Some(f_event) = self.f_event else {
            return Ok(());
        };
        let json = serde_json::to_string(event)
            .map_err(|e| PluginError::Runtime(format!("event serialize: {e}")))?;
        let ptr = self.write_str(&json)?;
        let code = f_event
            .call(&mut self.store, (ptr,))
            .map_err(|e| PluginError::Runtime(format!("handle_event: {e}")))?;
        self.f_dealloc
            .call(&mut self.store, (ptr, json.len() as i32 + 1))?;
        result_from_wasm_code(code, "handle_event")
    }
}

fn optional_func<Params, Results>(
    instance: &Instance,
    store: &Store<WasmHostCtx>,
    name: &str,
) -> PluginResult<Option<TypedFunc<Params, Results>>>
where
    Params: WasmParams,
    Results: WasmResults,
{
    Ok(instance.get_typed_func(store, name).ok())
}

fn result_from_wasm_code(code: i32, context: &str) -> PluginResult<()> {
    match code {
        0 => Ok(()),
        _ => Err(PluginError::Runtime(format!(
            "{context}: plugin returned {code}"
        ))),
    }
}

impl PluginRuntime for WasmPlugin {
    fn manifest(&self) -> &PluginManifest {
        &self.manifest
    }

    fn init(&mut self, _host: &dyn HostApi) -> PluginResult<()> {
        let Some(f_init) = self.f_init else {
            return Ok(());
        };
        self.with_fuel(|this| {
            let code = f_init
                .call(&mut this.store, ())
                .map_err(|e| PluginError::Runtime(format!("init: {e}")))?;
            result_from_wasm_code(code, "init")
        })
    }

    fn deinit(&mut self) {
        if let Some(f_deinit) = &self.f_deinit {
            let _ = f_deinit.call(&mut self.store, ());
        }
    }

    fn process_audio(&mut self, ctx: &mut AudioFrameCtx<'_>) -> PluginResult<ProcessStatus> {
        let Some(f_process) = self.f_process else {
            return Ok(ProcessStatus::Bypass);
        };
        if ctx.data.is_empty() {
            return Ok(ProcessStatus::Bypass);
        }
        self.with_fuel(|this| {
            // f32 → little-endian bytes in plugin memory
            let mut bytes = Vec::with_capacity(ctx.data.len() * 4);
            for sample in ctx.data.iter() {
                bytes.extend_from_slice(&sample.to_le_bytes());
            }
            let ptr = this.write_bytes(&bytes)?;
            let code = f_process
                .call(
                    &mut this.store,
                    (
                        ptr,
                        ctx.data.len() as i32,
                        ctx.channels as i32,
                        ctx.queued_ms,
                    ),
                )
                .map_err(|e| PluginError::Runtime(format!("process: {e}")))?;
            let processed = this.read_bytes(ptr, bytes.len())?;
            this.f_dealloc
                .call(&mut this.store, (ptr, bytes.len() as i32))?;
            for (sample, chunk) in ctx.data.iter_mut().zip(processed.chunks_exact(4)) {
                *sample = f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
            }
            if code == 1 {
                Ok(ProcessStatus::Bypass)
            } else {
                result_from_wasm_code(code, "process")?;
                Ok(ProcessStatus::Ok)
            }
        })
    }

    fn handle_event(&mut self, event: &PluginEvent) -> PluginResult<()> {
        self.with_fuel(|this| this.deliver_event(event))
    }

    fn handle_message(&mut self, _source: &str, _topic: &str, payload: &[u8]) -> PluginResult<()> {
        let Some(f_message) = self.f_message else {
            return Ok(());
        };
        self.with_fuel(|this| {
            // Payload lives in memory; source/topic are delivered via host events.
            let ptr = this.write_bytes(payload)?;
            let code = f_message
                .call(&mut this.store, (ptr, payload.len() as i32))
                .map_err(|e| PluginError::Runtime(format!("handle_message: {e}")))?;
            this.f_dealloc
                .call(&mut this.store, (ptr, payload.len() as i32))?;
            result_from_wasm_code(code, "handle_message")
        })
    }
}

/// Convenience: load a WASM plugin and wrap it as a `PluginInstance`.
pub fn load_wasm_instance(
    manifest: PluginManifest,
    plugin_dir: &Path,
    host: Arc<dyn HostApi>,
) -> PluginResult<PluginInstance> {
    Ok(PluginInstance::Wasm(Box::new(WasmPlugin::load(
        manifest, plugin_dir, host,
    )?)))
}

// ── Host function registration ─────────────────────────────────────────────

fn register_host_functions(linker: &mut Linker<WasmHostCtx>) {
    // log(level: i32, msg_ptr: i32)
    linker
        .func_wrap(
            WASM_IMPORT_MODULE,
            "log",
            |mut caller: wasmi::Caller<'_, WasmHostCtx>, level: i32, ptr: i32| {
                let memory = caller.get_export("memory").and_then(|e| e.into_memory());
                let Some(memory) = memory else {
                    return Err(wasmi::Error::new("memory export missing"));
                };
                let text = read_str_from_memory(&mut caller, &memory, ptr)?;
                let level = match level {
                    0 => PluginLogLevel::Error,
                    1 => PluginLogLevel::Warn,
                    2 => PluginLogLevel::Info,
                    3 => PluginLogLevel::Debug,
                    _ => PluginLogLevel::Trace,
                };
                caller.data().host.log(level, &text);
                Ok(())
            },
        )
        .unwrap();

    // get_config(key_ptr: i32) -> ptr (host-allocated JSON, or 0)
    linker
        .func_wrap(
            WASM_IMPORT_MODULE,
            "get_config",
            |mut caller: wasmi::Caller<'_, WasmHostCtx>,
             key_ptr: i32|
             -> Result<i32, wasmi::Error> {
                caller
                    .data()
                    .require(crate::manifest::capabilities::CONFIG_READ)
                    .map_err(|e| wasmi::Error::new(e.to_string()))?;
                let memory = export_memory(&caller)?;
                let key = read_str_from_memory(&mut caller, &memory, key_ptr)?;
                match caller.data().host.get_config(&key) {
                    Some(value) => {
                        let json = value.to_string();
                        let ptr = write_str_to_memory(&mut caller, &memory, &json)?;
                        Ok(ptr)
                    }
                    None => Ok(0),
                }
            },
        )
        .unwrap();

    // set_config(key_ptr, value_ptr) -> result code
    linker
        .func_wrap(
            WASM_IMPORT_MODULE,
            "set_config",
            |mut caller: wasmi::Caller<'_, WasmHostCtx>,
             key_ptr: i32,
             value_ptr: i32|
             -> Result<i32, wasmi::Error> {
                caller
                    .data()
                    .require(crate::manifest::capabilities::CONFIG_WRITE)
                    .map_err(|e| wasmi::Error::new(e.to_string()))?;
                let memory = export_memory(&caller)?;
                let key = read_str_from_memory(&mut caller, &memory, key_ptr)?;
                let value_json = read_str_from_memory(&mut caller, &memory, value_ptr)?;
                let value: serde_json::Value = serde_json::from_str(&value_json)
                    .map_err(|e| wasmi::Error::new(format!("invalid json config: {e}")))?;
                caller
                    .data()
                    .host
                    .set_config(&key, value)
                    .map(|_| mpl_result_t::MPL_OK as i32)
                    .map_err(|e| wasmi::Error::new(e.to_string()))
            },
        )
        .unwrap();

    // emit_event(topic_ptr, payload_ptr) -> result code
    linker
        .func_wrap(
            WASM_IMPORT_MODULE,
            "emit_event",
            |mut caller: wasmi::Caller<'_, WasmHostCtx>,
             topic_ptr: i32,
             payload_ptr: i32|
             -> Result<i32, wasmi::Error> {
                caller
                    .data()
                    .require(crate::manifest::capabilities::EVENT_EMIT)
                    .map_err(|e| wasmi::Error::new(e.to_string()))?;
                let memory = export_memory(&caller)?;
                let topic = read_str_from_memory(&mut caller, &memory, topic_ptr)?;
                let payload_json = read_str_from_memory(&mut caller, &memory, payload_ptr)?;
                let payload: serde_json::Value = serde_json::from_str(&payload_json)
                    .map_err(|e| wasmi::Error::new(format!("invalid json payload: {e}")))?;
                caller
                    .data()
                    .host
                    .emit_event(&topic, payload)
                    .map(|_| mpl_result_t::MPL_OK as i32)
                    .map_err(|e| wasmi::Error::new(e.to_string()))
            },
        )
        .unwrap();

    // send_message(target_ptr, payload_ptr, payload_len) -> result code
    linker
        .func_wrap(
            WASM_IMPORT_MODULE,
            "send_message",
            |mut caller: wasmi::Caller<'_, WasmHostCtx>,
             target_ptr: i32,
             payload_ptr: i32,
             payload_len: i32|
             -> Result<i32, wasmi::Error> {
                caller
                    .data()
                    .require(crate::manifest::capabilities::MESSAGE_SEND)
                    .map_err(|e| wasmi::Error::new(e.to_string()))?;
                let memory = export_memory(&caller)?;
                let target_json = read_str_from_memory(&mut caller, &memory, target_ptr)?;
                let target: crate::host::MessageTarget = serde_json::from_str(&target_json)
                    .map_err(|e| wasmi::Error::new(format!("invalid target: {e}")))?;
                let payload =
                    read_bytes_from_memory(&mut caller, &memory, payload_ptr, payload_len)?;
                caller
                    .data()
                    .host
                    .send_message(target, payload)
                    .map(|_| mpl_result_t::MPL_OK as i32)
                    .map_err(|e| wasmi::Error::new(e.to_string()))
            },
        )
        .unwrap();

    // audio_state() -> ptr (host-allocated JSON, or 0)
    linker
        .func_wrap(
            WASM_IMPORT_MODULE,
            "audio_state",
            |mut caller: wasmi::Caller<'_, WasmHostCtx>| -> Result<i32, wasmi::Error> {
                caller
                    .data()
                    .require(crate::manifest::capabilities::AUDIO_STATE)
                    .map_err(|e| wasmi::Error::new(e.to_string()))?;
                let memory = export_memory(&caller)?;
                let json = serde_json::to_string(&caller.data().host.audio_state())
                    .map_err(|e| wasmi::Error::new(format!("serialize: {e}")))?;
                let ptr = write_str_to_memory(&mut caller, &memory, &json)?;
                Ok(ptr)
            },
        )
        .unwrap();

    // play_sound(path_ptr) -> result code
    linker
        .func_wrap(
            WASM_IMPORT_MODULE,
            "play_sound",
            |mut caller: wasmi::Caller<'_, WasmHostCtx>,
             path_ptr: i32|
             -> Result<i32, wasmi::Error> {
                caller
                    .data()
                    .require(crate::manifest::capabilities::AUDIO_PLAY)
                    .map_err(|e| wasmi::Error::new(e.to_string()))?;
                let memory = export_memory(&caller)?;
                let path = read_str_from_memory(&mut caller, &memory, path_ptr)?;
                caller
                    .data()
                    .host
                    .play_sound(&path)
                    .map(|_| mpl_result_t::MPL_OK as i32)
                    .map_err(|e| wasmi::Error::new(e.to_string()))
            },
        )
        .unwrap();

    // connected_devices() -> ptr (host-allocated JSON array, or 0)
    linker
        .func_wrap(
            WASM_IMPORT_MODULE,
            "connected_devices",
            |mut caller: wasmi::Caller<'_, WasmHostCtx>| -> Result<i32, wasmi::Error> {
                caller
                    .data()
                    .require(crate::manifest::capabilities::DEVICE_LIST)
                    .map_err(|e| wasmi::Error::new(e.to_string()))?;
                let memory = export_memory(&caller)?;
                let json = serde_json::to_string(&caller.data().host.connected_devices())
                    .map_err(|e| wasmi::Error::new(format!("serialize: {e}")))?;
                let ptr = write_str_to_memory(&mut caller, &memory, &json)?;
                Ok(ptr)
            },
        )
        .unwrap();
}

fn export_memory(caller: &wasmi::Caller<'_, WasmHostCtx>) -> Result<Memory, wasmi::Error> {
    caller
        .get_export("memory")
        .and_then(|e| e.into_memory())
        .ok_or_else(|| wasmi::Error::new("memory export missing"))
}

fn read_str_from_memory(
    caller: &mut wasmi::Caller<'_, WasmHostCtx>,
    memory: &Memory,
    ptr: i32,
) -> Result<String, wasmi::Error> {
    if ptr < 0 {
        return Ok(String::new());
    }
    let mut bytes: Vec<u8> = Vec::new();
    let mut offset = ptr as usize;
    let mut one = [0u8; 1];
    loop {
        memory
            .read(&mut *caller, offset, &mut one)
            .map_err(|e| wasmi::Error::new(format!("read string: {e}")))?;
        if one[0] == 0 {
            break;
        }
        bytes.push(one[0]);
        offset += 1;
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

fn read_bytes_from_memory(
    caller: &mut wasmi::Caller<'_, WasmHostCtx>,
    memory: &Memory,
    ptr: i32,
    len: i32,
) -> Result<Vec<u8>, wasmi::Error> {
    if ptr < 0 || len <= 0 {
        return Ok(Vec::new());
    }
    let mut buf = vec![0u8; len as usize];
    memory
        .read(&mut *caller, ptr as usize, &mut buf)
        .map_err(|e| wasmi::Error::new(format!("read bytes: {e}")))?;
    Ok(buf)
}

/// Allocate + write a NUL-terminated string via the plugin's exported alloc.
fn write_str_to_memory(
    caller: &mut wasmi::Caller<'_, WasmHostCtx>,
    memory: &Memory,
    text: &str,
) -> Result<i32, wasmi::Error> {
    let alloc: TypedFunc<(i32,), i32> = caller
        .get_export("alloc")
        .and_then(|e| e.into_func())
        .ok_or_else(|| wasmi::Error::new("alloc export missing"))?
        .typed(&mut *caller)
        .map_err(|e| wasmi::Error::new(format!("alloc typed: {e}")))?;
    let bytes = text.as_bytes();
    let ptr = alloc
        .call(&mut *caller, (bytes.len() as i32 + 1,))
        .map_err(|e| wasmi::Error::new(format!("alloc call: {e}")))?;
    memory
        .write(&mut *caller, ptr as usize, bytes)
        .map_err(|e| wasmi::Error::new(format!("write: {e}")))?;
    memory
        .write(&mut *caller, ptr as usize + bytes.len(), &[0u8])
        .map_err(|e| wasmi::Error::new(format!("write NUL: {e}")))?;
    Ok(ptr)
}
