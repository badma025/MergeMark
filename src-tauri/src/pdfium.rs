use std::path::PathBuf;
use tauri::path::BaseDirectory;
use tauri::Manager;
use pdfium_render::prelude::*;

/// Get the path to the bundled PDFium library for the current platform
pub fn get_bundled_pdfium_path(app_handle: &tauri::AppHandle) -> Result<PathBuf, String> {
    let resource_dir = app_handle
        .path()
        .resolve("resources/pdfium", BaseDirectory::Resource)
        .map_err(|e| format!("Failed to resolve resource dir: {}", e))?;

    let lib_name = if cfg!(target_os = "windows") {
        "win-x64/pdfium.dll"
    } else if cfg!(target_os = "macos") {
        "mac-universal/libpdfium.dylib"
    } else {
        "linux-x64/libpdfium.so"
    };

    let lib_path = resource_dir.join(lib_name);

    if !lib_path.exists() {
        // Try fallback: maybe using a different naming convention
        let fallback = resource_dir.join(if cfg!(target_os = "windows") {
            "pdfium.dll"
        } else if cfg!(target_os = "macos") {
            "libpdfium.dylib"
        } else {
            "libpdfium.so"
        });

        if fallback.exists() {
            return Ok(fallback);
        }

        return Err(format!(
            "PDFium library not found at: {} (also checked: {})",
            lib_path.display(),
            fallback.display()
        ));
    }

    Ok(lib_path)
}

/// Initialize Pdfium by binding to the bundled library
pub fn init_pdfium(app_handle: &tauri::AppHandle) -> Result<Pdfium, String> {
    let lib_path = get_bundled_pdfium_path(app_handle)?;

    // Get the directory containing the PDFium library
    let lib_dir = lib_path.parent().ok_or("PDFium library path has no parent directory")?;
    let lib_dir_str = lib_dir
        .to_str()
        .ok_or("PDFium directory path contains invalid UTF-8")?;

    // Use pdfium_platform_library_name_at_path to get the platform-specific library name
    // Then bind to it, with fallback to system library
    let bindings = Pdfium::bind_to_library(
        Pdfium::pdfium_platform_library_name_at_path(lib_dir_str),
    )
    .or_else(|_| Pdfium::bind_to_system_library())
    .map_err(|e| format!("Failed to bind PDFium from {}: {}", lib_dir.display(), e))?;

    // Create Pdfium instance with the bindings
    let pdfium = Pdfium::new(bindings);

    Ok(pdfium)
}

/// Initialize Pdfium for tests (without AppHandle) - falls back to system library
#[cfg(test)]
pub fn init_pdfium_for_test() -> Result<Pdfium, String> {
    let bindings = Pdfium::bind_to_system_library()
        .map_err(|e| format!("Failed to bind system PDFium: {}", e))?;
    Ok(Pdfium::new(bindings))
}