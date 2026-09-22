// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    #[cfg(target_os = "linux")]
    work_around_webkitgtk_dmabuf();

    cled_desktop_lib::run()
}

/// WebKitGTK's DMA-BUF renderer crashes on some Wayland setups (notably NVIDIA's proprietary
/// driver) with "Error 71 (Protocol error) dispatching to Wayland display". Our UI is light
/// enough that the fallback renderer costs nothing noticeable. Users can opt back in by
/// setting `WEBKIT_DISABLE_DMABUF_RENDERER=0`.
#[cfg(target_os = "linux")]
fn work_around_webkitgtk_dmabuf() {
    const VAR: &str = "WEBKIT_DISABLE_DMABUF_RENDERER";
    if std::env::var_os(VAR).is_none() {
        // SAFETY: called at the very start of `main`, before any other threads exist.
        unsafe { std::env::set_var(VAR, "1") };
    }
}
