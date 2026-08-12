#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(target_os = "linux")]
fn configure_renderer() {
    let software_requested = std::env::args_os().any(|arg| arg == "--software-rendering")
        || std::env::var("MICYOU_RENDERER")
            .is_ok_and(|value| value.eq_ignore_ascii_case("software"));

    if software_requested {
        // Compatibility fallback for drivers/compositors where WebKitGTK's
        // accelerated DMA-BUF path produces a blank window or crashes.
        std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
        std::env::set_var("WEBKIT_DISABLE_COMPOSITING_MODE", "1");
        eprintln!("[Renderer] Software rendering fallback enabled");
    }
}

fn main() {
    #[cfg(target_os = "linux")]
    configure_renderer();

    tauri_app_lib::run()
}
