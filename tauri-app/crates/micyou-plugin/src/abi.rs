//! C ABI surface for native plugins.
//!
//! The structs in this module mirror `include/micyou_plugin_abi.h` exactly
//! (same field order, same types). The header is the source of truth for
//! plugin authors; this module is the host-side binding.
//!
//! ABI versioning: `MPL_ABI_VERSION` protects the struct layouts below,
//! `MPL_API_VERSION` (Host API) is what manifests declare. Both must match for
//! a native plugin to load.

#![allow(non_camel_case_types)]
#![allow(non_snake_case)]

use crate::error::{PluginError, PluginResult};
use crate::host::{HostApi, MessageTarget, PluginLogLevel};
use std::ffi::{c_char, c_void, CStr, CString};
use std::sync::Arc;

/// ABI version of the struct layouts in this module.
pub const MPL_ABI_VERSION: u32 = 1;

/// Result codes returned by plugin and host functions (mpl_result_t).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum mpl_result_t {
    MPL_OK = 0,
    MPL_ERR_NOT_IMPLEMENTED = 1,
    MPL_ERR_INVALID_ARG = 2,
    MPL_ERR_RUNTIME = 3,
    MPL_ERR_BUFFER_TOO_SMALL = 4,
    MPL_ERR_PERMISSION = 5,
}

/// Log levels (mpl_log_level_t).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum mpl_log_level_t {
    MPL_LOG_ERROR = 0,
    MPL_LOG_WARN = 1,
    MPL_LOG_INFO = 2,
    MPL_LOG_DEBUG = 3,
    MPL_LOG_TRACE = 4,
}

impl From<mpl_log_level_t> for PluginLogLevel {
    fn from(level: mpl_log_level_t) -> Self {
        match level {
            mpl_log_level_t::MPL_LOG_ERROR => PluginLogLevel::Error,
            mpl_log_level_t::MPL_LOG_WARN => PluginLogLevel::Warn,
            mpl_log_level_t::MPL_LOG_INFO => PluginLogLevel::Info,
            mpl_log_level_t::MPL_LOG_DEBUG => PluginLogLevel::Debug,
            mpl_log_level_t::MPL_LOG_TRACE => PluginLogLevel::Trace,
        }
    }
}

/// Host callback table handed to the plugin (mpl_host_api_t).
#[repr(C)]
#[derive(Clone)]
pub struct mpl_host_api_t {
    pub log: unsafe extern "C" fn(ctx: *mut c_void, level: mpl_log_level_t, msg: *const c_char),
    pub get_config: unsafe extern "C" fn(
        ctx: *mut c_void,
        key: *const c_char,
        out: *mut c_char,
        out_size: *mut u32,
    ) -> mpl_result_t,
    pub set_config: unsafe extern "C" fn(
        ctx: *mut c_void,
        key: *const c_char,
        json_value: *const c_char,
    ) -> mpl_result_t,
    pub emit_event: unsafe extern "C" fn(
        ctx: *mut c_void,
        topic: *const c_char,
        json_payload: *const c_char,
    ) -> mpl_result_t,
    pub send_message: unsafe extern "C" fn(
        ctx: *mut c_void,
        target_json: *const c_char,
        payload: *const u8,
        payload_len: u32,
    ) -> mpl_result_t,
    pub audio_state: unsafe extern "C" fn(
        ctx: *mut c_void,
        out: *mut c_char,
        out_size: *mut u32,
    ) -> mpl_result_t,
    pub connected_devices: unsafe extern "C" fn(
        ctx: *mut c_void,
        out: *mut c_char,
        out_size: *mut u32,
    ) -> mpl_result_t,
    /// Appended after `ctx` on purpose: older plugins compiled against the
    /// previous layout still see `ctx` at its original offset, so adding
    /// fields here stays ABI-compatible for them.
    pub ctx: *mut c_void,
    pub play_sound: unsafe extern "C" fn(ctx: *mut c_void, path: *const c_char) -> mpl_result_t,
    pub plugin_dir: unsafe extern "C" fn(
        ctx: *mut c_void,
        out: *mut c_char,
        out_size: *mut u32,
    ) -> mpl_result_t,
    pub register_hotkey: unsafe extern "C" fn(
        ctx: *mut c_void,
        shortcut: *const c_char,
        out_id: *mut u64,
    ) -> mpl_result_t,
    pub open_window: unsafe extern "C" fn(ctx: *mut c_void, panel_id: *const c_char) -> mpl_result_t,
    pub fs_read: unsafe extern "C" fn(
        ctx: *mut c_void,
        path: *const c_char,
        out: *mut c_char,
        out_size: *mut u32,
    ) -> mpl_result_t,
    pub fs_write: unsafe extern "C" fn(
        ctx: *mut c_void,
        path: *const c_char,
        content: *const c_char,
    ) -> mpl_result_t,
}

// The table travels inside `NativePlugin` which is `Send`; the raw `ctx`
// pointer is never dereferenced off the plugin thread.
unsafe impl Send for mpl_host_api_t {}

/// Static plugin identity (mpl_plugin_info_t).
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct mpl_plugin_info_t {
    pub abi_version: u32,
    pub api_version: u32,
    pub id: *const c_char,
    pub version: *const c_char,
}

// Pointer fields are only read right after load, on the loading thread.
unsafe impl Send for mpl_plugin_info_t {}

/// Host-side context stored as the `ctx` in `mpl_host_api_t`.
/// The plugin's host-callback calls come back into Rust through here.
pub struct NativeHostCtx {
    pub host: Arc<dyn HostApi>,
    /// Rejects host calls that plugins without the capability may not use.
    pub capabilities: Vec<String>,
}

fn has_capability(ctx: &NativeHostCtx, cap: &str) -> bool {
    ctx.capabilities.iter().any(|c| c == cap)
}

// ── Host callback shims ────────────────────────────────────────────────────

unsafe extern "C" fn shim_log(ctx: *mut c_void, level: mpl_log_level_t, msg: *const c_char) {
    unsafe {
        if msg.is_null() {
            return;
        }
        let ctx = &*(ctx as *const NativeHostCtx);
        let message = CStr::from_ptr(msg).to_string_lossy();
        ctx.host.log(level.into(), &message);
    }
}

unsafe extern "C" fn shim_get_config(
    ctx: *mut c_void,
    key: *const c_char,
    out: *mut c_char,
    out_size: *mut u32,
) -> mpl_result_t {
    unsafe {
        let ctx = &*(ctx as *const NativeHostCtx);
        let (Some(key), Some(out), Some(out_size)) =
            (key.as_ref(), out.as_mut(), out_size.as_mut())
        else {
            return mpl_result_t::MPL_ERR_INVALID_ARG;
        };
        if !has_capability(ctx, crate::manifest::capabilities::CONFIG_READ) {
            return mpl_result_t::MPL_ERR_PERMISSION;
        }
        let key_str = CStr::from_ptr(key).to_string_lossy();
        match ctx.host.get_config(&key_str) {
            Some(value) => write_json_to_buf(&value.to_string(), out, out_size),
            None => {
                *out_size = 0;
                mpl_result_t::MPL_OK
            }
        }
    }
}

unsafe extern "C" fn shim_set_config(
    ctx: *mut c_void,
    key: *const c_char,
    json_value: *const c_char,
) -> mpl_result_t {
    unsafe {
        let ctx = &*(ctx as *const NativeHostCtx);
        let (Some(key), Some(value)) = (key.as_ref(), json_value.as_ref()) else {
            return mpl_result_t::MPL_ERR_INVALID_ARG;
        };
        if !has_capability(ctx, crate::manifest::capabilities::CONFIG_WRITE) {
            return mpl_result_t::MPL_ERR_PERMISSION;
        }
        let key_str = CStr::from_ptr(key).to_string_lossy();
        let value_str = CStr::from_ptr(value).to_string_lossy();
        match serde_json::from_str(&value_str) {
            Ok(json) => match ctx.host.set_config(&key_str, json) {
                Ok(()) => mpl_result_t::MPL_OK,
                Err(_) => mpl_result_t::MPL_ERR_RUNTIME,
            },
            Err(_) => mpl_result_t::MPL_ERR_INVALID_ARG,
        }
    }
}

unsafe extern "C" fn shim_emit_event(
    ctx: *mut c_void,
    topic: *const c_char,
    json_payload: *const c_char,
) -> mpl_result_t {
    unsafe {
        let ctx = &*(ctx as *const NativeHostCtx);
        let (Some(topic), Some(payload)) = (topic.as_ref(), json_payload.as_ref()) else {
            return mpl_result_t::MPL_ERR_INVALID_ARG;
        };
        if !has_capability(ctx, crate::manifest::capabilities::EVENT_EMIT) {
            return mpl_result_t::MPL_ERR_PERMISSION;
        }
        let topic_str = CStr::from_ptr(topic).to_string_lossy();
        let payload_str = CStr::from_ptr(payload).to_string_lossy();
        let json = serde_json::from_str(&payload_str).unwrap_or(serde_json::Value::Null);
        match ctx.host.emit_event(&topic_str, json) {
            Ok(()) => mpl_result_t::MPL_OK,
            Err(_) => mpl_result_t::MPL_ERR_RUNTIME,
        }
    }
}

unsafe extern "C" fn shim_send_message(
    ctx: *mut c_void,
    target_json: *const c_char,
    payload: *const u8,
    payload_len: u32,
) -> mpl_result_t {
    unsafe {
        let ctx = &*(ctx as *const NativeHostCtx);
        let Some(target_json) = target_json.as_ref() else {
            return mpl_result_t::MPL_ERR_INVALID_ARG;
        };
        if !has_capability(ctx, crate::manifest::capabilities::MESSAGE_SEND) {
            return mpl_result_t::MPL_ERR_PERMISSION;
        }
        let target_str = CStr::from_ptr(target_json).to_string_lossy();
        let target: MessageTarget = match serde_json::from_str(&target_str) {
            Ok(t) => t,
            Err(_) => return mpl_result_t::MPL_ERR_INVALID_ARG,
        };
        let bytes = if payload.is_null() || payload_len == 0 {
            Vec::new()
        } else {
            std::slice::from_raw_parts(payload, payload_len as usize).to_vec()
        };
        match ctx.host.send_message(target, bytes) {
            Ok(()) => mpl_result_t::MPL_OK,
            Err(_) => mpl_result_t::MPL_ERR_RUNTIME,
        }
    }
}

unsafe extern "C" fn shim_audio_state(
    ctx: *mut c_void,
    out: *mut c_char,
    out_size: *mut u32,
) -> mpl_result_t {
    unsafe {
        let ctx = &*(ctx as *const NativeHostCtx);
        let (Some(out), Some(out_size)) = (out.as_mut(), out_size.as_mut()) else {
            return mpl_result_t::MPL_ERR_INVALID_ARG;
        };
        if !has_capability(ctx, crate::manifest::capabilities::AUDIO_STATE) {
            return mpl_result_t::MPL_ERR_PERMISSION;
        }
        let json = serde_json::to_string(&ctx.host.audio_state()).unwrap_or_else(|_| "{}".into());
        write_json_to_buf(&json, out, out_size)
    }
}

unsafe extern "C" fn shim_play_sound(ctx: *mut c_void, path: *const c_char) -> mpl_result_t {
    unsafe {
        let ctx = &*(ctx as *const NativeHostCtx);
        if !has_capability(ctx, crate::manifest::capabilities::AUDIO_PLAY) {
            return mpl_result_t::MPL_ERR_PERMISSION;
        }
        let path = if path.is_null() {
            String::new()
        } else {
            CStr::from_ptr(path).to_string_lossy().into_owned()
        };
        match ctx.host.play_sound(&path) {
            Ok(()) => mpl_result_t::MPL_OK,
            Err(_) => mpl_result_t::MPL_ERR_RUNTIME,
        }
    }
}

unsafe extern "C" fn shim_register_hotkey(
    ctx: *mut c_void,
    shortcut: *const c_char,
    out_id: *mut u64,
) -> mpl_result_t {
    unsafe {
        let ctx = &*(ctx as *const NativeHostCtx);
        let Some(out_id) = out_id.as_mut() else {
            return mpl_result_t::MPL_ERR_INVALID_ARG;
        };
        let shortcut_str = if shortcut.is_null() {
            String::new()
        } else {
            CStr::from_ptr(shortcut).to_string_lossy().into_owned()
        };
        match ctx.host.register_hotkey(&shortcut_str) {
            Ok(id) => {
                *out_id = id;
                mpl_result_t::MPL_OK
            }
            Err(_) => mpl_result_t::MPL_ERR_RUNTIME,
        }
    }
}

unsafe extern "C" fn shim_open_window(
    ctx: *mut c_void,
    panel_id: *const c_char,
) -> mpl_result_t {
    unsafe {
        let ctx = &*(ctx as *const NativeHostCtx);
        let panel = if panel_id.is_null() {
            String::new()
        } else {
            CStr::from_ptr(panel_id).to_string_lossy().into_owned()
        };
        match ctx.host.open_window(&panel) {
            Ok(()) => mpl_result_t::MPL_OK,
            Err(_) => mpl_result_t::MPL_ERR_RUNTIME,
        }
    }
}

unsafe extern "C" fn shim_fs_read(
    ctx: *mut c_void,
    path: *const c_char,
    out: *mut c_char,
    out_size: *mut u32,
) -> mpl_result_t {
    unsafe {
        let ctx = &*(ctx as *const NativeHostCtx);
        if !has_capability(ctx, crate::manifest::capabilities::FS_READ) {
            return mpl_result_t::MPL_ERR_PERMISSION;
        }
        let path = if path.is_null() {
            String::new()
        } else {
            CStr::from_ptr(path).to_string_lossy().into_owned()
        };
        match ctx.host.fs_read(&path) {
            Ok(text) => write_json_to_buf(&text, out, out_size),
            Err(_) => mpl_result_t::MPL_ERR_RUNTIME,
        }
    }
}

unsafe extern "C" fn shim_fs_write(
    ctx: *mut c_void,
    path: *const c_char,
    content: *const c_char,
) -> mpl_result_t {
    unsafe {
        let ctx = &*(ctx as *const NativeHostCtx);
        if !has_capability(ctx, crate::manifest::capabilities::FS_WRITE) {
            return mpl_result_t::MPL_ERR_PERMISSION;
        }
        let path = if path.is_null() {
            String::new()
        } else {
            CStr::from_ptr(path).to_string_lossy().into_owned()
        };
        let content = if content.is_null() {
            String::new()
        } else {
            CStr::from_ptr(content).to_string_lossy().into_owned()
        };
        match ctx.host.fs_write(&path, &content) {
            Ok(()) => mpl_result_t::MPL_OK,
            Err(_) => mpl_result_t::MPL_ERR_RUNTIME,
        }
    }
}

unsafe extern "C" fn shim_plugin_dir(
    ctx: *mut c_void,
    out: *mut c_char,
    out_size: *mut u32,
) -> mpl_result_t {
    unsafe {
        let ctx = &*(ctx as *const NativeHostCtx);
        let (Some(out), Some(out_size)) = (out.as_mut(), out_size.as_mut()) else {
            return mpl_result_t::MPL_ERR_INVALID_ARG;
        };
        let dir = ctx.host.plugin_dir();
        write_json_to_buf(&dir, out, out_size)
    }
}

unsafe extern "C" fn shim_connected_devices(
    ctx: *mut c_void,
    out: *mut c_char,
    out_size: *mut u32,
) -> mpl_result_t {
    unsafe {
        let ctx = &*(ctx as *const NativeHostCtx);
        let (Some(out), Some(out_size)) = (out.as_mut(), out_size.as_mut()) else {
            return mpl_result_t::MPL_ERR_INVALID_ARG;
        };
        if !has_capability(ctx, crate::manifest::capabilities::DEVICE_LIST) {
            return mpl_result_t::MPL_ERR_PERMISSION;
        }
        let json =
            serde_json::to_string(&ctx.host.connected_devices()).unwrap_or_else(|_| "[]".into());
        write_json_to_buf(&json, out, out_size)
    }
}

/// Write a NUL-terminated string into `out`, following the out/out_size
/// contract (see the header). Returns MPL_ERR_BUFFER_TOO_SMALL when the
/// plugin buffer is too small, with the required size in *out_size.
fn write_json_to_buf(text: &str, out: *mut c_char, out_size: *mut u32) -> mpl_result_t {
    unsafe {
        let needed = text.len() as u32 + 1;
        if *out_size < needed {
            *out_size = needed;
            return mpl_result_t::MPL_ERR_BUFFER_TOO_SMALL;
        }
        let bytes = text.as_bytes();
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), out as *mut u8, bytes.len());
        *out.add(bytes.len()) = 0;
        *out_size = bytes.len() as u32;
        mpl_result_t::MPL_OK
    }
}

/// Build the host table for a plugin instance.
pub fn host_table_for(ctx: Arc<NativeHostCtx>) -> mpl_host_api_t {
    let ctx_ptr = Arc::into_raw(ctx);
    mpl_host_api_t {
        log: shim_log,
        get_config: shim_get_config,
        set_config: shim_set_config,
        emit_event: shim_emit_event,
        send_message: shim_send_message,
        audio_state: shim_audio_state,
        connected_devices: shim_connected_devices,
        ctx: ctx_ptr as *mut c_void,
        play_sound: shim_play_sound,
        plugin_dir: shim_plugin_dir,
        register_hotkey: shim_register_hotkey,
        open_window: shim_open_window,
        fs_read: shim_fs_read,
        fs_write: shim_fs_write,
    }
}

/// Recover the Arc dropped by `host_table_for` (called on plugin deinit).
pub unsafe fn release_host_ctx(ctx: *mut c_void) {
    unsafe {
        if !ctx.is_null() {
            drop(Arc::from_raw(ctx as *const NativeHostCtx));
        }
    }
}

/// Result-code mapping used by the native bridge.
pub fn result_from_code(code: mpl_result_t, context: &str) -> PluginResult<()> {
    match code {
        mpl_result_t::MPL_OK => Ok(()),
        mpl_result_t::MPL_ERR_NOT_IMPLEMENTED => {
            Err(PluginError::Runtime(format!("{context}: not implemented")))
        }
        mpl_result_t::MPL_ERR_INVALID_ARG => {
            Err(PluginError::Runtime(format!("{context}: invalid argument")))
        }
        mpl_result_t::MPL_ERR_RUNTIME => Err(PluginError::Runtime(format!(
            "{context}: plugin runtime error"
        ))),
        mpl_result_t::MPL_ERR_BUFFER_TOO_SMALL => {
            Err(PluginError::Runtime(format!("{context}: buffer too small")))
        }
        mpl_result_t::MPL_ERR_PERMISSION => Err(PluginError::PermissionDenied(context.to_string())),
    }
}

/// Helpers for building null-terminated strings for FFI calls.
pub fn cstr(value: &str) -> PluginResult<CString> {
    CString::new(value).map_err(|e| PluginError::Runtime(format!("invalid string: {e}")))
}
