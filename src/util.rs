//! Small helpers shared by the engine, the command line and the app:
//! sizes, durations, file names and kinds.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// `1.5 MB`, using powers of 1024.
pub fn format_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = bytes as f64;
    let mut unit = 0;
    while v >= 1024.0 && unit < UNITS.len() - 1 {
        v /= 1024.0;
        unit += 1;
    }
    if unit == 0 { format!("{bytes} B") } else { format!("{v:.1} {}", UNITS[unit]) }
}

/// `2.4 MB/s`.
pub fn format_speed(bytes_per_sec: f64) -> String {
    format!("{}/s", format_size(bytes_per_sec.max(0.0) as u64))
}

/// `45s`, `3m 12s`, `1h 05m`.
pub fn format_eta(secs: u64) -> String {
    match secs {
        0..=59 => format!("{secs}s"),
        60..=3599 => format!("{}m {:02}s", secs / 60, secs % 60),
        _ => format!("{}h {:02}m", secs / 3600, (secs % 3600) / 60),
    }
}

/// A file name that every system accepts: no path separators, reserved
/// characters or trailing dots, and not too long.
pub fn sanitize_filename(name: &str) -> String {
    let mut out: String = name
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect();
    out = out.trim().trim_end_matches(['.', ' ']).to_string();
    // Windows reserves these names whatever the extension.
    let stem = out.split('.').next().unwrap_or("").to_ascii_uppercase();
    let reserved = ["CON", "PRN", "AUX", "NUL"].contains(&stem.as_str())
        || (stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.as_bytes()[3].is_ascii_digit());
    if reserved {
        out.insert(0, '_');
    }
    if out.chars().count() > 180 {
        let ext = Path::new(&out).extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_default();
        let keep = 180 - ext.chars().count().min(20) - 1;
        let stem: String = out.chars().take(keep).collect();
        out = if ext.is_empty() || ext.chars().count() > 20 { stem } else { format!("{stem}.{ext}") };
    }
    if out.is_empty() { "download".into() } else { out }
}

/// `dir/name`, or `dir/name (2).ext` and so on if that already exists
/// (also counting an unfinished `.part` download of the same name).
pub fn unique_path(dir: &Path, name: &str) -> PathBuf {
    let taken = |p: &Path| p.exists() || part_path(p).exists();
    let first = dir.join(name);
    if !taken(&first) {
        return first;
    }
    let p = Path::new(name);
    let stem = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| name.to_string());
    let ext = p.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    (2..).map(|i| dir.join(format!("{stem} ({i}){ext}"))).find(|p| !taken(p)).expect("an unused name exists")
}

/// Where a file is written while it downloads.
pub fn part_path(path: &Path) -> PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(".part");
    PathBuf::from(s)
}

/// What a file is, by its extension: used for icons and filters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum FileKind {
    Video,
    Audio,
    Image,
    Archive,
    Program,
    Document,
    Other,
}

impl FileKind {
    pub const ALL: [FileKind; 7] = [
        FileKind::Video,
        FileKind::Audio,
        FileKind::Image,
        FileKind::Archive,
        FileKind::Program,
        FileKind::Document,
        FileKind::Other,
    ];

    pub fn of(name: &str) -> FileKind {
        let ext = Path::new(name).extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
        match ext.as_str() {
            "mp4" | "mkv" | "webm" | "mov" | "avi" | "m4v" | "flv" | "wmv" | "3gp" | "ts" | "mpg" | "mpeg" => {
                FileKind::Video
            }
            "mp3" | "m4a" | "aac" | "flac" | "wav" | "ogg" | "opus" | "wma" | "alac" | "aiff" => FileKind::Audio,
            "jpg" | "jpeg" | "png" | "gif" | "webp" | "bmp" | "svg" | "heic" | "tif" | "tiff" | "avif" => {
                FileKind::Image
            }
            "zip" | "rar" | "7z" | "tar" | "gz" | "tgz" | "bz2" | "xz" | "zst" | "iso" => FileKind::Archive,
            "exe" | "msi" | "dmg" | "pkg" | "deb" | "rpm" | "appimage" | "apk" | "msix" => FileKind::Program,
            "pdf" | "doc" | "docx" | "xls" | "xlsx" | "ppt" | "pptx" | "txt" | "csv" | "rtf" | "odt" | "epub"
            | "md" | "json" => FileKind::Document,
            _ => FileKind::Other,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            FileKind::Video => "Video",
            FileKind::Audio => "Music",
            FileKind::Image => "Images",
            FileKind::Archive => "Archives",
            FileKind::Program => "Programs",
            FileKind::Document => "Documents",
            FileKind::Other => "Other",
        }
    }
}

/// Whether `text` is a link SwiftDM can download.
pub fn is_link(text: &str) -> bool {
    let t = text.trim();
    (t.starts_with("http://") || t.starts_with("https://"))
        && t.len() > 10
        && !t.contains(char::is_whitespace)
        && t.split("://").nth(1).is_some_and(|rest| rest.split('/').next().is_some_and(|host| host.contains('.')))
}

/// The host of a link, without `www.`.
pub fn host(url: &str) -> &str {
    let rest = url.split("://").nth(1).unwrap_or(url);
    let host = rest.split(['/', '?', '#']).next().unwrap_or(rest);
    let host = host.rsplit('@').next().unwrap_or(host);
    let host = host.split(':').next().unwrap_or(host);
    host.strip_prefix("www.").unwrap_or(host)
}

/// Percent-decoding for file names taken from links.
pub fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Ok(b) = u8::from_str_radix(&s[i + 1..i + 3], 16)
        {
            out.push(b);
            i += 3;
            continue;
        }
        out.push(if bytes[i] == b'+' { b' ' } else { bytes[i] });
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Seconds since 1970.
pub fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// Opens a file or folder with the system's default program.
pub fn open_path(path: &Path) -> std::io::Result<()> {
    #[cfg(windows)]
    let mut cmd = {
        let mut c = std::process::Command::new("explorer");
        c.arg(path);
        c
    };
    #[cfg(target_os = "macos")]
    let mut cmd = {
        let mut c = std::process::Command::new("open");
        c.arg(path);
        c
    };
    #[cfg(not(any(windows, target_os = "macos")))]
    let mut cmd = {
        let mut c = std::process::Command::new("xdg-open");
        c.arg(path);
        c
    };
    cmd.spawn().map(|_| ())
}

/// Shows `path` selected in the file manager (or its folder, where the file
/// manager cannot select).
pub fn reveal_path(path: &Path) -> std::io::Result<()> {
    if !path.exists() {
        return open_path(path.parent().unwrap_or(path));
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // `/select,` takes the rest of the command line as the path, quotes included.
        std::process::Command::new("explorer").raw_arg(format!("/select,\"{}\"", path.display())).spawn().map(|_| ())
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open").arg("-R").arg(path).spawn().map(|_| ())
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        open_path(path.parent().unwrap_or(path))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes() {
        assert_eq!(format_size(0), "0 B");
        assert_eq!(format_size(1023), "1023 B");
        assert_eq!(format_size(1536), "1.5 KB");
        assert_eq!(format_size(5 * 1024 * 1024 * 1024), "5.0 GB");
        assert_eq!(format_eta(75), "1m 15s");
        assert_eq!(format_eta(3900), "1h 05m");
    }

    #[test]
    fn file_names_are_safe() {
        assert_eq!(sanitize_filename("a/b:c?.mp4"), "a_b_c_.mp4");
        assert_eq!(sanitize_filename("  name.  "), "name");
        assert_eq!(sanitize_filename("CON.txt"), "_CON.txt");
        assert_eq!(sanitize_filename(""), "download");
        let long = format!("{}.mp4", "x".repeat(300));
        let s = sanitize_filename(&long);
        assert!(s.ends_with(".mp4") && s.chars().count() <= 180);
    }

    #[test]
    fn links() {
        assert!(is_link("https://www.youtube.com/watch?v=abc"));
        assert!(is_link(" http://example.com/file.zip "));
        assert!(!is_link("hello world"));
        assert!(!is_link("https://localhost"));
        assert_eq!(host("https://www.YouTube.com/watch?v=1"), "YouTube.com");
        assert_eq!(host("https://user@cdn.example.org:8080/x"), "cdn.example.org");
        assert_eq!(percent_decode("my%20file%2B1.zip"), "my file+1.zip");
    }

    #[test]
    fn kinds() {
        assert_eq!(FileKind::of("a.MP4"), FileKind::Video);
        assert_eq!(FileKind::of("song.mp3"), FileKind::Audio);
        assert_eq!(FileKind::of("setup.exe"), FileKind::Program);
        assert_eq!(FileKind::of("noext"), FileKind::Other);
    }

    #[test]
    fn unique_names() {
        let dir = std::env::temp_dir().join(format!("swiftdm-unique-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.txt"), "x").unwrap();
        std::fs::write(dir.join("a (2).txt.part"), "x").unwrap();
        assert_eq!(unique_path(&dir, "a.txt"), dir.join("a (3).txt"));
        assert_eq!(unique_path(&dir, "b.txt"), dir.join("b.txt"));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
