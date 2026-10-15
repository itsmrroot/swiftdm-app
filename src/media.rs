//! Videos and music from sites (YouTube, TikTok, Instagram, X and 1800+
//! more), read by yt-dlp.

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::tools::{self, Tool};

/// What to keep of a video.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Mode {
    /// Video with sound (or whatever the post is: a photo, a clip).
    #[default]
    Video,
    /// The sound only, as MP3.
    Audio,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Options {
    pub mode: Mode,
    /// Highest video height (1080, 720, …); `None` is the best there is.
    pub max_height: Option<u32>,
    /// MP3 bit rate in kbit/s; 0 is the best (variable) quality.
    pub audio_kbps: u32,
    /// Browser to take the login (cookies) from: `chrome`, `firefox`, ….
    pub cookies_from: Option<String>,
    /// For the sound only.
    #[serde(default)]
    pub audio_format: AudioFormat,
    /// For videos.
    #[serde(default)]
    pub container: Container,
    /// Subtitle languages, e.g. `en,ar` (`all` for every one); empty: none.
    #[serde(default)]
    pub subtitles: String,
    /// Cut out sponsor segments, intros asking to subscribe and the like
    /// (SponsorBlock).
    #[serde(default)]
    pub sponsorblock: bool,
    /// Only this part, from and to in seconds.
    #[serde(default)]
    pub section: Option<(u32, u32)>,
}

/// File type for the sound only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum AudioFormat {
    #[default]
    Mp3,
    M4a,
    Opus,
    Flac,
    Wav,
}

impl AudioFormat {
    pub const ALL: [AudioFormat; 5] =
        [AudioFormat::Mp3, AudioFormat::M4a, AudioFormat::Opus, AudioFormat::Flac, AudioFormat::Wav];

    /// As yt-dlp names it, which is also the extension.
    pub fn name(self) -> &'static str {
        match self {
            AudioFormat::Mp3 => "mp3",
            AudioFormat::M4a => "m4a",
            AudioFormat::Opus => "opus",
            AudioFormat::Flac => "flac",
            AudioFormat::Wav => "wav",
        }
    }

    /// Whether a bit rate applies (not to lossless formats).
    pub fn lossy(self) -> bool {
        !matches!(self, AudioFormat::Flac | AudioFormat::Wav)
    }
}

/// File type for videos.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Container {
    /// Plays everywhere (H.264 preferred).
    #[default]
    Mp4,
    /// Takes any codec: the best quality as it is.
    Mkv,
    /// VP9 / AV1 with Opus.
    Webm,
}

impl Container {
    pub const ALL: [Container; 3] = [Container::Mp4, Container::Mkv, Container::Webm];

    pub fn name(self) -> &'static str {
        match self {
            Container::Mp4 => "mp4",
            Container::Mkv => "mkv",
            Container::Webm => "webm",
        }
    }
}

/// One video, song or post.
#[derive(Debug, Clone, Default)]
pub struct Info {
    pub title: String,
    pub uploader: String,
    pub duration: Option<f64>,
    pub thumbnail: Option<String>,
    /// The site, e.g. `Youtube`.
    pub site: String,
    /// Video heights offered, highest first.
    pub heights: Vec<u32>,
    /// Estimated size at each of `heights` (with sound), when known.
    pub sizes: Vec<Option<u64>>,
    pub is_live: bool,
}

/// An item of a playlist or channel.
#[derive(Debug, Clone)]
pub struct Entry {
    pub title: String,
    pub url: String,
    pub duration: Option<f64>,
}

#[derive(Debug, Clone)]
pub enum Found {
    Single(Info),
    Playlist { title: String, entries: Vec<Entry> },
}

/// Why yt-dlp failed, so that the app can react (retry with a login, fall
/// back to a plain download, offer to install a helper).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    /// yt-dlp is not installed.
    NoYtDlp,
    /// The site wants a logged-in user.
    Login,
    /// The login of the chosen browser could not be read (not installed,
    /// no permission, locked while the browser runs).
    Cookies,
    /// The site refused (often fixed by a login or a yt-dlp update).
    Forbidden,
    /// yt-dlp does not know this link: it may be a plain file.
    Unsupported,
    /// Private, deleted, or blocked in this country.
    Unavailable,
    NeedsFfmpeg,
    NeedsJs,
    Other,
}

#[derive(Debug, Clone)]
pub struct MediaError {
    pub kind: ErrorKind,
    pub message: String,
}

impl std::fmt::Display for MediaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for MediaError {}

/// Turns yt-dlp's error output into a friendly message.
pub fn classify(stderr: &str) -> MediaError {
    let error = stderr
        .lines()
        .rev()
        .find(|l| l.starts_with("ERROR:"))
        .or_else(|| stderr.lines().rev().find(|l| !l.trim().is_empty() && !l.starts_with("WARNING:")))
        .unwrap_or("yt-dlp stopped without saying why")
        .trim_start_matches("ERROR:")
        .trim();
    // Drop "[youtube] abc123: " in front of the message.
    let detail = match error.strip_prefix('[') {
        Some(rest) => rest.split_once("] ").map(|(_, m)| m).unwrap_or(error),
        None => error,
    };
    let detail =
        detail.split_once(": ").filter(|(id, _)| !id.contains(' ') && id.len() < 40).map_or(detail, |(_, m)| m);
    // The error decides; warnings only tell about a missing JavaScript runtime.
    let low = error.to_lowercase();
    let warnings = stderr.to_lowercase();
    let cookie_trouble = (low.contains("cookie")
        && (low.contains("database")
            || low.contains("could not copy")
            || low.contains("decrypt")
            || low.contains("keyring")
            || low.contains("binarycookies")
            || low.contains("not permitted")))
        || low.contains("unsupported browser");
    let (kind, message) = if low.contains("unsupported url") {
        (ErrorKind::Unsupported, "This link has no video or music SwiftDM can read.".to_string())
    } else if cookie_trouble {
        (
            ErrorKind::Cookies,
            "The login of the chosen browser could not be read. Choose another browser in Settings, or none."
                .to_string(),
        )
    } else if warnings.contains("javascript runtime")
        || warnings.contains("js runtime")
        || warnings.contains("challenge solving failed")
    {
        (ErrorKind::NeedsJs, "YouTube needs Deno: install it in Settings → Helpers.".to_string())
    } else if low.contains("sign in")
        || low.contains("login required")
        || low.contains("log in")
        || low.contains("--cookies")
        || low.contains("rate-limit reached")
    {
        (
            ErrorKind::Login,
            "The site wants you to be logged in. In Settings, choose the browser you are logged in with.".to_string(),
        )
    } else if low.contains("ffmpeg") && (low.contains("not found") || low.contains("not installed")) {
        (ErrorKind::NeedsFfmpeg, "This needs FFmpeg: install it in Settings → Helpers.".to_string())
    } else if low.contains("private video")
        || low.contains("video unavailable")
        || low.contains("video is unavailable")
        || low.contains("has been removed")
        || low.contains("not available in your country")
        || low.contains("geo restrict")
        || low.contains("this content isn't available")
    {
        (ErrorKind::Unavailable, format!("Not available: {detail}"))
    } else if low.contains("http error 403") || low.contains("forbidden") {
        (
            ErrorKind::Forbidden,
            "The site refused the download (HTTP 403). Updating yt-dlp or logging in usually helps.".to_string(),
        )
    } else {
        (ErrorKind::Other, detail.to_string())
    };
    MediaError { kind, message }
}

/// The step a `[sdm-pp]` line announces, if it starts one.
fn postprocess_stage(line: &str) -> Option<&'static str> {
    let rest = line.strip_prefix("[sdm-pp] ")?;
    let (status, pp) = rest.split_once('|').unwrap_or((rest, ""));
    (status == "started").then_some(match pp {
        "Merger" => "Joining video and sound",
        "ExtractAudio" => "Converting the sound",
        "EmbedThumbnail" => "Adding the cover",
        "Metadata" | "FFmpegMetadata" => "Adding details",
        _ => "Finishing",
    })
}

fn base_command(ytdlp: &Path) -> std::process::Command {
    let mut cmd = tools::command(ytdlp);
    // Warnings stay on: a missing JavaScript runtime is only a warning.
    cmd.args(["--ignore-config", "--color", "never", "--no-playlist"])
        // Titles in any language, also on Windows.
        .env("PYTHONIOENCODING", "utf-8")
        .env("PYTHONUTF8", "1")
        .stdin(Stdio::null());
    if let Some((name, path)) = tools::js_runtime() {
        cmd.arg("--js-runtimes").arg(format!("{name}:{}", path.display()));
    }
    if let Some(ffmpeg) = tools::find(Tool::Ffmpeg) {
        cmd.arg("--ffmpeg-location").arg(ffmpeg);
    }
    // The static FFmpeg for macOS does not know where the system keeps its
    // certificates, and so fails on https (parts of videos, live streams).
    if cfg!(target_os = "macos")
        && std::env::var_os("SSL_CERT_FILE").is_none()
        && Path::new("/etc/ssl/cert.pem").is_file()
    {
        cmd.env("SSL_CERT_FILE", "/etc/ssl/cert.pem");
    }
    cmd
}

fn ytdlp() -> std::result::Result<PathBuf, MediaError> {
    tools::find(Tool::YtDlp).ok_or(MediaError {
        kind: ErrorKind::NoYtDlp,
        message: "yt-dlp is not installed: install it in Settings → Helpers.".into(),
    })
}

/// Reads what is behind `url` without downloading it.
pub fn inspect(
    url: &str,
    cookies_from: Option<&str>,
    proxy: Option<&str>,
    stop: &AtomicBool,
) -> std::result::Result<Found, MediaError> {
    let mut cmd = base_command(&ytdlp()?);
    cmd.args(["-J", "--flat-playlist"]);
    if let Some(p) = proxy.map(str::trim).filter(|p| !p.is_empty()) {
        cmd.args(["--proxy", p]);
    }
    if let Some(b) = cookies_from {
        cmd.args(["--cookies-from-browser", b]);
    }
    cmd.arg("--").arg(url);
    // Big channels take a while to list.
    let (status, stdout, stderr) = run_collect(cmd, stop, Duration::from_secs(300))
        .map_err(|e| MediaError { kind: ErrorKind::Other, message: format!("{e:#}") })?;
    if !status {
        return Err(classify(&stderr));
    }
    let v: Value = serde_json::from_str(&stdout)
        .map_err(|_| MediaError { kind: ErrorKind::Other, message: "yt-dlp gave an unexpected answer".into() })?;
    Ok(parse_info(&v))
}

fn parse_info(v: &Value) -> Found {
    let s = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    if v.get("_type").and_then(Value::as_str) == Some("playlist") {
        let entries = v
            .get("entries")
            .and_then(Value::as_array)
            .map(|es| {
                es.iter()
                    .filter_map(|e| {
                        let url = e.get("url").or_else(|| e.get("webpage_url")).and_then(Value::as_str)?;
                        Some(Entry {
                            title: e
                                .get("title")
                                .and_then(Value::as_str)
                                .map_or_else(|| title_from_url(url), str::to_string),
                            url: url.to_string(),
                            duration: e.get("duration").and_then(Value::as_f64),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        return Found::Playlist { title: s("title"), entries };
    }
    let formats = v.get("formats").and_then(Value::as_array).cloned().unwrap_or_default();
    let size = |f: &Value| f.get("filesize").or_else(|| f.get("filesize_approx")).and_then(Value::as_u64);
    let best_audio = formats
        .iter()
        .filter(|f| f.get("vcodec").and_then(Value::as_str) == Some("none"))
        .filter_map(size)
        .max()
        .unwrap_or(0);
    let mut heights: Vec<(u32, Option<u64>)> = Vec::new();
    for f in &formats {
        let Some(h) = f.get("height").and_then(Value::as_u64) else { continue };
        if f.get("vcodec").and_then(Value::as_str) == Some("none") || h == 0 {
            continue;
        }
        // yt-dlp's "res" (used to pick the quality) is the smaller side, so
        // a portrait 1080 × 1920 video is "1080p".
        let h = f.get("width").and_then(Value::as_u64).filter(|w| *w > 0).map_or(h, |w| w.min(h));
        let audio_inside = f.get("acodec").and_then(Value::as_str).is_some_and(|a| a != "none");
        let est = size(f).map(|s| if audio_inside { s } else { s + best_audio });
        match heights.iter_mut().find(|(x, _)| *x == h as u32) {
            Some((_, e)) => *e = (*e).max(est),
            None => heights.push((h as u32, est)),
        }
    }
    heights.sort_by_key(|h| std::cmp::Reverse(h.0));
    let uploader = ["uploader", "channel", "creator", "uploader_id"]
        .iter()
        .find_map(|k| v.get(*k).and_then(Value::as_str).filter(|x| !x.is_empty()))
        .unwrap_or("")
        .to_string();
    Found::Single(Info {
        title: s("title"),
        uploader,
        duration: v.get("duration").and_then(Value::as_f64),
        thumbnail: v.get("thumbnail").and_then(Value::as_str).map(str::to_string),
        site: s("extractor_key"),
        heights: heights.iter().map(|h| h.0).collect(),
        sizes: heights.iter().map(|h| h.1).collect(),
        is_live: v.get("is_live").and_then(Value::as_bool).unwrap_or(false),
    })
}

/// A readable name from a link's last part ("paradigm-shift" → "paradigm shift").
fn title_from_url(url: &str) -> String {
    let last = url.split(['?', '#']).next().unwrap_or(url).trim_end_matches('/').rsplit('/').next().unwrap_or(url);
    let name = crate::util::percent_decode(last).replace(['-', '_'], " ");
    if name.trim().is_empty() { url.to_string() } else { name }
}

/// What a running download reports.
pub trait Report: Sync {
    fn progress(&self, done: u64, total: Option<u64>, speed: Option<f64>, eta: Option<u64>);
    /// A step after downloading: "Merging", "Converting to MP3", ….
    fn stage(&self, _stage: &str) {}
    fn title(&self, _title: &str) {}
}

pub struct Job<'a> {
    pub url: &'a str,
    pub dir: &'a Path,
    pub options: &'a Options,
    /// Bytes per second; 0 is unlimited.
    pub rate_limit: u64,
    /// Parts of a stream fetched at once.
    pub fragments: usize,
    /// `http://…` or `socks5://…`.
    pub proxy: Option<&'a str>,
    /// The page the link was found on (sent by the browser extension).
    pub referer: Option<&'a str>,
}

/// The yt-dlp arguments for `job` (without the program and the link).
fn download_args(job: &Job, has_ffmpeg: bool) -> Vec<String> {
    let mut a: Vec<String> = vec![
        "--newline".into(),
        "--progress".into(),
        "--no-simulate".into(),
        "--no-mtime".into(),
        "--progress-template".into(),
        "download:[sdm] %(progress.downloaded_bytes)s|%(progress.total_bytes)s|%(progress.total_bytes_estimate)s|%(progress.speed)s|%(progress.eta)s|%(info.format_id)s".into(),
        "--progress-template".into(),
        "postprocess:[sdm-pp] %(progress.status)s|%(progress.postprocessor)s".into(),
        "--print".into(),
        "before_dl:[sdm-size] %(filesize,filesize_approx)s".into(),
        "--print".into(),
        "before_dl:[sdm-title] %(title)s".into(),
        "--print".into(),
        "after_move:[sdm-file] %(filepath)s".into(),
        "-P".into(),
        job.dir.display().to_string(),
        "-o".into(),
        // With the id: different posts often share a title ("Video by …"),
        // and yt-dlp would take an existing file for this one.
        "%(title).150B [%(id)s].%(ext)s".into(),
        "-N".into(),
        job.fragments.clamp(1, 16).to_string(),
    ];
    if job.rate_limit > 0 {
        a.extend(["-r".into(), job.rate_limit.to_string()]);
    }
    let o = job.options;
    if let Some(b) = &o.cookies_from {
        a.extend(["--cookies-from-browser".into(), b.clone()]);
    }
    if let Some(p) = job.proxy.filter(|p| !p.trim().is_empty()) {
        a.extend(["--proxy".into(), p.trim().to_string()]);
    }
    if let Some(r) = job.referer.filter(|r| !r.is_empty()) {
        a.extend(["--referer".into(), r.to_string()]);
    }
    if let Some((from, to)) = o.section.filter(|(f, t)| t > f) {
        a.extend(["--download-sections".into(), format!("*{from}-{to}")]);
    }
    if has_ffmpeg && o.sponsorblock {
        a.extend(["--sponsorblock-remove".into(), "sponsor,selfpromo,interaction".into()]);
    }
    match o.mode {
        Mode::Audio if has_ffmpeg => {
            let fmt = o.audio_format;
            a.extend(["-f".into(), "ba/b".into(), "-x".into(), "--audio-format".into(), fmt.name().into()]);
            if fmt.lossy() {
                let q = if o.audio_kbps == 0 { "0".to_string() } else { format!("{}K", o.audio_kbps) };
                a.extend(["--audio-quality".into(), q]);
            }
            a.push("--embed-metadata".into());
            // WAV has no place for a cover.
            if fmt != AudioFormat::Wav {
                a.push("--embed-thumbnail".into());
            }
        }
        // Without FFmpeg: the best sound as it is (usually M4A).
        Mode::Audio => a.extend(["-f".into(), "ba[ext=m4a]/ba/b".into()]),
        Mode::Video => {
            let h = o.max_height;
            let res = h.map_or("res".to_string(), |h| format!("res:{h}"));
            // Same height: smooth first; then the codecs that suit the file type
            // (MP4: H.264 and AAC, which every player plays).
            let codecs = match o.container {
                Container::Mp4 => ",vcodec:h264,acodec:m4a",
                Container::Webm => ",vcodec:vp9,acodec:opus",
                Container::Mkv => "",
            };
            a.extend(["-S".into(), format!("{res},fps{codecs}")]);
            if !o.subtitles.trim().is_empty() {
                a.extend(["--write-subs".into(), "--write-auto-subs".into()]);
                a.extend(["--sub-langs".into(), o.subtitles.trim().replace(' ', "")]);
                if has_ffmpeg {
                    a.push("--embed-subs".into());
                }
            }
            if has_ffmpeg {
                a.extend(["--merge-output-format".into(), o.container.name().into(), "--embed-metadata".into()]);
                a.push("--embed-chapters".into());
            } else {
                // Separate video and sound cannot be joined: pick one file with both.
                let f = h.map_or("b".to_string(), |h| format!("b[height<={h}]/b"));
                a.extend(["-f".into(), f]);
            }
        }
    }
    a
}

/// Values of a `[sdm]` progress line.
#[derive(Debug, PartialEq)]
struct Line {
    done: u64,
    total: Option<u64>,
    speed: Option<f64>,
    eta: Option<u64>,
    format: String,
}

fn parse_progress(line: &str) -> Option<Line> {
    let rest = line.strip_prefix("[sdm] ")?;
    let mut p = rest.split('|');
    let num = |s: Option<&str>| s.and_then(|s| s.trim().parse::<f64>().ok()).filter(|v| v.is_finite() && *v >= 0.0);
    let done = num(p.next())? as u64;
    let total = num(p.next()).map(|v| v as u64);
    let estimate = num(p.next()).map(|v| v as u64);
    let speed = num(p.next());
    let eta = num(p.next()).map(|v| v as u64);
    let format = p.next().unwrap_or("").trim().to_string();
    Some(Line { done, total: total.or(estimate).filter(|t| *t > 0), speed, eta, format })
}

/// Downloads with yt-dlp. Returns the saved file, or `None` when stopped
/// (yt-dlp continues from its `.part` file next time).
pub fn download(job: Job, stop: &AtomicBool, report: &dyn Report) -> std::result::Result<Option<PathBuf>, MediaError> {
    let other = |e: anyhow::Error| MediaError { kind: ErrorKind::Other, message: format!("{e:#}") };
    std::fs::create_dir_all(job.dir).map_err(|e| other(anyhow!("cannot create {}: {e}", job.dir.display())))?;
    let mut cmd = base_command(&ytdlp()?);
    cmd.args(download_args(&job, tools::find(Tool::Ffmpeg).is_some()));
    cmd.arg("--").arg(job.url);
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = spawn(cmd).map_err(other)?;

    let file: Arc<Mutex<Option<PathBuf>>> = Arc::default();
    let errors: Arc<Mutex<String>> = Arc::default();
    let stdout = child.stdout.take().expect("piped");
    let stderr = child.stderr.take().expect("piped");
    std::thread::scope(|scope| {
        let file = file.clone();
        scope.spawn(move || {
            // A video and its sound are separate downloads: add them up.
            let mut overall: Option<u64> = None;
            let (mut base, mut last_format, mut last_total) = (0u64, String::new(), 0u64);
            for line in BufReader::new(stdout).lines().map_while(|l| l.ok()) {
                if let Some(p) = parse_progress(&line) {
                    if !last_format.is_empty() && p.format != last_format {
                        base += last_total;
                    }
                    last_format = p.format.clone();
                    last_total = p.total.unwrap_or(p.done);
                    let total = match (overall, p.total) {
                        (Some(o), Some(t)) => Some(o.max(base + t)),
                        (Some(o), None) => Some(o),
                        (None, t) => t.map(|t| base + t),
                    };
                    report.progress(base + p.done, total, p.speed, p.eta);
                } else if let Some(stage) = postprocess_stage(&line) {
                    report.stage(stage);
                } else if let Some(size) = line.strip_prefix("[sdm-size] ") {
                    overall = size.trim().parse::<f64>().ok().map(|v| v as u64).filter(|v| *v > 0);
                } else if let Some(title) = line.strip_prefix("[sdm-title] ") {
                    report.title(title.trim());
                } else if let Some(path) = line.strip_prefix("[sdm-file] ") {
                    *file.lock().unwrap_or_else(|e| e.into_inner()) = Some(PathBuf::from(path.trim()));
                }
            }
        });
        let errors = errors.clone();
        scope.spawn(move || {
            for line in BufReader::new(stderr).lines().map_while(|l| l.ok()) {
                // In quiet mode (implied by --print) yt-dlp writes these here.
                if line.starts_with("[sdm-pp] ") {
                    if let Some(stage) = postprocess_stage(&line) {
                        report.stage(stage);
                    }
                    continue;
                }
                let mut e = errors.lock().unwrap_or_else(|e| e.into_inner());
                if e.len() < 64 * 1024 {
                    e.push_str(&line);
                    e.push('\n');
                }
            }
        });
        // Wait, stopping yt-dlp when asked.
        loop {
            if stop.load(Ordering::Relaxed) {
                kill_tree(&mut child);
                break;
            }
            match child.try_wait() {
                Ok(Some(_)) | Err(_) => break,
                Ok(None) => std::thread::sleep(Duration::from_millis(150)),
            }
        }
    });
    if stop.load(Ordering::Relaxed) {
        return Ok(None);
    }
    let status = child.wait().map_err(|e| other(e.into()))?;
    let errors = errors.lock().unwrap_or_else(|e| e.into_inner()).clone();
    if !status.success() {
        return Err(classify(&errors));
    }
    let file = file.lock().unwrap_or_else(|e| e.into_inner()).take();
    match file {
        Some(f) => Ok(Some(f)),
        None => Err(MediaError { kind: ErrorKind::Other, message: "yt-dlp finished but saved no file".into() }),
    }
}

/// Starts `cmd` in its own process group, so that stopping it also stops
/// what it started (yt-dlp's standalone build runs itself as a child).
fn spawn(mut cmd: std::process::Command) -> Result<Child> {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    cmd.spawn().context("could not start yt-dlp")
}

/// Stops `child` and everything it started.
fn kill_tree(child: &mut Child) {
    #[cfg(unix)]
    {
        let pgid = child.id() as libc::pid_t;
        // SAFETY: signals the process group created by `spawn`.
        unsafe { libc::kill(-pgid, libc::SIGTERM) };
        let until = Instant::now() + Duration::from_secs(3);
        while Instant::now() < until {
            if matches!(child.try_wait(), Ok(Some(_))) {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        // SAFETY: as above.
        unsafe { libc::kill(-pgid, libc::SIGKILL) };
    }
    #[cfg(windows)]
    {
        let _ = tools::command("taskkill")
            .args(["/T", "/F", "/PID", &child.id().to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    let _ = child.kill();
    let _ = child.wait();
}

/// Runs `cmd` to the end and returns (success, stdout, stderr).
fn run_collect(mut cmd: std::process::Command, stop: &AtomicBool, limit: Duration) -> Result<(bool, String, String)> {
    use std::io::Read;
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = spawn(cmd)?;
    let mut stdout = child.stdout.take().expect("piped");
    let mut stderr = child.stderr.take().expect("piped");
    let started = Instant::now();
    let (out, err, stopped) = std::thread::scope(|scope| {
        let out = scope.spawn(move || {
            let mut s = String::new();
            let _ = stdout.read_to_string(&mut s);
            s
        });
        let err = scope.spawn(move || {
            let mut s = String::new();
            let _ = stderr.read_to_string(&mut s);
            s
        });
        let mut stopped = false;
        loop {
            if stop.load(Ordering::Relaxed) || started.elapsed() > limit {
                stopped = true;
                kill_tree(&mut child);
                break;
            }
            match child.try_wait() {
                Ok(Some(_)) | Err(_) => break,
                Ok(None) => std::thread::sleep(Duration::from_millis(100)),
            }
        }
        (out.join().unwrap_or_default(), err.join().unwrap_or_default(), stopped)
    });
    if stopped {
        anyhow::bail!(if stop.load(Ordering::Relaxed) { "cancelled" } else { "the site took too long to answer" });
    }
    let ok = child.wait().map(|s| s.success()).unwrap_or(false);
    Ok((ok, out, err))
}

/// Whether yt-dlp handles links to this site itself (rather than SwiftDM
/// downloading the link as a file).
pub fn is_media_site(url: &str) -> bool {
    // Only links to the site itself (http or https), not any link that
    // mentions it.
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return false;
    }
    const SITES: &[&str] = &[
        "youtube.com",
        "youtu.be",
        "vimeo.com",
        "twitter.com",
        "x.com",
        "instagram.com",
        "tiktok.com",
        "soundcloud.com",
        "twitch.tv",
        "dailymotion.com",
        "facebook.com",
        "fb.watch",
        "reddit.com",
        "pinterest.com",
        "tumblr.com",
        "bilibili.com",
        "bandcamp.com",
        "streamable.com",
        "ok.ru",
        "vk.com",
        "threads.net",
        "snapchat.com",
        "linkedin.com",
        "rumble.com",
        "kick.com",
        "mixcloud.com",
        "9gag.com",
        "imgur.com",
        "music.apple.com",
        "dai.ly",
    ];
    let host = crate::util::host(url).to_ascii_lowercase();
    SITES.iter().any(|s| host == *s || host.ends_with(&format!(".{s}")))
}

/// How a link is best downloaded.
pub enum Route {
    /// Through yt-dlp: a video site, or a web page that may hold a video.
    Media,
    /// As a plain file.
    File(crate::http::Probe),
}

/// Decides how to download `url`, asking its server if the link alone
/// does not tell.
pub fn route(url: &str, net: &crate::http::Net) -> Route {
    if is_media_site(url) {
        return Route::Media;
    }
    match crate::http::probe_with(url, net) {
        Ok(p) if !p.is_web_page() => Route::File(p),
        // A web page (yt-dlp may find a video in it), or unreachable
        // (yt-dlp will say why).
        _ => Route::Media,
    }
}

/// Whether the link's path ends in a file name with a known extension.
pub fn looks_like_file(url: &str) -> bool {
    let path = url.split(['?', '#']).next().unwrap_or(url);
    let Some(last) =
        path.split("://").nth(1).and_then(|p| p.split_once('/')).map(|(_, p)| p.rsplit('/').next().unwrap_or(""))
    else {
        return false;
    };
    let ext = Path::new(last).extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    !ext.is_empty() && crate::util::FileKind::of(last) != crate::util::FileKind::Other
        || matches!(ext.as_str(), "bin" | "img" | "torrent" | "vsix" | "jar" | "whl")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_lines() {
        assert_eq!(
            parse_progress("[sdm] 1048576|NA|4194304.0|524288.5|6|137"),
            Some(Line {
                done: 1048576,
                total: Some(4194304),
                speed: Some(524288.5),
                eta: Some(6),
                format: "137".into()
            })
        );
        assert_eq!(parse_progress("[sdm] NA|NA|NA|NA|NA|18"), None);
        assert_eq!(parse_progress("[download] 5%"), None);
    }

    #[test]
    fn errors_are_explained() {
        let e = classify("WARNING: x\nERROR: [youtube] abc: Sign in to confirm you're not a bot\n");
        assert_eq!(e.kind, ErrorKind::Login);
        let e = classify("ERROR: Unsupported URL: https://example.com/page\n");
        assert_eq!(e.kind, ErrorKind::Unsupported);
        let e = classify("ERROR: [youtube] abc: Private video. Sign in if you've been granted access\n");
        assert_eq!(e.kind, ErrorKind::Login);
        let e = classify("ERROR: [vimeo] 123: Video unavailable\n");
        assert_eq!(e.kind, ErrorKind::Unavailable);
        let e = classify("ERROR: could not find opera cookies database in \"/x\"\n");
        assert_eq!(e.kind, ErrorKind::Cookies);
        let e = classify("ERROR: Could not copy Chrome cookie database. See …\n");
        assert_eq!(e.kind, ErrorKind::Cookies);
        let e = classify(
            "WARNING: [youtube] No supported JavaScript runtime could be found\nERROR: Requested format is not available\n",
        );
        assert_eq!(e.kind, ErrorKind::NeedsJs);
        let e = classify("ERROR: [youtube] x: This video is unavailable\n");
        assert_eq!(e.kind, ErrorKind::Unavailable);
        assert_eq!(postprocess_stage("[sdm-pp] started|ExtractAudio"), Some("Converting the sound"));
        assert_eq!(postprocess_stage("[sdm-pp] finished|ExtractAudio"), None);
        assert_eq!(title_from_url("https://soundcloud.com/forss/paradigm-shift"), "paradigm shift");
        let e = classify("ERROR: [generic] something odd happened\n");
        assert_eq!(e.kind, ErrorKind::Other);
        assert_eq!(e.message, "something odd happened");
    }

    #[test]
    fn info_from_json() {
        let v: Value = serde_json::from_str(
            r#"{"title":"T","uploader":"U","duration":61.0,"extractor_key":"Youtube","formats":[
                {"vcodec":"none","acodec":"opus","filesize":100},
                {"height":720,"vcodec":"avc1","acodec":"none","filesize":1000},
                {"height":1080,"vcodec":"vp9","acodec":"none","filesize_approx":3000},
                {"height":360,"vcodec":"avc1","acodec":"mp4a","filesize":50}]}"#,
        )
        .unwrap();
        let Found::Single(i) = parse_info(&v) else { panic!() };
        assert_eq!(i.heights, vec![1080, 720, 360]);
        assert_eq!(i.sizes, vec![Some(3100), Some(1100), Some(50)]);
        assert_eq!(i.uploader, "U");
        // Portrait: the smaller side counts.
        let v: Value = serde_json::from_str(
            r#"{"title":"S","formats":[{"width":1080,"height":1920,"vcodec":"avc1","acodec":"none","filesize":40},
                {"width":480,"height":854,"vcodec":"avc1","acodec":"none","filesize":10}]}"#,
        )
        .unwrap();
        let Found::Single(i) = parse_info(&v) else { panic!() };
        assert_eq!((i.heights, i.sizes), (vec![1080, 480], vec![Some(40), Some(10)]));
        let v: Value = serde_json::from_str(
            r#"{"_type":"playlist","title":"P","entries":[{"title":"a","url":"https://y/1"},{"url":"https://y/2"}]}"#,
        )
        .unwrap();
        let Found::Playlist { title, entries } = parse_info(&v) else { panic!() };
        assert_eq!((title.as_str(), entries.len(), entries[1].title.as_str()), ("P", 2, "2"));
    }

    #[test]
    fn sites_and_files() {
        assert!(is_media_site("https://www.youtube.com/watch?v=x"));
        assert!(is_media_site("https://m.facebook.com/x"));
        assert!(!is_media_site("https://notyoutube.com/x"));
        assert!(looks_like_file("https://cdn.example.com/a/setup.exe?token=1"));
        assert!(!looks_like_file("https://example.com/watch"));
        assert!(!looks_like_file("https://example.com"));
    }

    #[test]
    fn arguments_follow_the_options() {
        let dir = PathBuf::from("/tmp/x");
        let opts = Options { mode: Mode::Audio, audio_kbps: 192, ..Default::default() };
        let job = Job { url: "u", dir: &dir, options: &opts, rate_limit: 0, fragments: 4, proxy: None, referer: None };
        let a = download_args(&job, true).join(" ");
        assert!(a.contains("--audio-format mp3") && a.contains("--audio-quality 192K"));
        let opts = Options { max_height: Some(720), ..Default::default() };
        let job =
            Job { url: "u", dir: &dir, options: &opts, rate_limit: 1000, fragments: 4, proxy: None, referer: None };
        let a = download_args(&job, false).join(" ");
        assert!(a.contains("-S res:720,") && a.contains("-f b[height<=720]/b") && a.contains("-r 1000"));
        let opts = Options {
            container: Container::Mkv,
            subtitles: "en, ar".into(),
            sponsorblock: true,
            section: Some((60, 90)),
            ..Default::default()
        };
        let job = Job {
            url: "u",
            dir: &dir,
            options: &opts,
            rate_limit: 0,
            fragments: 4,
            proxy: Some("socks5://127.0.0.1:9050"),
            referer: Some("https://page/"),
        };
        let a = download_args(&job, true).join(" ");
        for want in [
            "--merge-output-format mkv",
            "--sub-langs en,ar",
            "--embed-subs",
            "--sponsorblock-remove",
            "--download-sections *60-90",
            "--proxy socks5://127.0.0.1:9050",
            "--referer https://page/",
        ] {
            assert!(a.contains(want), "{want} in {a}");
        }
        let opts =
            Options { mode: Mode::Audio, audio_format: AudioFormat::Flac, audio_kbps: 320, ..Default::default() };
        let job = Job { url: "u", dir: &dir, options: &opts, rate_limit: 0, fragments: 4, proxy: None, referer: None };
        let a = download_args(&job, true).join(" ");
        assert!(a.contains("--audio-format flac") && !a.contains("--audio-quality"));
    }
}
