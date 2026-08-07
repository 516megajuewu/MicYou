//! MicYou native plugin example — a configurable gain DSP node
//!
//! 这是一个完整的原生 DSP 插件示例
//! 它实现 `micyou_plugin_abi.h` 中的 ABI：版本握手、init/deinit、process
//! 并通过 host 回调读取自己的配置（gain），把输入音频按增益缩放
//!
//! 构建：`cargo build --release`，产物 `target/release/libmicyou_example_native_gain.so`
//! 安装：把 `libmicyou_example_native_gain.{so,dylib,dll}` 与 `plugin.json`
//!       放进 `~/.config/micyou/plugins/dev.micyou.example.gain/` 目录

#![allow(non_camel_case_types)]

use std::ffi::{c_char, c_void, CStr};

// ── ABI 类型（与 include/micyou_plugin_abi.h 严格对应）─────────────────────

const MPL_ABI_VERSION: u32 = 1;
const MPL_API_VERSION: u32 = 1;

#[repr(C)]
#[derive(PartialEq, Eq)]
pub enum mpl_result_t {
    MPL_OK = 0,
    MPL_ERR_NOT_IMPLEMENTED = 1,
    MPL_ERR_INVALID_ARG = 2,
    MPL_ERR_RUNTIME = 3,
    MPL_ERR_BUFFER_TOO_SMALL = 4,
    MPL_ERR_PERMISSION = 5,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub enum mpl_log_level_t {
    MPL_LOG_ERROR = 0,
    MPL_LOG_WARN = 1,
    MPL_LOG_INFO = 2,
    MPL_LOG_DEBUG = 3,
    MPL_LOG_TRACE = 4,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct mpl_host_api_t {
    pub log: unsafe extern "C" fn(*mut c_void, mpl_log_level_t, *const c_char),
    pub get_config: unsafe extern "C" fn(
        *mut c_void,
        *const c_char,
        *mut c_char,
        *mut u32,
    ) -> mpl_result_t,
    pub set_config: unsafe extern "C" fn(*mut c_void, *const c_char, *const c_char) -> mpl_result_t,
    pub emit_event: unsafe extern "C" fn(*mut c_void, *const c_char, *const c_char) -> mpl_result_t,
    pub send_message: unsafe extern "C" fn(
        *mut c_void,
        *const c_char,
        *const u8,
        u32,
    ) -> mpl_result_t,
    pub audio_state: unsafe extern "C" fn(*mut c_void, *mut c_char, *mut u32) -> mpl_result_t,
    pub connected_devices: unsafe extern "C" fn(*mut c_void, *mut c_char, *mut u32) -> mpl_result_t,
    pub ctx: *mut c_void,
}

#[repr(C)]
pub struct mpl_plugin_info_t {
    pub abi_version: u32,
    pub api_version: u32,
    pub id: *const c_char,
    pub version: *const c_char,
}

// 静态 info 只读，raw 指针仅在加载线程读取
unsafe impl Sync for mpl_plugin_info_t {}

// ── 插件状态 ───────────────────────────────────────────────────────────────

const PLUGIN_ID: &[u8] = b"dev.micyou.example.gain\0";
const PLUGIN_VERSION: &[u8] = b"1.0.0\0";
const CONFIG_KEY: &[u8] = b"gain\0"; // 配置键：增益（倍率，默认 2.0）

static mut HOST: Option<mpl_host_api_t> = None;
static mut GAIN: f64 = 2.0;

/// 防止 panic 跨 FFI 边界传播（未定义行为），统一转成运行时错误码
fn guard<F: FnOnce() -> mpl_result_t + std::panic::UnwindSafe>(f: F) -> mpl_result_t {
    std::panic::catch_unwind(f).unwrap_or(mpl_result_t::MPL_ERR_RUNTIME)
}

// ── 必需入口 ───────────────────────────────────────────────────────────────

/// 返回指向静态 info 的指针（库生命周期内有效）
#[unsafe(no_mangle)]
pub extern "C" fn micyou_plugin_info() -> *const mpl_plugin_info_t {
    static INFO: mpl_plugin_info_t = mpl_plugin_info_t {
        abi_version: MPL_ABI_VERSION,
        api_version: MPL_API_VERSION,
        id: PLUGIN_ID.as_ptr() as *const c_char,
        version: PLUGIN_VERSION.as_ptr() as *const c_char,
    };
    &INFO
}

/// 初始化：保存 host 回调表，并从配置读取增益
/// # Safety
/// `host` 必须指向有效的 mpl_host_api_t，且生命周期长于插件
#[unsafe(no_mangle)]
pub unsafe extern "C" fn micyou_plugin_init(host: *const mpl_host_api_t) -> mpl_result_t {
    guard(|| {
        if host.is_null() || (*host).log as usize == 0 {
            return mpl_result_t::MPL_ERR_INVALID_ARG;
        }
        unsafe {
            HOST = Some(*host);
            // 读取配置：get_config("gain") -> "2.0"
            let mut buf = [0i8; 64];
            let mut size: u32 = buf.len() as u32;
            let code = ((*host).get_config)(
                (*host).ctx,
                CONFIG_KEY.as_ptr() as *const c_char,
                buf.as_mut_ptr(),
                &mut size,
            );
            if code == mpl_result_t::MPL_OK && size > 0 {
                let value = CStr::from_ptr(buf.as_ptr()).to_string_lossy().to_string();
                if let Ok(g) = value.parse::<f64>() {
                    GAIN = g;
                }
            }
            ((*host).log)(
                (*host).ctx,
                mpl_log_level_t::MPL_LOG_INFO,
                b"native-gain initialized\0".as_ptr() as *const c_char,
            );
        }
        mpl_result_t::MPL_OK
    })
}

/// 反初始化
/// # Safety
/// 无额外要求
#[unsafe(no_mangle)]
pub unsafe extern "C" fn micyou_plugin_deinit() {
    unsafe {
        HOST = None;
    }
}

// ── 可选入口 ───────────────────────────────────────────────────────────────

/// 实时 DSP：原地缩放 `samples` 个交错 f32 采样
/// 返回 0=已处理，1=旁路（宿主保留输入）
/// 实时安全要求：不得调用阻塞 host API，不得分配内存
/// # Safety
/// `data` 必须指向 `samples` 个 f32，`bypass` 必须有效
#[unsafe(no_mangle)]
pub unsafe extern "C" fn micyou_plugin_process(
    data: *mut f32,
    samples: u32,
    _channels: u32,
    _queued_ms: f64,
    bypass: *mut u32,
) -> mpl_result_t {
    guard(|| {
        if data.is_null() || bypass.is_null() {
            return mpl_result_t::MPL_ERR_INVALID_ARG;
        }
        let gain = unsafe { GAIN };
        if gain <= 0.0 {
            unsafe { *bypass = 1 };
            return mpl_result_t::MPL_OK;
        }
        unsafe {
            for i in 0..samples as usize {
                *data.add(i) *= gain as f32;
            }
            *bypass = 0;
        }
        mpl_result_t::MPL_OK
    })
}

/// 事件通知（可选入口，本示例不处理）
#[unsafe(no_mangle)]
pub extern "C" fn micyou_plugin_handle_event(
    _type_name: *const c_char,
    _json: *const c_char,
) -> mpl_result_t {
    mpl_result_t::MPL_OK
}

/// 跨端消息（可选入口，本示例不处理）
#[unsafe(no_mangle)]
pub extern "C" fn micyou_plugin_handle_message(
    _source: *const c_char,
    _topic: *const c_char,
    _payload: *const u8,
    _payload_len: u32,
) -> mpl_result_t {
    mpl_result_t::MPL_OK
}
