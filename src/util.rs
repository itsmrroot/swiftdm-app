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
            // Invisible and direction-changing characters can disguise a
            // name ("invoice\u{202E}fdp.exe" shows as "invoiceexe.pdf").
            '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' | '\u{FEFF}' => '_',
            c => c,
        })
        .collect();
    out = out.trim().trim_end_matches(['.', ' ']).to_string();
    // Windows reserves these names whatever the extension.
    let stem = out.split('.').next().unwrap_or("").trim_end().to_ascii_uppercase();
    let reserved = ["CON", "PRN", "AUX", "NUL"].contains(&stem.as_str())
        || (stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.as_bytes()[3].is_ascii_digit());
    if reserved {
        out.insert(0, '_');
    }
    // Most file systems allow 255 bytes per name (not characters: an Arabic
    // or Chinese character takes 2-3); room is left for " (2)" and ".part".
    const MAX_BYTES: usize = 200;
    if out.len() > MAX_BYTES {
        let ext = Path::new(&out).extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_default();
        let ext = if ext.is_empty() || ext.len() > 20 { String::new() } else { format!(".{ext}") };
        let mut stem = String::new();
        for c in out.chars() {
            if stem.len() + c.len_utf8() + ext.len() > MAX_BYTES {
                break;
            }
            stem.push(c);
        }
        out = format!("{}{ext}", stem.trim_end_matches(['.', ' ']));
    }
    if out.is_empty() { "download".into() } else { out }
}

/// `dir/name`, or `dir/name (2).ext` and so on if that already exists
/// (also counting an unfinished `.part` download of the same name).
pub fn unique_path(dir: &Path, name: &str) -> PathBuf {
    unique_path_except(dir, name, &[])
}

/// Like [`unique_path`], also avoiding `reserved` (files other downloads
/// will write, which may not exist yet).
pub fn unique_path_except(dir: &Path, name: &str, reserved: &[&Path]) -> PathBuf {
    let taken = |p: &Path| p.exists() || part_path(p).exists() || reserved.contains(&p);
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

/// How many links one pattern may make.
pub const MAX_PATTERN_LINKS: usize = 500;

/// One bracket group of a pattern: `[1-30]`, `[001-120]` or `[a-f]`.
enum Group {
    Numbers { start: u64, count: u64, width: usize },
    Letters { start: u8, count: u64 },
}

impl Group {
    fn parse(text: &str) -> Option<Group> {
        let (a, b) = text.split_once('-')?;
        let digits = |t: &str| !t.is_empty() && t.len() <= 9 && t.bytes().all(|c| c.is_ascii_digit());
        if digits(a) && digits(b) {
            let (start, end) = (a.parse::<u64>().ok()?, b.parse::<u64>().ok()?);
            // "[001-120]" keeps its zeros.
            let width = if a.len() > 1 && a.starts_with('0') { a.len() } else { 0 };
            return (end >= start).then(|| Group::Numbers { start, count: end - start + 1, width });
        }
        match (a.as_bytes(), b.as_bytes()) {
            ([x], [y])
                if x.is_ascii_alphabetic()
                    && y.is_ascii_alphabetic()
                    && x.is_ascii_lowercase() == y.is_ascii_lowercase()
                    && y >= x =>
            {
                Some(Group::Letters { start: *x, count: (*y - *x) as u64 + 1 })
            }
            _ => None,
        }
    }

    fn count(&self) -> u64 {
        match self {
            Group::Numbers { count, .. } | Group::Letters { count, .. } => *count,
        }
    }

    fn nth(&self, n: u64) -> String {
        match self {
            Group::Numbers { start, width, .. } => format!("{:0w$}", start + n, w = *width),
            Group::Letters { start, .. } => ((*start + n as u8) as char).to_string(),
        }
    }
}

/// The links a pattern stands for: `https://site/img[1-30].jpg` is thirty
/// links, `[001-120]` keeps its zeros, `[a-f]` goes through letters; several
/// groups make every combination. The pattern is looked for after the host
/// (an IPv6 address has brackets too). `Ok(None)`: not a pattern. `Err(n)`:
/// it would make `n` links, more than [`MAX_PATTERN_LINKS`].
pub fn expand_pattern(url: &str) -> Result<Option<Vec<String>>, u64> {
    let host_end = url.find("://").map_or(0, |i| i + 3);
    let path_at = url[host_end..].find('/').map_or(url.len(), |i| host_end + i);
    let (head, tail) = url.split_at(path_at);
    let mut literals: Vec<String> = vec![String::new()];
    let mut groups: Vec<Group> = Vec::new();
    let mut rest = tail;
    while let Some(open) = rest.find('[') {
        let Some(close) = rest[open..].find(']').map(|i| open + i) else { break };
        match Group::parse(&rest[open + 1..close]) {
            Some(g) => {
                if let Some(last) = literals.last_mut() {
                    last.push_str(&rest[..open]);
                }
                groups.push(g);
                literals.push(String::new());
            }
            // Not a range: brackets that belong to the link.
            None => {
                if let Some(last) = literals.last_mut() {
                    last.push_str(&rest[..=close]);
                }
            }
        }
        rest = &rest[close + 1..];
    }
    if groups.is_empty() {
        return Ok(None);
    }
    if let Some(last) = literals.last_mut() {
        last.push_str(rest);
    }
    let total = groups.iter().fold(1u64, |t, g| t.saturating_mul(g.count()));
    if total > MAX_PATTERN_LINKS as u64 {
        return Err(total);
    }
    let mut links = Vec::with_capacity(total as usize);
    for n in 0..total {
        let mut url = String::from(head);
        url.push_str(&literals[0]);
        // The last group changes fastest.
        let mut left = n;
        let mut picks = vec![0; groups.len()];
        for (i, g) in groups.iter().enumerate().rev() {
            picks[i] = left % g.count();
            left /= g.count();
        }
        for (i, g) in groups.iter().enumerate() {
            url.push_str(&g.nth(picks[i]));
            url.push_str(&literals[i + 1]);
        }
        links.push(url);
    }
    Ok(Some(links))
}

/// A checksum out of text a person pasted: "sha256:abc…", "abc…  file.iso"
/// or just the digits. SHA-256 (64) and SHA-512 (128 hex digits) are known.
pub fn parse_checksum(text: &str) -> Option<String> {
    text.split(|c: char| c.is_whitespace() || c == ':' || c == '=')
        .map(str::trim)
        .find(|t| matches!(t.len(), 64 | 128) && t.chars().all(|c| c.is_ascii_hexdigit()))
        .map(str::to_ascii_lowercase)
}

/// The hex digest of `path`, of the kind `expected` is (64 digits: SHA-256,
/// 128: SHA-512).
pub fn hash_file(path: &Path, expected: &str) -> std::io::Result<String> {
    use sha2::{Digest, Sha256, Sha512};
    use std::io::Read;
    let mut file = std::fs::File::open(path)?;
    let mut buf = vec![0u8; 1 << 20];
    let hex = |bytes: &[u8]| bytes.iter().map(|b| format!("{b:02x}")).collect::<String>();
    if expected.len() == 128 {
        let mut h = Sha512::new();
        loop {
            let n = file.read(&mut buf)?;
            if n == 0 {
                return Ok(hex(&h.finalize()));
            }
            h.update(&buf[..n]);
        }
    }
    let mut h = Sha256::new();
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            return Ok(hex(&h.finalize()));
        }
        h.update(&buf[..n]);
    }
}

/// A command line into its parts: spaces separate, double quotes group (a
/// backslash is nothing special, as it is in Windows paths).
pub fn split_command(line: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let (mut quoted, mut any) = (false, false);
    for c in line.chars() {
        match c {
            '"' => {
                quoted = !quoted;
                any = true;
            }
            c if c.is_whitespace() && !quoted => {
                if any || !current.is_empty() {
                    parts.push(std::mem::take(&mut current));
                    any = false;
                }
            }
            c => current.push(c),
        }
    }
    if any || !current.is_empty() {
        parts.push(current);
    }
    parts
}

/// The host of a link, without `www.`.
pub fn host(url: &str) -> &str {
    let rest = url.split("://").nth(1).unwrap_or(url);
    let host = rest.split(['/', '?', '#']).next().unwrap_or(rest);
    let host = host.rsplit('@').next().unwrap_or(host);
    // An IPv6 address is in brackets: its colons are not a port's.
    let host = match host.strip_prefix('[').and_then(|h| h.find(']').map(|i| &host[..i + 2])) {
        Some(bracketed) => bracketed,
        None => host.split(':').next().unwrap_or(host),
    };
    host.strip_prefix("www.").unwrap_or(host)
}

/// `url` as it may be written to a log: scheme, host and path, without the
/// user name and password, the query or the fragment (they may hold secrets).
pub fn redact_url(url: &str) -> String {
    let (scheme, rest) = url.split_once("://").map_or(("", url), |(s, r)| (s, r));
    let end = rest.find(['?', '#']).unwrap_or(rest.len());
    let rest = &rest[..end];
    let (authority, path) = rest.split_at(rest.find('/').unwrap_or(rest.len()));
    let host = authority.rsplit('@').next().unwrap_or(authority);
    if scheme.is_empty() { format!("{host}{path}") } else { format!("{scheme}://{host}{path}") }
}

/// Percent-decoding for file names taken from links.
pub fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |b: u8| (b as char).to_digit(16);
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let (Some(h), Some(l)) = (hex(bytes[i + 1]), hex(bytes[i + 2]))
        {
            let b = (h * 16 + l) as u8;
            out.push(b);
            i += 3;
            continue;
        }
        // `+` is only a space in query strings, never in paths or names.
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Marks a downloaded file as coming from the internet, as browsers do, so
/// that the system's checks still apply when it is opened (Gatekeeper on
/// macOS, SmartScreen and Protected View on Windows).
pub fn mark_from_internet(path: &Path, url: &str, referer: Option<&str>) {
    #[cfg(target_os = "macos")]
    {
        let _ = (url, referer);
        let flags = format!("0081;{:x};SwiftDM;", now());
        let _ = std::process::Command::new("xattr")
            .args(["-w", "com.apple.quarantine", &flags])
            .arg(path)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }
    #[cfg(windows)]
    {
        let mut zone = String::from("[ZoneTransfer]\r\nZoneId=3\r\n");
        if let Some(r) = referer {
            zone.push_str(&format!("ReferrerUrl={r}\r\n"));
        }
        zone.push_str(&format!("HostUrl={url}\r\n"));
        let mut stream = path.as_os_str().to_owned();
        stream.push(":Zone.Identifier");
        let _ = std::fs::write(PathBuf::from(stream), zone);
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    let _ = (path, url, referer);
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
        assert_eq!(sanitize_filename("con .txt"), "_con .txt");
        assert_eq!(sanitize_filename(""), "download");
        let long = format!("{}.mp4", "x".repeat(300));
        let s = sanitize_filename(&long);
        assert!(s.ends_with(".mp4") && s.len() <= 200);
        let arabic = format!("{}.zip", "ملف".repeat(100));
        let s = sanitize_filename(&arabic);
        assert!(s.ends_with(".zip") && s.len() <= 200, "{}", s.len());
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
        assert_eq!(percent_decode("a+b.zip"), "a+b.zip");
        assert_eq!(percent_decode("%aé"), "%aé");
        assert_eq!(percent_decode("a%20"), "a ");
        assert_eq!(sanitize_filename("invoice\u{202E}fdp.exe"), "invoice_fdp.exe");
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
        let reserved = dir.join("b.txt");
        assert_eq!(unique_path_except(&dir, "b.txt", &[&reserved]), dir.join("b (2).txt"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn hosts_of_addresses() {
        assert_eq!(host("https://www.example.com:8080/a"), "example.com");
        assert_eq!(host("http://[::1]:8080/f"), "[::1]");
        assert_ne!(host("http://[::1]:8080/f"), host("http://[2001:db8::1]/f"));
        assert_eq!(host("http://user@host.org/x"), "host.org");
    }

    #[test]
    fn urls_in_the_log_have_no_secrets() {
        assert_eq!(redact_url("https://user:pw@host.org/f/a.zip?token=SECRET#x"), "https://host.org/f/a.zip");
        assert_eq!(redact_url("http://host.org"), "http://host.org");
        assert_eq!(redact_url("https://h.org/a?b"), "https://h.org/a");
    }

    #[test]
    fn patterns_make_links() {
        let links = expand_pattern("https://a.org/img[1-3].jpg").unwrap().unwrap();
        assert_eq!(links, ["https://a.org/img1.jpg", "https://a.org/img2.jpg", "https://a.org/img3.jpg"]);
        let zeros = expand_pattern("https://a.org/p[008-011].png").unwrap().unwrap();
        assert_eq!(
            zeros,
            ["https://a.org/p008.png", "https://a.org/p009.png", "https://a.org/p010.png", "https://a.org/p011.png"]
        );
        let letters = expand_pattern("https://a.org/[a-c]/x.zip").unwrap().unwrap();
        assert_eq!(letters, ["https://a.org/a/x.zip", "https://a.org/b/x.zip", "https://a.org/c/x.zip"]);
        // Several groups: every combination, the last changing fastest.
        let both = expand_pattern("https://a.org/[1-2]/p[1-2].jpg").unwrap().unwrap();
        assert_eq!(
            both,
            ["https://a.org/1/p1.jpg", "https://a.org/1/p2.jpg", "https://a.org/2/p1.jpg", "https://a.org/2/p2.jpg"]
        );
        // Not patterns: no brackets, an IPv6 host, brackets that are no range.
        assert_eq!(expand_pattern("https://a.org/x.zip"), Ok(None));
        assert_eq!(expand_pattern("http://[::1]:8080/file.zip"), Ok(None));
        assert_eq!(expand_pattern("https://a.org/f[abc].zip"), Ok(None));
        assert_eq!(expand_pattern("https://a.org/f[3-1].zip"), Ok(None));
        // Too many.
        assert_eq!(expand_pattern("https://a.org/f[1-100000].zip"), Err(100_000));
        assert_eq!(expand_pattern("https://a.org/[1-30]/[1-30].zip"), Err(900));
    }

    #[test]
    fn checksums_are_found_in_pasted_text() {
        let h = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
        assert_eq!(parse_checksum(h).as_deref(), Some(h));
        assert_eq!(parse_checksum(&format!("SHA256: {}", h.to_uppercase())).as_deref(), Some(h));
        assert_eq!(parse_checksum(&format!("{h}  ubuntu.iso\n")).as_deref(), Some(h));
        assert_eq!(parse_checksum("sha256:e3b0"), None);
        assert_eq!(parse_checksum("not a checksum"), None);
        let long = "a".repeat(128);
        assert_eq!(parse_checksum(&long).as_deref(), Some(long.as_str()));
    }

    #[test]
    fn a_file_is_hashed_by_the_kind_of_the_expected_checksum() {
        let dir = std::env::temp_dir().join(format!("swiftdm-hash-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("empty");
        std::fs::write(&file, b"").unwrap();
        let sha256 = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
        assert_eq!(hash_file(&file, sha256).unwrap(), sha256);
        let sha512 = hash_file(&file, &"0".repeat(128)).unwrap();
        assert!(sha512.starts_with("cf83e1357eefb8bd"), "{sha512}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn commands_are_split_like_a_person_reads_them() {
        assert_eq!(split_command(r#"scan -f {file}"#), ["scan", "-f", "{file}"]);
        assert_eq!(
            split_command(r#""C:\Program Files\Defender\Scan.exe" -File "{file}" --quiet"#),
            [r"C:\Program Files\Defender\Scan.exe", "-File", "{file}", "--quiet"]
        );
        assert_eq!(split_command("   "), Vec::<String>::new());
        assert_eq!(split_command(r#"a "" b"#), ["a", "", "b"]);
    }
}
