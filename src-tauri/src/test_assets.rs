#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    /// Simulates the frontend `convertFileSrc` behavior for the asset protocol
    fn convert_file_src(path: &str) -> String {
        #[cfg(target_os = "windows")]
        {
            // Windows format: asset://localhost/C:/path/to/file
            // Ensure the path starts with a drive letter, and forward slashes
            let normalized = path.replace("\\", "/");
            let encoded = urlencoding::encode(&normalized).into_owned().replace("%2F", "/").replace("%3A", ":");
            format!("asset://localhost/{}", encoded)
        }
        #[cfg(not(target_os = "windows"))]
        {
            // Unix format: asset://localhost/path/to/file
            let encoded = urlencoding::encode(path).into_owned().replace("%2F", "/");
            format!("asset://localhost{}", encoded)
        }
    }

    #[test]
    fn test_asset_protocol_resolution() {
        // 1. Create a dummy diagrams directory and write a dummy PNG
        let temp_dir = std::env::temp_dir();
        let diagrams_dir = temp_dir.join("MergeMark_Test").join("diagrams");
        fs::create_dir_all(&diagrams_dir).unwrap();

        let dummy_png_path = diagrams_dir.join("test_diagram.png");
        fs::write(&dummy_png_path, b"dummy png content").unwrap();

        // 2. Resolve its path via simulated Tauri asset URL mechanism
        let path_str = dummy_png_path.to_string_lossy();
        let asset_url = convert_file_src(&path_str);

        // 3. Validate protocol string format
        assert!(asset_url.starts_with("asset://localhost/"));
        assert!(asset_url.ends_with("/diagrams/test_diagram.png"));

        // Cleanup
        let _ = fs::remove_file(&dummy_png_path);
    }
}
