//! MicYou native plugin example — queries host + system APIs
//!
//! 演示能力：
//!   1 调用宿主 API：audio_state（音频流状态）、connected_devices（设备
//!     列表）、plugin_dir（插件目录）
//!   2 直接调用系统 API：读取 /proc 内核版本与内存信息（Linux）
//!   3 把结果汇总为一条报告日志，可在插件页日志区查看
//!
//! 构建：`cargo build --release`
//! 安装：.so + plugin.json -> ~/.config/micyou/plugins/dev.micyou.example.systeminfo/
//! 使用：启用后打开插件日志即可看到报告

#![allow(non_camel_case_types)]

use std::ffi::{c_char, c_void, CStr, CString};

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

const PLUGIN_ID: &[u8] = b"dev.micyou.example.systeminfo\0";
const PLUGIN_VERSION: &[u8] = b"1.0.0\0";

static mut HOST: Option<mpl_host_api_t> = None;

fn guard<F: FnOnce() -> mpl_result_t + std::panic::UnwindSafe>(f: F) -> mpl_result_t {
    std::panic::catch_unwind(f).unwrap_or(mpl_result_t::MPL_ERR_RUNTIME)
}

/// 调用 host 的 JSON 输出类 API（audio_state / connected_devices / plugin_dir）
unsafe fn host_query(
    f: unsafe extern "C" fn(*mut c_void, *mut c_char, *mut u32) -> mpl_result_t,
) -> String {
    unsafe {
        let Some(h) = HOST else { return String::new() };
        let mut buf = [0i8; 16384];
        let mut size: u32 = buf.len() as u32;
        let code = f(h.ctx, buf.as_mut_ptr(), &mut size);
        if code == mpl_result_t::MPL_OK && size > 0 {
            CStr::from_ptr(buf.as_ptr()).to_string_lossy().to_string()
        } else {
            String::new()
        }
    }
}

unsafe fn log_info(msg: &str) {
    unsafe {
        if let Some(h) = HOST {
            if let Ok(c) = CString::new(msg) {
                ((h.log)(h.ctx, mpl_log_level_t::MPL_LOG_INFO, c.as_ptr()));
            }
        }
    }
}

/// 直接读取系统 API（Linux /proc 系；其他平台返回占位信息）
fn system_report() -> String {
    #[cfg(target_os = "linux")]
    {
        let osrelease = std::fs::read_to_string("/proc/sys/kernel/osrelease")
            .unwrap_or_default()
            .trim()
            .to_string();
        let meminfo = std::fs::read_to_string("/proc/meminfo").unwrap_or_default();
        let mem_total = meminfo
            .lines()
            .find(|l| l.starts_with("MemTotal:"))
            .unwrap_or("")
            .trim()
            .to_string();
        let hostname = std::fs::read_to_string("/proc/sys/kernel/hostname")
            .unwrap_or_default()
            .trim()
            .to_string();
        format!(
            "system: kernel={osrelease}, host={hostname}, {mem_total}"
        )
    }
    #[cfg(not(target_os = "linux"))]
    {
        format!(
            "system: os={}",
            std::env::consts::OS
        )
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

/// 初始化：采集宿主与系统信息并输出报告日志
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
            let dir = host_query((*host).plugin_dir);
            let audio = host_query((*host).audio_state);
            let devices = host_query((*host).connected_devices);
            let report = format!(
                "system-info: dir={dir} | audio={audio} | devices={devices} | {}",
                system_report()
            );
            log_info(&report);
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

// ── 可选入口（本示例不处理） ───────────────────────────────────────────────

#[unsafe(no_mangle)]
pub extern "C" fn micyou_plugin_handle_event(
    _type_name: *const c_char,
    _json: *const c_char,
) -> mpl_result_t {
    mpl_result_t::MPL_OK
}

#[unsafe(no_mangle)]
pub extern "C" fn micyou_plugin_handle_message(
    _source: *const c_char,
    _topic: *const c_char,
    _payload: *const u8,
    _payload_len: u32,
) -> mpl_result_t {
    mpl_result_t::MPL_OK
}
