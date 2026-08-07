//! MicYou native plugin example — an RMS noise gate DSP node
//!
//! 演示能力：新增一个降噪引擎（噪声门）
//!   对低于阈值的背景噪音按 depth 衰减，高于阈值正常通过
//!   使用 attack/release 包络平滑，避免门限切换产生咔哒声
//!   process 全程无分配、无 host 调用，满足实时安全要求
//!
//! 构建：`cargo build --release`，产物 target/release/libmicyou_example_native_noisegate.so
//! 安装：把 .so 与 plugin.json 放进 ~/.config/micyou/plugins/dev.micyou.example.noisegate/
//! 使用：启用后进 DSP 链（AEC 之后），手机连上后低音量环境音会被压掉

#![allow(non_camel_case_types)]

use std::ffi::{c_char, c_void, CStr};
use std::sync::atomic::{AtomicU64, Ordering};

const MPL_ABI_VERSION: u32 = 1;
const MPL_API_VERSION: u32 = 1;

#[repr(C)]
#[derive(PartialEq, Eq, Clone, Copy)]
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
    pub get_config:
        unsafe extern "C" fn(*mut c_void, *const c_char, *mut c_char, *mut u32) -> mpl_result_t,
    pub set_config: unsafe extern "C" fn(*mut c_void, *const c_char, *const c_char) -> mpl_result_t,
    pub emit_event: unsafe extern "C" fn(*mut c_void, *const c_char, *const c_char) -> mpl_result_t,
    pub send_message:
        unsafe extern "C" fn(*mut c_void, *const c_char, *const u8, u32) -> mpl_result_t,
    pub audio_state: unsafe extern "C" fn(*mut c_void, *mut c_char, *mut u32) -> mpl_result_t,
    pub connected_devices: unsafe extern "C" fn(*mut c_void, *mut c_char, *mut u32) -> mpl_result_t,
    pub ctx: *mut c_void,
    pub play_sound: unsafe extern "C" fn(*mut c_void, *const c_char) -> mpl_result_t,
    pub plugin_dir: unsafe extern "C" fn(*mut c_void, *mut c_char, *mut u32) -> mpl_result_t,
}

#[repr(C)]
pub struct mpl_plugin_info_t {
    pub abi_version: u32,
    pub api_version: u32,
    pub id: *const c_char,
    pub version: *const c_char,
}

unsafe impl Sync for mpl_plugin_info_t {}

const PLUGIN_ID: &[u8] = b"dev.micyou.example.noisegate\0";
const PLUGIN_VERSION: &[u8] = b"1.0.0\0";

static mut HOST: Option<mpl_host_api_t> = None;

// 配置：threshold(dB)、depth(dB)、attack/release(ms)
// 用 AtomicU64 存 f64 位模式，process 线程无锁读取
static THRESHOLD: AtomicU64 = AtomicU64::new(f64::to_bits(-40.0));
static DEPTH: AtomicU64 = AtomicU64::new(f64::to_bits(20.0));
static ATTACK: AtomicU64 = AtomicU64::new(f64::to_bits(5.0));
static RELEASE: AtomicU64 = AtomicU64::new(f64::to_bits(150.0));

/// 当前包络增益（线性），process 时独占修改
static mut ENVELOPE: f64 = 1.0;

fn guard<F: FnOnce() -> mpl_result_t + std::panic::UnwindSafe>(f: F) -> mpl_result_t {
    std::panic::catch_unwind(f).unwrap_or(mpl_result_t::MPL_ERR_RUNTIME)
}

unsafe fn log_info(msg: &str) {
    unsafe {
        if let Some(h) = HOST {
            if let Ok(c) = std::ffi::CString::new(msg) {
                ((h.log)(h.ctx, mpl_log_level_t::MPL_LOG_INFO, c.as_ptr()));
            }
        }
    }
}

fn read_f64(atom: &AtomicU64, default: f64) -> f64 {
    let bits = atom.load(Ordering::Relaxed);
    let v = f64::from_bits(bits);
    if v.is_finite() {
        v
    } else {
        default
    }
}

/// 读取配置（key -> 设置对应原子），失败保持默认
unsafe fn load_config() {
    unsafe {
        let Some(h) = HOST else { return };
        for (key, atom, _default) in [
            (b"threshold\0" as &[u8], &THRESHOLD, -40.0f64),
            (b"depth\0" as &[u8], &DEPTH, 20.0f64),
            (b"attackMs\0" as &[u8], &ATTACK, 5.0f64),
            (b"releaseMs\0" as &[u8], &RELEASE, 150.0f64),
        ] {
            let mut buf = [0i8; 128];
            let mut size: u32 = buf.len() as u32;
            let code = (h.get_config)(
                h.ctx,
                key.as_ptr() as *const c_char,
                buf.as_mut_ptr(),
                &mut size,
            );
            if code == mpl_result_t::MPL_OK && size > 0 {
                let text = CStr::from_ptr(buf.as_ptr()).to_string_lossy().to_string();
                if let Ok(v) = text.trim().parse::<f64>() {
                    if v.is_finite() {
                        atom.store(v.to_bits(), Ordering::Relaxed);
                    }
                }
            }
        }
    }
}

// ── 必需入口 ───────────────────────────────────────────────────────────────

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

/// 初始化：保存 host、读取降噪参数
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
            load_config();
            log_info("noisegate initialized");
        }
        mpl_result_t::MPL_OK
    })
}

/// 反初始化
/// # Safety
/// 无额外要求
#[unsafe(no_mangle)]
#[allow(static_mut_refs)]
pub unsafe extern "C" fn micyou_plugin_deinit() {
    unsafe {
        HOST = None;
        ENVELOPE = 1.0;
    }
}

// ── 可选入口 ───────────────────────────────────────────────────────────────

/// 实时 DSP：RMS 噪声门
/// 逐帧（默认 480 样本）计算 RMS，低于阈值时按 depth 衰减，
/// attack/release 包络平滑过渡
/// 实时安全：无分配、无 host 调用、纯算术
/// # Safety
/// `data` 必须指向 `samples` 个 f32，`bypass` 必须有效
#[unsafe(no_mangle)]
#[allow(static_mut_refs)]
pub unsafe extern "C" fn micyou_plugin_process(
    data: *mut f32,
    samples: u32,
    channels: u32,
    _queued_ms: f64,
    bypass: *mut u32,
) -> mpl_result_t {
    guard(|| {
        if data.is_null() || bypass.is_null() || samples == 0 || channels == 0 {
            return mpl_result_t::MPL_ERR_INVALID_ARG;
        }
        let threshold = read_f64(&THRESHOLD, -40.0);
        let depth = read_f64(&DEPTH, 20.0).clamp(0.0, 60.0);
        let attack = read_f64(&ATTACK, 5.0).max(0.1);
        let release = read_f64(&RELEASE, 150.0).max(1.0);
        let sample_rate = 48_000.0f64;

        let frames = samples as usize / channels.max(1) as usize;
        let mut env = unsafe { ENVELOPE };

        for frame in 0..frames {
            let base = frame * channels as usize;
            // 帧 RMS
            let mut sum = 0.0f64;
            for ch in 0..channels as usize {
                let v = *unsafe { data.add(base + ch) } as f64;
                sum += v * v;
            }
            let rms = (sum / channels as f64).sqrt();
            let db = 20.0 * (rms + 1e-9).log10();
            // 目标增益：低于阈值 -> 按 depth 衰减（线性域）
            let target = if db < threshold {
                (-depth / 20.0).exp10()
            } else {
                1.0
            };
            // 包络：信号低于阈值时用 attack 快速关，恢复时用 release 慢开
            let coeff = if db < threshold {
                (-1.0 / (attack * 0.001 * sample_rate)).exp()
            } else {
                (-1.0 / (release * 0.001 * sample_rate)).exp()
            };
            env = target + (env - target) * coeff;
            let gain = env as f32;
            for ch in 0..channels as usize {
                *unsafe { data.add(base + ch) } *= gain;
            }
        }

        unsafe {
            ENVELOPE = env;
            *bypass = 0;
        }
        mpl_result_t::MPL_OK
    })
}

/// 事件通知（本示例不处理）
#[unsafe(no_mangle)]
pub extern "C" fn micyou_plugin_handle_event(
    _type_name: *const c_char,
    _json: *const c_char,
) -> mpl_result_t {
    mpl_result_t::MPL_OK
}

/// 跨端消息（本示例不处理）
#[unsafe(no_mangle)]
pub extern "C" fn micyou_plugin_handle_message(
    _source: *const c_char,
    _topic: *const c_char,
    _payload: *const u8,
    _payload_len: u32,
) -> mpl_result_t {
    mpl_result_t::MPL_OK
}

/// f64::exp10（稳定版 rustc 无此方法，自行实现）
trait Exp10 {
    fn exp10(self) -> f64;
}
impl Exp10 for f64 {
    fn exp10(self) -> f64 {
        (self * std::f64::consts::LN_10).exp()
    }
}
