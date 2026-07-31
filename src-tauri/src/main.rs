// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(target_os = "linux")]
fn configure_linux_graphics() {
    let is_wayland = std::env::var("XDG_SESSION_TYPE")
        .map(|value| value.eq_ignore_ascii_case("wayland"))
        .unwrap_or(false);
    let has_nvidia = std::path::Path::new("/sys/module/nvidia").exists()
        || std::path::Path::new("/dev/nvidiactl").exists();
    let user_set_explicit_sync = std::env::var_os("__NV_DISABLE_EXPLICIT_SYNC").is_some();
    let user_set_dmabuf = std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_some();

    if is_wayland && has_nvidia && !user_set_explicit_sync && !user_set_dmabuf {
        // Must run before GTK/WebKit initialization.
        unsafe {
            std::env::set_var("__NV_DISABLE_EXPLICIT_SYNC", "1");
        }
    }
}

fn main() {
    #[cfg(target_os = "linux")]
    configure_linux_graphics();

    mergemark_lib::run()
}
