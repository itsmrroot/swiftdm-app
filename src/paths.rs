//! Where SwiftDM keeps its things.

use std::path::PathBuf;

/// The app's own folder: the download list and the helper programs.
/// `%APPDATA%\SwiftDM`, `~/Library/Application Support/SwiftDM`,
/// `~/.local/share/swiftdm`.
pub fn data_dir() -> PathBuf {
    let base = dirs::data_dir().unwrap_or_else(std::env::temp_dir);
    if cfg!(any(windows, target_os = "macos")) { base.join("SwiftDM") } else { base.join("swiftdm") }
}

/// The helper programs SwiftDM downloads itself (yt-dlp, FFmpeg, Deno).
pub fn tools_dir() -> PathBuf {
    data_dir().join("tools")
}

/// `Downloads/SwiftDM`, where files go unless chosen otherwise.
pub fn default_download_dir() -> PathBuf {
    dirs::download_dir()
        .or_else(|| dirs::home_dir().map(|h| h.join("Downloads")))
        .unwrap_or_else(std::env::temp_dir)
        .join("SwiftDM")
}
