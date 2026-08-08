//! `micyou plugin` — plugin development toolkit.
//!
//! Subcommands:
//! - `validate <dir>`  validate a plugin directory's plugin.json
//! - `package <dir>`   pack a plugin directory into an importable .zip
//! - `create <id>`     scaffold a new plugin (wasm or native skeleton)

use std::io::Write as _;
use std::path::Path;

use clap::Subcommand;

#[derive(Subcommand)]
pub enum PluginAction {
    /// 校验插件目录中的 plugin.json（结构、版本、能力、平台）
    Validate {
        /// 插件目录（含 plugin.json）
        dir: String,
    },
    /// 将插件目录打包为可导入的 .zip（根目录含 plugin.json）
    Package {
        /// 插件目录
        dir: String,
        /// 输出 zip 路径（默认 <plugin_id>.zip）
        #[arg(short, long)]
        out: Option<String>,
    },
    /// 生成新插件骨架（wasm 或 native 模板）
    Create {
        /// 插件 id（反向域名，如 dev.micyou.myplugin）
        id: String,
        /// 运行时：wasm（默认，沙箱安全）| native（高级 DSP/系统集成）
        #[arg(long, value_parser = ["wasm", "native"], default_value = "wasm")]
        runtime: String,
        /// 插件显示名（默认取自 id）
        #[arg(long)]
        name: Option<String>,
        /// 输出目录（默认 ./<id 最后一段>）
        #[arg(short, long)]
        out: Option<String>,
    },
}

pub fn run(action: PluginAction) -> Result<(), String> {
    match action {
        PluginAction::Validate { dir } => validate(&dir),
        PluginAction::Package { dir, out } => package(&dir, out.as_deref()),
        PluginAction::Create {
            id,
            runtime,
            name,
            out,
        } => create(&id, &runtime, name.as_deref(), out.as_deref()),
    }
}

fn validate(dir: &str) -> Result<(), String> {
    let manifest_path = Path::new(dir).join("plugin.json");
    let text = std::fs::read_to_string(&manifest_path)
        .map_err(|e| format!("read {}: {e}", manifest_path.display()))?;
    let manifest = micyou_plugin::PluginManifest::from_json(&text)
        .map_err(|e| format!("invalid plugin.json: {e}"))?;
    println!(
        "OK  id={} name={} version={} runtime={:?}",
        manifest.id, manifest.name, manifest.version, manifest.runtime
    );
    println!("    capabilities={:?}", manifest.capabilities);
    println!(
        "    kind={:?} platforms={:?} arches={:?}",
        manifest.kind, manifest.platforms, manifest.arches
    );
    let entry = Path::new(dir).join(&manifest.entry);
    if !entry.exists() {
        return Err(format!("entry artifact missing: {}", entry.display()));
    }
    println!("    entry={} (exists)", entry.display());
    Ok(())
}

fn package(dir: &str, out: Option<&str>) -> Result<(), String> {
    let manifest_text = std::fs::read_to_string(Path::new(dir).join("plugin.json"))
        .map_err(|e| format!("read plugin.json: {e}"))?;
    let manifest = micyou_plugin::PluginManifest::from_json(&manifest_text)
        .map_err(|e| format!("invalid plugin.json: {e}"))?;
    let out_path = out
        .map(|o| o.to_string())
        .unwrap_or_else(|| format!("{}.zip", manifest.id));
    let file = std::fs::File::create(&out_path).map_err(|e| format!("create {}: {e}", out_path))?;
    let mut zipw = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);

    // Walk the plugin dir (skip target/ and hidden files).
    let mut entries = Vec::new();
    collect_entries(Path::new(dir), Path::new(dir), &mut entries)?;
    for (abs, rel) in &entries {
        let rel_str = rel.to_string_lossy().replace('\\', "/");
        zipw.start_file(rel_str.clone(), options)
            .map_err(|e| format!("zip add {}: {e}", rel_str))?;
        let bytes = std::fs::read(abs).map_err(|e| format!("read {}: {e}", abs.display()))?;
        zipw.write_all(&bytes)
            .map_err(|e| format!("zip write {}: {e}", rel_str))?;
    }
    zipw.finish().map_err(|e| format!("zip finish: {e}"))?;
    println!("packed {} entries -> {}", entries.len(), out_path);
    Ok(())
}

fn collect_entries(
    root: &Path,
    dir: &Path,
    out: &mut Vec<(std::path::PathBuf, std::path::PathBuf)>,
) -> Result<(), String> {
    let rd = std::fs::read_dir(dir).map_err(|e| format!("read_dir {}: {e}", dir.display()))?;
    for entry in rd {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        let name = entry.file_name();
        if name == "target" || name.to_string_lossy().starts_with('.') {
            continue;
        }
        let rel = path
            .strip_prefix(root)
            .map_err(|e| e.to_string())?
            .to_path_buf();
        if path.is_dir() {
            collect_entries(root, &path, out)?;
        } else {
            out.push((path, rel));
        }
    }
    Ok(())
}

const WASM_PLUGIN_JSON: &str = r#"{
  "id": "dev.micyou.example.myplugin",
  "name": "My Plugin",
  "version": "1.0.0",
  "author": "you@example.com",
  "description": "A WASM plugin scaffold",
  "license": "MIT",
  "homepage": "https://example.com",
  "keywords": ["wasm"],
  "runtime": "wasm",
  "entry": "main.wasm",
  "platforms": ["linux", "windows", "macos"],
  "arches": [],
  "apiVersion": 1,
  "minHostVersion": "1.0.0",
  "capabilities": ["config.read", "config.write"],
  "kind": "utility",
  "config": {}
}
"#;

const WASM_TEMPLATE_WAT: &str = r#";; MicYou WASM plugin template
;; Build: micyou plugin package <dir> (or compile with wat2wasm)
(module
  (import "micyou" "log" (func $log (param i32 i32)))
  (import "micyou" "get_config" (func $get_config (param i32 i32) (result i32)))
  (import "micyou" "set_config" (func $set_config (param i32 i32 i32) (result i32)))
  (memory (export "memory") 4)
  ;; bump allocator (heap starts after statics)
  (global $heap (mut i32) (i32.const 0x2000))
  (func (export "alloc") (param $n i32) (result i32)
    (local $p i32)
    (local.set $p (global.get $heap))
    (global.set $heap (i32.add (global.get $heap) (local.get $n)))
    (i32.store (local.get $p) (local.get $n))
    (i32.add (local.get $p) (i32.const 4)))
  (func (export "dealloc") (param $p i32) (param $n i32))
  (func (export "api_version") (result i32) (i32.const 1))
  (func (export "init") (result i32)
    (i32.store (i32.const 0) (i32.const 0))
    (i32.const 0))
  (func (export "process") (param $data i32) (param $samples i32) (param $channels i32) (param $queued f64) (result i32)
    (i32.const 0))
  (func (export "handle_message") (param $ptr i32) (param $len i32) (result i32)
    (i32.const 0))
  (func (export "deinit"))
)
"#;

const WASM_PANEL_HTML: &str = r#"<!DOCTYPE html>
<html lang="zh">
<head><meta charset="utf-8"><title>插件面板</title>
<style>
  body { font-family: system-ui, sans-serif; background: hsl(var(--surface)); color: hsl(var(--on-surface)); padding: 20px; }
  .card { background: hsl(var(--surface-bright)); border: 1px solid hsl(var(--border)); border-radius: 1rem; padding: 16px; }
  h2 { color: hsl(var(--primary)); margin: 0 0 12px; }
</style>
</head>
<body>
<div class="card"><h2>插件面板</h2><p>在此编写你的插件 UI，通过 postMessage 桥调用宿主 API</p></div>
<script>
function call(api, args) {
  return new Promise((resolve, reject) => {
    const id = Math.random().toString(36).slice(2);
    const on = (e) => { if (e.data && e.data.__micyou === 1 && e.data.id === id) {
      window.removeEventListener('message', on);
      e.data.ok ? resolve(e.data.value) : reject(new Error(e.data.error));
    } };
    window.addEventListener('message', on);
    window.parent.postMessage({ __micyou: 1, id, api, args: args || {} }, '*');
  });
}
call('get_config', {}).then((cfg) => console.log('config', cfg)).catch(console.error);
</script>
</body>
</html>
"#;

const NATIVE_PLUGIN_JSON: &str = r#"{
  "id": "dev.micyou.example.mynative",
  "name": "My Native Plugin",
  "version": "1.0.0",
  "author": "you@example.com",
  "description": "A native cdylib plugin scaffold",
  "license": "MIT",
  "runtime": "native",
  "entry": "libmicyou_example_mynative.so",
  "platforms": ["linux", "windows", "macos"],
  "arches": ["x86_64", "aarch64"],
  "apiVersion": 1,
  "minHostVersion": "1.0.0",
  "capabilities": ["config.read", "config.write"],
  "kind": "utility",
  "config": {}
}
"#;

const NATIVE_TEMPLATE_LIB: &str = r#"//! MicYou native plugin template (cdylib).
//! Copy include/micyou_plugin_abi.h into your crate or match these structs.

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(non_camel_case_types)]
pub enum mpl_result_t {
    MPL_OK = 0,
    MPL_ERR_NOT_IMPLEMENTED = 1,
    MPL_ERR_INVALID_ARG = 2,
    MPL_ERR_RUNTIME = 3,
    MPL_ERR_BUFFER_TOO_SMALL = 4,
    MPL_ERR_PERMISSION = 5,
}

#[repr(C)]
pub struct mpl_plugin_info_t {
    pub abi_version: u32,
    pub id: *const std::ffi::c_char,
    pub name: *const std::ffi::c_char,
    pub version: *const std::ffi::c_char,
}

#[repr(C)]
pub struct mpl_host_api_t {
    pub ctx: *mut std::ffi::c_void,
    pub log: unsafe extern "C" fn(*mut std::ffi::c_void, i32, *const std::ffi::c_char) -> mpl_result_t,
    pub get_config: unsafe extern "C" fn(*mut std::ffi::c_void, *const std::ffi::c_char, *mut std::ffi::c_char, *mut u32) -> mpl_result_t,
    pub set_config: unsafe extern "C" fn(*mut std::ffi::c_void, *const std::ffi::c_char, *const std::ffi::c_char) -> mpl_result_t,
    pub emit_event: unsafe extern "C" fn(*mut std::ffi::c_void, *const std::ffi::c_char, *const std::ffi::c_char) -> mpl_result_t,
    pub send_message: unsafe extern "C" fn(*mut std::ffi::c_void, *const std::ffi::c_char, *const u8, u32) -> mpl_result_t,
    pub audio_state: unsafe extern "C" fn(*mut std::ffi::c_void, *mut std::ffi::c_char, *mut u32) -> mpl_result_t,
    pub connected_devices: unsafe extern "C" fn(*mut std::ffi::c_void, *mut std::ffi::c_char, *mut u32) -> mpl_result_t,
    pub play_sound: unsafe extern "C" fn(*mut std::ffi::c_void, *const std::ffi::c_char) -> mpl_result_t,
    pub plugin_dir: unsafe extern "C" fn(*mut std::ffi::c_void, *mut std::ffi::c_char, *mut u32) -> mpl_result_t,
    pub register_hotkey: unsafe extern "C" fn(*mut std::ffi::c_void, *const std::ffi::c_char, *mut u64) -> mpl_result_t,
    pub open_window: unsafe extern "C" fn(*mut std::ffi::c_void, *const std::ffi::c_char) -> mpl_result_t,
}

const ID: &str = "dev.micyou.example.mynative";

#[no_mangle]
pub extern "C" fn micyou_plugin_info() -> *const mpl_plugin_info_t {
    static INFO: mpl_plugin_info_t = mpl_plugin_info_t {
        abi_version: 1,
        id: b"dev.micyou.example.mynative\0".as_ptr() as *const std::ffi::c_char,
        name: b"My Native Plugin\0".as_ptr() as *const std::ffi::c_char,
        version: b"1.0.0\0".as_ptr() as *const std::ffi::c_char,
    };
    &INFO
}

#[no_mangle]
pub extern "C" fn micyou_plugin_init(host: *const mpl_host_api_t) -> mpl_result_t {
    unsafe {
        let host = &*host;
        let msg = std::ffi::CString::new(format!("{ID} initialized")).unwrap();
        ((*host).log)(host.ctx, 2, msg.as_ptr());
    }
    mpl_result_t::MPL_OK
}

#[no_mangle]
pub extern "C" fn micyou_plugin_deinit() {}

#[no_mangle]
pub extern "C" fn micyou_plugin_process(
    data: *mut f32,
    samples: u32,
    channels: u32,
    queued_ms: f64,
) -> mpl_result_t {
    unsafe {
        // TODO: real-time-safe DSP here. Never call host APIs from process().
        let _ = (data, samples, channels, queued_ms);
    }
    mpl_result_t::MPL_OK
}

#[no_mangle]
pub extern "C" fn micyou_plugin_handle_message(
    source: *const std::ffi::c_char,
    topic: *const std::ffi::c_char,
    payload: *const u8,
    payload_len: u32,
) -> mpl_result_t {
    let _ = (source, topic, payload, payload_len);
    mpl_result_t::MPL_OK
}
"#;

const NATIVE_CARGO: &str = r#"[package]
name = "micyou-example-mynative"
version = "1.0.0"
edition = "2021"

[lib]
crate-type = ["cdylib"]
"#;

fn create(id: &str, runtime: &str, name: Option<&str>, out: Option<&str>) -> Result<(), String> {
    let last = id.rsplit('.').next().unwrap_or(id);
    let out_dir = out
        .map(|o| o.to_string())
        .unwrap_or_else(|| last.to_string());
    let dir = Path::new(&out_dir);
    std::fs::create_dir_all(dir).map_err(|e| format!("mkdir {out_dir}: {e}"))?;
    let display_name = name.unwrap_or(last).to_string();
    if runtime == "native" {
        let plugin_json = NATIVE_PLUGIN_JSON
            .replace("dev.micyou.example.mynative", id)
            .replace("My Native Plugin", &display_name)
            .replace("libmicyou_example_mynative.so", &format!("lib{last}.so"));
        write_file(dir, "plugin.json", &plugin_json)?;
        write_file(dir, "README.md", NATIVE_README)?;
        write_file(dir, "Cargo.toml", NATIVE_CARGO)?;
        let lib = NATIVE_TEMPLATE_LIB.replace("dev.micyou.example.mynative", id);
        std::fs::create_dir_all(dir.join("src")).map_err(|e| format!("mkdir src: {e}"))?;
        write_file(&dir.join("src"), "lib.rs", &lib)?;
    } else {
        let plugin_json = WASM_PLUGIN_JSON
            .replace("dev.micyou.example.myplugin", id)
            .replace("My Plugin", &display_name);
        write_file(dir, "plugin.json", &plugin_json)?;
        write_file(dir, "README.md", WASM_README)?;
        write_file(dir, "main.wat", WASM_TEMPLATE_WAT)?;
        write_file(dir, "panel.html", WASM_PANEL_HTML)?;
        let _ = micyou_plugin::manifest::RuntimeKind::Wasm; // keep import alive
    }
    println!(
        "created {runtime} plugin skeleton in {}/  (compile the entry artifact, then `micyou plugin package {out_dir}`)",
        dir.display()
    );
    Ok(())
}

const WASM_README: &str = r#"# 插件骨架

## 构建入口
`main.wat` 编译为 `main.wasm`（wat2wasm 或 wat crate），产物放回本目录

## 安装
- 开发：把本目录放入 ~/.config/micyou/plugins/<id>/
- 分发：`micyou plugin package .` 打包 zip 后在应用内导入

## 面板
panel.html 通过 postMessage 桥调用宿主 API（get_config/set_config/trigger 等）

## 能力
在 plugin.json 的 capabilities 中声明所需能力（config.read/config.write/dsp.node/...）
"#;

const NATIVE_README: &str = r#"# Native 插件骨架

## 构建
`cargo build --release`，产物 target/release/lib*.so 复制到插件目录并改名与
plugin.json 的 entry 一致

## 说明
native 插件拥有宿主完整权限，用于实时 DSP、硬件与深度系统集成；
普通逻辑/UI 优先使用 wasm 插件（沙箱安全）

## 能力
按需声明 capabilities；process() 内禁止调用宿主 API（实时安全）
"#;

fn write_file(dir: &Path, name: &str, content: &str) -> Result<(), String> {
    let p = dir.join(name);
    std::fs::write(&p, content).map_err(|e| format!("write {}: {e}", p.display()))
}
