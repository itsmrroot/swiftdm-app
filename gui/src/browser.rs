//! SwiftDM in the browser.
//!
//! The extension (in `assets/browser-extension`) talks to SwiftDM through
//! native messaging: the browser starts this program as its "helper" with
//! the extension's address as argument; the helper reads the extension's
//! messages, hands the links to the running app (starting it if needed) and
//! answers. Each start of the app registers the helper with the browsers on
//! this computer, so that it keeps working after updates and moves.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use anyhow::{Context, Result};
use serde_json::{Value, json};

use crate::instance::{self, Incoming};

/// The helper's name, as the extension asks for it.
pub const HOST_NAME: &str = "io.github.itsmrroot.swiftdm";
/// The extension's id in Chrome, Edge, Brave, … (fixed by the key in its manifest).
pub const CHROME_ID: &str = "fjeibcdeopjapibjafigloehhleijepj";
/// The extension's id in Firefox.
pub const FIREFOX_ID: &str = "swiftdm@itsmrroot.github.io";

/// Whether the browser started this program as the extension's helper.
pub fn launched_as_helper() -> bool {
    std::env::args().skip(1).any(|a| a.starts_with("chrome-extension://") || a == FIREFOX_ID)
}

// ---------------------------------------------------------------------------
// The helper

/// Answers the extension until the browser closes the connection.
pub fn run_helper() -> ! {
    let mut input = std::io::stdin().lock();
    let mut output = std::io::stdout().lock();
    loop {
        let mut len = [0u8; 4];
        if input.read_exact(&mut len).is_err() {
            std::process::exit(0);
        }
        let len = u32::from_ne_bytes(len) as usize;
        if len > 4 * 1024 * 1024 {
            std::process::exit(1);
        }
        let mut message = vec![0; len];
        if input.read_exact(&mut message).is_err() {
            std::process::exit(0);
        }
        let reply = match serde_json::from_slice::<Value>(&message) {
            Ok(m) => answer(&m),
            Err(_) => json!({ "ok": false, "error": "not a message" }),
        };
        let bytes = serde_json::to_vec(&reply).unwrap_or_default();
        let sent = output
            .write_all(&(bytes.len() as u32).to_ne_bytes())
            .and_then(|_| output.write_all(&bytes))
            .and_then(|_| output.flush());
        if sent.is_err() {
            std::process::exit(0);
        }
    }
}

fn seen_file() -> PathBuf {
    swiftdm::paths::data_dir().join("browser-seen")
}

/// When the extension last talked to SwiftDM (seconds since 1970).
pub fn last_seen() -> Option<i64> {
    std::fs::read_to_string(seen_file()).ok()?.trim().parse().ok()
}

fn answer(m: &Value) -> Value {
    let _ = std::fs::write(seen_file(), swiftdm::util::now().to_string());
    match m.get("type").and_then(Value::as_str) {
        Some("ping") => json!({ "ok": true, "version": env!("CARGO_PKG_VERSION") }),
        Some("show") => deliver(&[]),
        Some("download") => {
            let ask = m.get("ask").and_then(Value::as_bool);
            let items: Vec<Incoming> = m
                .get("items")
                .and_then(Value::as_array)
                .map(|items| items.iter().filter_map(|i| incoming(i, ask)).take(500).collect())
                .unwrap_or_default();
            if items.is_empty() { json!({ "ok": false, "error": "no link" }) } else { deliver(&items) }
        }
        _ => json!({ "ok": false, "error": "unknown message" }),
    }
}

/// One link from the extension, with what came along.
fn incoming(item: &Value, ask: Option<bool>) -> Option<Incoming> {
    let s = |k: &str| item.get(k).and_then(Value::as_str).map(str::trim).filter(|v| !v.is_empty());
    let url = s("url").filter(|u| swiftdm::util::is_link(u))?;
    let mut headers = Vec::new();
    for (key, name) in [("cookies", "Cookie"), ("referer", "Referer"), ("userAgent", "User-Agent")] {
        if let Some(v) = s(key) {
            headers.push((name.to_string(), v.to_string()));
        }
    }
    Some(Incoming {
        url: url.to_string(),
        mode: s("mode").unwrap_or("auto").to_string(),
        headers,
        name: s("filename").map(str::to_string),
        ask,
    })
}

/// Hands `items` to the running app, starting it if needed.
fn deliver(items: &[Incoming]) -> Value {
    if instance::send(items) {
        return json!({ "ok": true });
    }
    if let Err(e) = start_app() {
        return json!({ "ok": false, "error": format!("{e:#}") });
    }
    for _ in 0..80 {
        std::thread::sleep(Duration::from_millis(250));
        if instance::send(items) {
            return json!({ "ok": true });
        }
    }
    json!({ "ok": false, "error": "SwiftDM did not start" })
}

/// Starts the app on its own, outside the browser's control (the browser
/// ends its helper, and everything it started, when it is done with it).
fn start_app() -> Result<()> {
    let exe = std::env::current_exe()?;
    #[cfg(target_os = "macos")]
    if let Some(app) = exe.ancestors().find(|p| p.extension().is_some_and(|e| e == "app")) {
        Command::new("open").arg(app).status()?;
        return Ok(());
    }
    // An AppImage's files exist only while it runs: start the image itself.
    let program = std::env::var_os("APPIMAGE").map(PathBuf::from).unwrap_or(exe);
    let mut cmd = Command::new(program);
    cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;
        cmd.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP | CREATE_BREAKAWAY_FROM_JOB);
        if cmd.spawn().is_ok() {
            return Ok(());
        }
        // The browser's job may not allow leaving it.
        cmd.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
    }
    cmd.spawn().context("could not start SwiftDM")?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Browsers

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Browser {
    Chrome,
    Edge,
    Brave,
    Opera,
    Vivaldi,
    Chromium,
    Firefox,
}

impl Browser {
    pub const ALL: [Browser; 7] = [
        Browser::Chrome,
        Browser::Edge,
        Browser::Brave,
        Browser::Opera,
        Browser::Vivaldi,
        Browser::Chromium,
        Browser::Firefox,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Browser::Chrome => "Google Chrome",
            Browser::Edge => "Microsoft Edge",
            Browser::Brave => "Brave",
            Browser::Opera => "Opera",
            Browser::Vivaldi => "Vivaldi",
            Browser::Chromium => "Chromium",
            Browser::Firefox => "Firefox",
        }
    }

    pub fn is_firefox(self) -> bool {
        self == Browser::Firefox
    }

    /// The browser's page for extensions.
    pub fn extensions_page(self) -> &'static str {
        match self {
            Browser::Chrome | Browser::Chromium => "chrome://extensions",
            Browser::Edge => "edge://extensions",
            Browser::Brave => "brave://extensions",
            Browser::Opera => "opera://extensions",
            Browser::Vivaldi => "vivaldi://extensions",
            Browser::Firefox => "about:debugging#/runtime/this-firefox",
        }
    }

    /// The browser's own folder for this user (where it keeps profiles).
    fn data_dir(self) -> Option<PathBuf> {
        let home = dirs::home_dir()?;
        if cfg!(target_os = "macos") {
            let support = home.join("Library/Application Support");
            Some(support.join(match self {
                Browser::Chrome => "Google/Chrome",
                Browser::Edge => "Microsoft Edge",
                Browser::Brave => "BraveSoftware/Brave-Browser",
                Browser::Opera => "com.operasoftware.Opera",
                Browser::Vivaldi => "Vivaldi",
                Browser::Chromium => "Chromium",
                Browser::Firefox => "Firefox",
            }))
        } else if cfg!(windows) {
            let local = dirs::data_local_dir()?;
            let roaming = dirs::data_dir()?;
            Some(match self {
                Browser::Chrome => local.join(r"Google\Chrome"),
                Browser::Edge => local.join(r"Microsoft\Edge"),
                Browser::Brave => local.join(r"BraveSoftware\Brave-Browser"),
                Browser::Opera => roaming.join(r"Opera Software\Opera Stable"),
                Browser::Vivaldi => local.join("Vivaldi"),
                Browser::Chromium => local.join("Chromium"),
                Browser::Firefox => roaming.join(r"Mozilla\Firefox"),
            })
        } else {
            let config = home.join(".config");
            Some(match self {
                Browser::Chrome => config.join("google-chrome"),
                Browser::Edge => config.join("microsoft-edge"),
                Browser::Brave => config.join("BraveSoftware/Brave-Browser"),
                Browser::Opera => config.join("opera"),
                Browser::Vivaldi => config.join("vivaldi"),
                Browser::Chromium => config.join("chromium"),
                Browser::Firefox => home.join(".mozilla"),
            })
        }
    }

    /// Whether the browser seems to be installed (it has its folder).
    pub fn installed(self) -> bool {
        self.data_dir().is_some_and(|d| d.is_dir())
    }

    /// Where the browser looks for helpers (macOS and Linux).
    fn hosts_dir(self) -> Option<PathBuf> {
        let home = dirs::home_dir()?;
        Some(match self {
            Browser::Firefox if cfg!(target_os = "macos") => {
                home.join("Library/Application Support/Mozilla/NativeMessagingHosts")
            }
            Browser::Firefox => home.join(".mozilla/native-messaging-hosts"),
            // Opera reads Chrome's helpers.
            Browser::Opera => Browser::Chrome.data_dir()?.join("NativeMessagingHosts"),
            b => b.data_dir()?.join("NativeMessagingHosts"),
        })
    }

    /// Where the browser looks for helpers (Windows registry, per user).
    #[cfg(windows)]
    fn registry_key(self) -> &'static str {
        match self {
            Browser::Chrome | Browser::Opera => r"HKCU\Software\Google\Chrome\NativeMessagingHosts",
            Browser::Edge => r"HKCU\Software\Microsoft\Edge\NativeMessagingHosts",
            Browser::Brave => r"HKCU\Software\BraveSoftware\Brave-Browser\NativeMessagingHosts",
            Browser::Vivaldi => r"HKCU\Software\Vivaldi\NativeMessagingHosts",
            Browser::Chromium => r"HKCU\Software\Chromium\NativeMessagingHosts",
            Browser::Firefox => r"HKCU\Software\Mozilla\NativeMessagingHosts",
        }
    }

    /// Opens `url` in this browser.
    pub fn open(self, url: &str) -> Result<()> {
        #[cfg(target_os = "macos")]
        {
            let app = match self {
                Browser::Chrome => "Google Chrome",
                Browser::Edge => "Microsoft Edge",
                Browser::Brave => "Brave Browser",
                Browser::Opera => "Opera",
                Browser::Vivaldi => "Vivaldi",
                Browser::Chromium => "Chromium",
                Browser::Firefox => "Firefox",
            };
            // The program itself: `open` does not pass chrome:// addresses on.
            let exe = PathBuf::from(format!("/Applications/{app}.app/Contents/MacOS/{app}"));
            if exe.is_file() {
                Command::new(exe).arg(url).stdout(Stdio::null()).stderr(Stdio::null()).spawn()?;
            } else {
                Command::new("open").args(["-a", app, url]).spawn()?;
            }
            Ok(())
        }
        #[cfg(windows)]
        {
            let exe = match self {
                Browser::Chrome => "chrome",
                Browser::Edge => "msedge",
                Browser::Brave => "brave",
                Browser::Opera => "opera",
                Browser::Vivaldi => "vivaldi",
                Browser::Chromium => "chromium",
                Browser::Firefox => "firefox",
            };
            swiftdm::tools::command("cmd").args(["/C", "start", "", exe, url]).spawn()?;
            Ok(())
        }
        #[cfg(not(any(target_os = "macos", windows)))]
        {
            let candidates: &[&str] = match self {
                Browser::Chrome => &["google-chrome", "google-chrome-stable"],
                Browser::Edge => &["microsoft-edge", "microsoft-edge-stable"],
                Browser::Brave => &["brave-browser", "brave"],
                Browser::Opera => &["opera"],
                Browser::Vivaldi => &["vivaldi", "vivaldi-stable"],
                Browser::Chromium => &["chromium", "chromium-browser"],
                Browser::Firefox => &["firefox"],
            };
            let exe = candidates
                .iter()
                .find_map(|c| swiftdm::tools::find_on_system(c))
                .with_context(|| format!("{} was not found", self.name()))?;
            Command::new(exe).arg(url).stdout(Stdio::null()).stderr(Stdio::null()).spawn()?;
            Ok(())
        }
    }
}

/// The program browsers start as the helper.
fn helper_program() -> Result<PathBuf> {
    // An AppImage runs from a temporary mount: a small script starts the
    // image itself.
    if let Some(image) = std::env::var_os("APPIMAGE") {
        let script = swiftdm::paths::data_dir().join("native-host").join("swiftdm-helper.sh");
        std::fs::create_dir_all(script.parent().unwrap_or(Path::new(".")))?;
        let image = PathBuf::from(image).display().to_string().replace('\'', r"'\''");
        std::fs::write(&script, format!("#!/bin/sh\nexec '{image}' \"$@\"\n"))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755))?;
        }
        return Ok(script);
    }
    Ok(std::env::current_exe()?)
}

fn host_manifest(firefox: bool, program: &Path) -> String {
    let mut m = json!({
        "name": HOST_NAME,
        "description": "SwiftDM — Powered by Bashar Salmo",
        "path": program.display().to_string(),
        "type": "stdio",
    });
    if firefox {
        m["allowed_extensions"] = json!([FIREFOX_ID]);
    } else {
        m["allowed_origins"] = json!([format!("chrome-extension://{CHROME_ID}/")]);
    }
    serde_json::to_string_pretty(&m).unwrap_or_default()
}

/// Registers the helper with every installed browser. Returns the browsers
/// it was registered with.
pub fn register() -> Result<Vec<Browser>> {
    let program = helper_program()?;
    let dir = swiftdm::paths::data_dir().join("native-host");
    std::fs::create_dir_all(&dir)?;
    let chrome = dir.join(format!("{HOST_NAME}.json"));
    let firefox = dir.join(format!("{HOST_NAME}.firefox.json"));
    std::fs::write(&chrome, host_manifest(false, &program))?;
    std::fs::write(&firefox, host_manifest(true, &program))?;
    let mut done = Vec::new();
    for b in Browser::ALL.into_iter().filter(|b| b.installed()) {
        let manifest = if b.is_firefox() { &firefox } else { &chrome };
        #[cfg(windows)]
        {
            let key = format!(r"{}\{HOST_NAME}", b.registry_key());
            let ok = swiftdm::tools::command("reg")
                .args(["add", &key, "/ve", "/t", "REG_SZ", "/d", &manifest.display().to_string(), "/f"])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .is_ok_and(|s| s.success());
            if ok {
                done.push(b);
            }
        }
        #[cfg(not(windows))]
        {
            let Some(hosts) = b.hosts_dir() else { continue };
            if std::fs::create_dir_all(&hosts).is_ok()
                && std::fs::copy(manifest, hosts.join(format!("{HOST_NAME}.json"))).is_ok()
            {
                done.push(b);
            }
        }
    }
    Ok(done)
}

// ---------------------------------------------------------------------------
// The extension's files

macro_rules! files {
    ($($path:literal),* $(,)?) => {
        &[$(($path, include_bytes!(concat!("../assets/browser-extension/", $path)) as &[u8])),*]
    };
}

const EXTENSION: &[(&str, &[u8])] = files!(
    "manifest.json",
    "background.js",
    "content.js",
    "popup.html",
    "popup.css",
    "popup.js",
    "icons/icon16.png",
    "icons/icon32.png",
    "icons/icon48.png",
    "icons/icon128.png",
    "_locales/en/messages.json",
    "_locales/ar/messages.json",
    "_locales/de/messages.json",
    "_locales/tr/messages.json",
);

/// The manifest for Firefox: background scripts instead of a service worker,
/// its own id, and no Chrome key.
fn firefox_manifest(chrome: &[u8]) -> Result<Vec<u8>> {
    let mut m: Value = serde_json::from_slice(chrome)?;
    let o = m.as_object_mut().context("bad manifest")?;
    o.remove("key");
    o.insert("background".into(), json!({ "scripts": ["background.js"] }));
    o.insert(
        "browser_specific_settings".into(),
        json!({ "gecko": { "id": FIREFOX_ID, "strict_min_version": "128.0" } }),
    );
    Ok(serde_json::to_vec_pretty(&m)?)
}

/// Writes the extension for `browser`'s kind into SwiftDM's folder (to load
/// it "unpacked") and returns the folder.
pub fn write_extension(firefox: bool) -> Result<PathBuf> {
    let dir = swiftdm::paths::data_dir().join("browser-extension").join(if firefox { "firefox" } else { "chrome" });
    for (path, bytes) in EXTENSION {
        let target = dir.join(path);
        std::fs::create_dir_all(target.parent().unwrap_or(&dir))?;
        let bytes = if firefox && *path == "manifest.json" { firefox_manifest(bytes)? } else { bytes.to_vec() };
        std::fs::write(&target, bytes).with_context(|| format!("cannot write {}", target.display()))?;
    }
    Ok(dir)
}

/// Keeps an extension loaded "unpacked" up to date with this version.
pub fn refresh_extension_files() {
    for firefox in [false, true] {
        let dir = swiftdm::paths::data_dir().join("browser-extension").join(if firefox { "firefox" } else { "chrome" });
        if dir.is_dir() {
            let _ = write_extension(firefox);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_extension_id_matches_its_key() {
        use sha2::{Digest, Sha256};
        let manifest: Value = serde_json::from_slice(EXTENSION[0].1).unwrap();
        let key = manifest["key"].as_str().unwrap();
        let der = base64_decode(key);
        let hash = Sha256::digest(&der);
        let id: String = hash[..16].iter().flat_map(|b| [b >> 4, b & 15]).map(|n| (b'a' + n) as char).collect();
        assert_eq!(id, CHROME_ID);
    }

    #[test]
    fn messages_become_downloads() {
        let item =
            json!({ "url": "https://x.org/a.zip", "mode": "file", "cookies": "s=1", "referer": "https://x.org/" });
        let i = incoming(&item, Some(false)).unwrap();
        assert_eq!(i.mode, "file");
        assert_eq!(i.ask, Some(false));
        assert!(i.headers.contains(&("Cookie".into(), "s=1".into())));
        assert!(incoming(&json!({ "url": "javascript:alert(1)" }), None).is_none());
        assert_eq!(answer(&json!({ "type": "ping" }))["ok"], true);
    }

    #[test]
    fn firefox_gets_its_own_manifest() {
        let m: Value = serde_json::from_slice(&firefox_manifest(EXTENSION[0].1).unwrap()).unwrap();
        assert!(m.get("key").is_none());
        assert_eq!(m["browser_specific_settings"]["gecko"]["id"], FIREFOX_ID);
        assert_eq!(m["background"]["scripts"][0], "background.js");
        let h: Value = serde_json::from_str(&host_manifest(false, Path::new("/x/swiftdm-gui"))).unwrap();
        assert_eq!(h["allowed_origins"][0], format!("chrome-extension://{CHROME_ID}/"));
    }

    fn base64_decode(s: &str) -> Vec<u8> {
        let table = |c: u8| match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => 0,
        };
        let bytes: Vec<u8> = s.bytes().filter(|c| *c != b'=').collect();
        let mut out = Vec::new();
        for chunk in bytes.chunks(4) {
            let v: Vec<u32> = chunk.iter().map(|c| table(*c) as u32).collect();
            let n = v.iter().enumerate().fold(0u32, |acc, (i, x)| acc | (x << (18 - 6 * i)));
            for i in 0..chunk.len().saturating_sub(1) {
                out.push((n >> (16 - 8 * i)) as u8);
            }
        }
        out
    }
}
