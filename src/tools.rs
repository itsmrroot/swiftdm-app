//! The free helper programs behind video and music downloads, which SwiftDM
//! installs into its own folder on request (nothing else is needed — no
//! Python, no package manager):
//!
//! * **yt-dlp** reads 1800+ sites (YouTube, TikTok, Instagram, X, …).
//! * **FFmpeg** joins video and sound, and converts to MP3.
//! * **Deno** runs the JavaScript YouTube requires (Node.js works too).
//!
//! Copies already on the system are used when SwiftDM has none of its own.

use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::{Context, Result, bail, ensure};
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::paths::tools_dir;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Tool {
    YtDlp,
    Ffmpeg,
    Deno,
}

impl Tool {
    pub const ALL: [Tool; 3] = [Tool::YtDlp, Tool::Ffmpeg, Tool::Deno];

    pub fn name(self) -> &'static str {
        match self {
            Tool::YtDlp => "yt-dlp",
            Tool::Ffmpeg => "FFmpeg",
            Tool::Deno => "Deno",
        }
    }

    pub fn purpose(self) -> &'static str {
        match self {
            Tool::YtDlp => "Reads videos and music from 1800+ sites",
            Tool::Ffmpeg => "Joins video with sound and converts to MP3",
            Tool::Deno => "Runs the JavaScript YouTube needs",
        }
    }

    /// Roughly how much is downloaded to install it.
    pub fn download_size(self) -> &'static str {
        match self {
            Tool::YtDlp => "20–40 MB",
            Tool::Ffmpeg => "60–200 MB",
            Tool::Deno => "40–50 MB",
        }
    }

    fn exe(self) -> &'static str {
        match (self, cfg!(windows)) {
            (Tool::YtDlp, false) => "yt-dlp",
            (Tool::YtDlp, true) => "yt-dlp.exe",
            (Tool::Ffmpeg, false) => "ffmpeg",
            (Tool::Ffmpeg, true) => "ffmpeg.exe",
            (Tool::Deno, false) => "deno",
            (Tool::Deno, true) => "deno.exe",
        }
    }

    /// Where SwiftDM keeps its own copy.
    pub fn managed_path(self) -> PathBuf {
        tools_dir().join(self.exe())
    }
}

/// A command that shows no console window on Windows.
pub fn command(program: impl AsRef<std::ffi::OsStr>) -> Command {
    #[allow(unused_mut)]
    let mut cmd = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

/// Folders searched besides `PATH`: apps started from the Dock or a menu
/// often get a short `PATH` without Homebrew and the like.
fn search_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> =
        std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
    if !cfg!(windows) {
        for d in ["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin", "/snap/bin"] {
            dirs.push(d.into());
        }
    }
    if let Some(home) = dirs::home_dir() {
        dirs.push(home.join(".deno").join("bin"));
        dirs.push(home.join(".local").join("bin"));
    }
    dirs
}

/// `name` (without `.exe`) on the system, outside SwiftDM's folder.
pub fn find_on_system(name: &str) -> Option<PathBuf> {
    let file = if cfg!(windows) { format!("{name}.exe") } else { name.to_string() };
    search_dirs().into_iter().map(|d| d.join(&file)).find(|p| p.is_file())
}

/// The copy of `tool` to use: SwiftDM's own, else one on the system.
pub fn find(tool: Tool) -> Option<PathBuf> {
    let own = tool.managed_path();
    if own.is_file() {
        return Some(own);
    }
    find_on_system(tool.exe().trim_end_matches(".exe"))
}

/// The JavaScript runtime yt-dlp can use for YouTube, as yt-dlp names it.
pub fn js_runtime() -> Option<(&'static str, PathBuf)> {
    if let Some(p) = find(Tool::Deno) {
        return Some(("deno", p));
    }
    find_on_system("node").map(|p| ("node", p))
}

/// The version `path` reports, e.g. `2026.08.19` or `8.0`.
pub fn version(tool: Tool, path: &Path) -> Option<String> {
    let arg = if tool == Tool::Ffmpeg { "-version" } else { "--version" };
    let out = command(path).arg(arg).stdin(Stdio::null()).stderr(Stdio::null()).output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let first = text.lines().next()?.trim();
    let v = match tool {
        // "ffmpeg version 8.0-essentials Copyright ..."
        Tool::Ffmpeg => first.split_whitespace().nth(2)?,
        // "deno 2.9.7 (stable, release, ...)"
        Tool::Deno => first.split_whitespace().nth(1)?,
        Tool::YtDlp => first,
    };
    let v = v.split("-https").next().unwrap_or(v);
    Some(v.chars().take(32).collect())
}

#[derive(Debug, Clone)]
pub struct Status {
    pub tool: Tool,
    pub path: Option<PathBuf>,
    pub version: Option<String>,
    /// Installed by SwiftDM (and so updated by it).
    pub managed: bool,
}

impl Status {
    pub fn installed(&self) -> bool {
        self.path.is_some()
    }
}

/// Which copy of `tool` is used, and its version. Runs the program.
pub fn status(tool: Tool) -> Status {
    let path = find(tool);
    let version = path.as_deref().and_then(|p| version(tool, p));
    Status { tool, managed: path.as_deref() == Some(tool.managed_path().as_path()), path, version }
}

/// Whether SwiftDM can install `tool` on this system.
pub fn installable(tool: Tool) -> bool {
    match tool {
        Tool::YtDlp => ytdlp_asset().is_some(),
        Tool::Ffmpeg => cfg!(any(windows, target_os = "macos", target_os = "linux")),
        Tool::Deno => deno_triple().is_some(),
    }
}

/// Whether this Linux uses musl (Alpine and others), whose programs differ.
fn musl_system() -> bool {
    // A glibc system may have musl beside it (musl-tools, for developers):
    // it is musl only when there is no glibc.
    let glibc = [
        "/lib64/ld-linux-x86-64.so.2",
        "/lib/ld-linux-x86-64.so.2",
        "/lib/ld-linux-aarch64.so.1",
        "/lib/x86_64-linux-gnu/libc.so.6",
        "/lib/aarch64-linux-gnu/libc.so.6",
        "/usr/lib/libc.so.6",
    ]
    .iter()
    .any(|p| Path::new(p).exists());
    cfg!(target_os = "linux")
        && !glibc
        && std::fs::read_dir("/lib")
            .map(|d| d.flatten().any(|e| e.file_name().to_string_lossy().starts_with("ld-musl-")))
            .unwrap_or(false)
}

fn ytdlp_asset() -> Option<&'static str> {
    let arm = cfg!(target_arch = "aarch64");
    if musl_system() {
        return Some(if arm { "yt-dlp_musllinux_aarch64" } else { "yt-dlp_musllinux" });
    }
    Some(if cfg!(windows) {
        if arm { "yt-dlp_arm64.exe" } else { "yt-dlp.exe" }
    } else if cfg!(target_os = "macos") {
        "yt-dlp_macos"
    } else if cfg!(target_os = "linux") {
        if arm { "yt-dlp_linux_aarch64" } else { "yt-dlp_linux" }
    } else {
        return None;
    })
}

fn deno_triple() -> Option<&'static str> {
    // Deno is only built for glibc.
    if musl_system() {
        return None;
    }
    let arm = cfg!(target_arch = "aarch64");
    Some(match (std::env::consts::OS, arm) {
        ("windows", false) => "x86_64-pc-windows-msvc",
        ("windows", true) => "aarch64-pc-windows-msvc",
        ("macos", false) => "x86_64-apple-darwin",
        ("macos", true) => "aarch64-apple-darwin",
        ("linux", false) => "x86_64-unknown-linux-gnu",
        ("linux", true) => "aarch64-unknown-linux-gnu",
        _ => return None,
    })
}

#[derive(Deserialize)]
struct GhRelease {
    tag_name: String,
    #[serde(default)]
    assets: Vec<GhAsset>,
}

#[derive(Deserialize, Clone)]
struct GhAsset {
    name: String,
    browser_download_url: String,
    size: u64,
    /// `sha256:<hex>`.
    digest: Option<String>,
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .user_agent(format!("SwiftDM/{}", env!("CARGO_PKG_VERSION")))
        .timeout_connect(Some(Duration::from_secs(20)))
        .timeout_recv_response(Some(Duration::from_secs(30)))
        // Programs that are run afterwards are never fetched over plain http
        // (not even through a redirect).
        .https_only(true)
        // A stalled download ends some day (cancelling is checked between reads).
        .timeout_recv_body(Some(Duration::from_secs(30 * 60)))
        .build()
        .into()
}

/// Where `url` leads after redirects.
fn resolve(url: &str) -> Result<String> {
    use ureq::ResponseExt;
    let resp =
        agent().get(url).header("Range", "bytes=0-0").call().with_context(|| format!("could not download {url}"))?;
    Ok(resp.get_uri().to_string())
}

/// The SHA-256 published next to `url` as `<url>.sha256`, if there is one.
fn published_sha256(url: &str) -> Option<String> {
    let text =
        agent().get(format!("{url}.sha256")).call().ok()?.body_mut().with_config().limit(4096).read_to_string().ok()?;
    let hex = text.split_whitespace().next()?.to_ascii_lowercase();
    (hex.len() == 64 && hex.chars().all(|c| c.is_ascii_hexdigit())).then_some(hex)
}

fn github_release(repo: &str, tag: Option<&str>) -> Result<GhRelease> {
    let url = match tag {
        Some(t) => format!("https://api.github.com/repos/{repo}/releases/tags/{t}"),
        None => format!("https://api.github.com/repos/{repo}/releases/latest"),
    };
    let text = agent()
        .get(&url)
        .header("Accept", "application/vnd.github+json")
        .call()
        .context("could not reach GitHub")?
        .body_mut()
        .with_config()
        .limit(4 * 1024 * 1024)
        .read_to_string()?;
    serde_json::from_str(&text).context("unexpected answer from GitHub")
}

fn asset(release: &GhRelease, name: &str) -> Result<GhAsset> {
    release.assets.iter().find(|a| a.name == name).cloned().with_context(|| format!("{name} is not available"))
}

/// The newest yt-dlp version, e.g. `2026.08.19`.
pub fn latest_ytdlp() -> Result<String> {
    Ok(github_release("yt-dlp/yt-dlp", None)?.tag_name)
}

/// Progress of an installation: bytes so far and the total, if known.
pub type OnProgress<'a> = &'a (dyn Fn(u64, Option<u64>) + Sync);

/// Downloads `url` to `dest`, checking the size and SHA-256 when known.
fn fetch(
    url: &str,
    dest: &Path,
    size: Option<u64>,
    sha256: Option<&str>,
    progress: OnProgress,
    stop: &AtomicBool,
) -> Result<()> {
    let mut resp = agent().get(url).call().with_context(|| format!("could not download {url}"))?;
    let total = size
        .or_else(|| resp.headers().get("content-length").and_then(|v| v.to_str().ok()).and_then(|v| v.parse().ok()));
    let mut body = resp.body_mut().as_reader();
    let mut file = File::create(dest).with_context(|| format!("cannot write {}", dest.display()))?;
    let mut hash = Sha256::new();
    let mut buf = vec![0; 256 * 1024];
    let mut done = 0u64;
    loop {
        if stop.load(Ordering::Relaxed) {
            bail!("cancelled");
        }
        let n = body.read(&mut buf).context("the download was interrupted")?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n])?;
        hash.update(&buf[..n]);
        done += n as u64;
        progress(done, total);
    }
    file.sync_all()?;
    if let Some(size) = size {
        ensure!(done == size, "the download is incomplete");
    }
    if let Some(want) = sha256 {
        let got: String = hash.finalize().iter().map(|b| format!("{b:02x}")).collect();
        ensure!(got.eq_ignore_ascii_case(want), "the download is damaged (checksum mismatch)");
    }
    Ok(())
}

fn digest(a: &GhAsset) -> Option<&str> {
    a.digest.as_deref().and_then(|d| d.strip_prefix("sha256:"))
}

/// Makes `path` executable and moves it to `dest`, replacing what is there.
fn put_in_place(path: &Path, dest: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))?;
    }
    // Windows cannot replace a running program, but can rename it away.
    if dest.exists() {
        let old = dest.with_extension("old");
        let _ = std::fs::remove_file(&old);
        if std::fs::rename(dest, &old).is_ok() {
            let _ = std::fs::remove_file(&old);
        }
    }
    std::fs::rename(path, dest).with_context(|| format!("cannot install {}", dest.display()))
}

/// Pulls the file called `name` (in any folder) out of a zip archive.
fn unzip_one(archive: &Path, name: &str, dest: &Path) -> Result<()> {
    let mut zip = zip::ZipArchive::new(File::open(archive)?).context("the archive is damaged")?;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i)?;
        let matches = entry.is_file() && entry.name().rsplit('/').next() == Some(name);
        if matches {
            let mut out = File::create(dest)?;
            std::io::copy(&mut entry, &mut out)?;
            out.sync_all()?;
            return Ok(());
        }
    }
    bail!("{name} is missing from the archive")
}

/// Installs SwiftDM's own copy of `tool` (or replaces it with the newest
/// version). Returns where it is.
pub fn install(tool: Tool, progress: OnProgress, stop: &AtomicBool) -> Result<PathBuf> {
    let dir = tools_dir();
    std::fs::create_dir_all(&dir).with_context(|| format!("cannot create {}", dir.display()))?;
    let work = dir.join(format!(".install-{}", tool.exe()));
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work)?;
    let result = install_into(tool, &work, progress, stop);
    let _ = std::fs::remove_dir_all(&work);
    result?;
    Ok(tool.managed_path())
}

fn install_into(tool: Tool, work: &Path, progress: OnProgress, stop: &AtomicBool) -> Result<()> {
    match tool {
        Tool::YtDlp => {
            let name = ytdlp_asset().context("yt-dlp is not available for this system")?;
            let rel = github_release("yt-dlp/yt-dlp", None)?;
            let a = asset(&rel, name)?;
            let tmp = work.join(tool.exe());
            fetch(&a.browser_download_url, &tmp, Some(a.size), digest(&a), progress, stop)?;
            put_in_place(&tmp, &tool.managed_path())
        }
        Tool::Deno => {
            let triple = deno_triple().context("Deno is not available for this system")?;
            let rel = github_release("denoland/deno", None)?;
            let a = asset(&rel, &format!("deno-{triple}.zip"))?;
            let zip = work.join("deno.zip");
            fetch(&a.browser_download_url, &zip, Some(a.size), digest(&a), progress, stop)?;
            let tmp = work.join(tool.exe());
            unzip_one(&zip, tool.exe(), &tmp)?;
            put_in_place(&tmp, &tool.managed_path())
        }
        Tool::Ffmpeg => install_ffmpeg(work, progress, stop),
    }
}

/// FFmpeg and FFprobe (which yt-dlp also uses).
fn install_ffmpeg(work: &Path, progress: OnProgress, stop: &AtomicBool) -> Result<()> {
    let dir = tools_dir();
    let exe = |n: &str| if cfg!(windows) { format!("{n}.exe") } else { n.to_string() };
    if cfg!(target_os = "macos") {
        // Static builds by Martin Riedl, one archive per program.
        let arch = if cfg!(target_arch = "aarch64") { "arm64" } else { "amd64" };
        for name in ["ffprobe", "ffmpeg"] {
            let link = format!("https://ffmpeg.martin-riedl.de/redirect/latest/macos/{arch}/release/{name}.zip");
            // The build's own address, next to which its checksum is published.
            let url = resolve(&link)?;
            let sha = published_sha256(&url);
            let zip = work.join(format!("{name}.zip"));
            fetch(&url, &zip, None, sha.as_deref(), progress, stop)?;
            let tmp = work.join(name);
            unzip_one(&zip, name, &tmp)?;
            put_in_place(&tmp, &dir.join(name))?;
        }
        return Ok(());
    }
    // Builds made for yt-dlp.
    let rel = github_release("yt-dlp/FFmpeg-Builds", Some("latest"))?;
    let arm = cfg!(target_arch = "aarch64");
    if cfg!(windows) {
        let name = if arm { "ffmpeg-master-latest-winarm64-gpl.zip" } else { "ffmpeg-master-latest-win64-gpl.zip" };
        let a = asset(&rel, name)?;
        let zip = work.join("ffmpeg.zip");
        fetch(&a.browser_download_url, &zip, Some(a.size), digest(&a), progress, stop)?;
        for n in ["ffprobe", "ffmpeg"] {
            let tmp = work.join(exe(n));
            unzip_one(&zip, &exe(n), &tmp)?;
            put_in_place(&tmp, &dir.join(exe(n)))?;
        }
        return Ok(());
    }
    let name =
        if arm { "ffmpeg-master-latest-linuxarm64-gpl.tar.xz" } else { "ffmpeg-master-latest-linux64-gpl.tar.xz" };
    let a = asset(&rel, name)?;
    let archive = work.join("ffmpeg.tar.xz");
    fetch(&a.browser_download_url, &archive, Some(a.size), digest(&a), progress, stop)?;
    // Every Linux system has tar with xz support.
    let ok = Command::new("tar")
        .arg("-xJf")
        .arg(&archive)
        .arg("-C")
        .arg(work)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success());
    ensure!(ok, "could not unpack FFmpeg (is xz installed?)");
    let folder = std::fs::read_dir(work)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .find(|p| p.join("bin").join("ffmpeg").is_file())
        .context("FFmpeg is missing from the archive")?;
    for n in ["ffprobe", "ffmpeg"] {
        put_in_place(&folder.join("bin").join(n), &dir.join(n))?;
    }
    Ok(())
}

/// Removes SwiftDM's own copy of `tool`.
pub fn uninstall(tool: Tool) -> Result<()> {
    let mut files = vec![tool.managed_path()];
    if tool == Tool::Ffmpeg {
        files.push(tools_dir().join(if cfg!(windows) { "ffprobe.exe" } else { "ffprobe" }));
    }
    for f in files {
        if f.exists() {
            std::fs::remove_file(&f).with_context(|| format!("cannot remove {}", f.display()))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_supported_system_has_downloads() {
        if cfg!(any(windows, target_os = "macos", target_os = "linux")) {
            assert!(ytdlp_asset().is_some());
            assert!(deno_triple().is_some());
        }
        assert!(Tool::YtDlp.managed_path().starts_with(tools_dir()));
    }
}
