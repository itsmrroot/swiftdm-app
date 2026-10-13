//! Direct file downloads over HTTP(S): several connections at once when the
//! server allows byte ranges, resumable after a pause, an app restart or a
//! lost connection.
//!
//! The file is written to `<name>.part` and renamed when complete. When the
//! server supports ranges, the file is split into segments that run in
//! parallel; a connection that finishes early takes over half of the largest
//! remaining segment, so all connections stay busy until the end.

use std::fs::{File, OpenOptions};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};

use crate::util::{part_path, percent_decode, sanitize_filename, unique_path};

/// Some sites refuse requests that do not look like a browser.
pub const USER_AGENT: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";

/// Segments are not split below this size.
const MIN_SPLIT: u64 = 1024 * 1024;
const CHUNK: usize = 256 * 1024;
/// A ranged request is renewed after this long, so that a connection that
/// silently stalls is replaced (ureq has no idle-read timeout).
const REQUEST_SPAN: Duration = Duration::from_secs(60);
/// Attempts in a row without any progress before a connection gives up.
const MAX_FAILURES: u32 = 6;
/// A server that sends nothing for this long is given up on.
const STALL: Duration = Duration::from_secs(60);

fn agent(body_timeout: Option<Duration>) -> ureq::Agent {
    ureq::Agent::config_builder()
        .user_agent(USER_AGENT)
        // Byte ranges must refer to the file itself, never a compressed copy.
        .accept_encoding("identity")
        .timeout_connect(Some(Duration::from_secs(20)))
        .timeout_recv_response(Some(Duration::from_secs(30)))
        .timeout_recv_body(body_timeout)
        .http_status_as_error(false)
        .build()
        .into()
}

/// What a link points to, found out before downloading.
#[derive(Debug, Clone)]
pub struct Probe {
    /// After redirects.
    pub url: String,
    /// A file name for it (from the server, the link or its type).
    pub name: String,
    pub size: Option<u64>,
    /// Whether the server allows byte ranges: several connections and resuming.
    pub resumable: bool,
    pub content_type: String,
    /// `ETag` or `Last-Modified`: changes when the file on the server does.
    pub validator: Option<String>,
}

impl Probe {
    /// The link opens a web page rather than a file.
    pub fn is_web_page(&self) -> bool {
        self.content_type.starts_with("text/html")
    }
}

/// Asks the server about `url` without downloading it.
pub fn probe(url: &str) -> Result<Probe> {
    use ureq::ResponseExt;
    let agent = agent(Some(Duration::from_secs(20)));
    let mut resp = agent.get(url).header("Range", "bytes=0-0").call().map_err(net_error)?;
    if resp.status().as_u16() == 416 {
        // Some servers refuse the range of an empty file.
        resp = agent.get(url).call().map_err(net_error)?;
    }
    let status = resp.status().as_u16();
    if status >= 400 {
        bail!("{}", status_message(status));
    }
    let header = |name: &str| resp.headers().get(name).and_then(|v| v.to_str().ok()).map(str::to_string);
    let final_url = resp.get_uri().to_string();
    let content_type = header("content-type").unwrap_or_default().to_ascii_lowercase();
    let (size, resumable) = if status == 206 {
        (header("content-range").and_then(|r| parse_content_range_total(&r)), true)
    } else {
        (header("content-length").and_then(|l| l.trim().parse().ok()), false)
    };
    let validator = header("etag").or_else(|| header("last-modified"));
    let name = header("content-disposition")
        .and_then(|cd| filename_from_disposition(&cd))
        .or_else(|| filename_from_url(&final_url))
        .or_else(|| filename_from_url(url))
        .unwrap_or_else(|| "download".into());
    let name = with_extension_for(&sanitize_filename(&name), &content_type);
    let resumable = resumable && size.is_some_and(|s| s > 0);
    Ok(Probe { url: final_url, name, size, resumable, content_type, validator })
}

/// Saved state of an unfinished ranged download.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Resume {
    pub size: u64,
    /// `[start, next byte to write, end (inclusive)]`.
    pub segments: Vec<[u64; 3]>,
    /// The server's `ETag` or `Last-Modified` when the download began.
    #[serde(default)]
    pub validator: Option<String>,
}

impl Resume {
    pub fn done(&self) -> u64 {
        self.segments.iter().map(|[start, pos, end]| (*pos).min(end + 1).saturating_sub(*start)).sum()
    }
}

/// Global download speed limit shared by all downloads (bytes per second; 0
/// is unlimited).
#[derive(Default)]
pub struct Limiter {
    rate: AtomicU64,
    bucket: Mutex<Option<(Instant, f64)>>,
}

impl Limiter {
    pub fn set_rate(&self, bytes_per_sec: u64) {
        self.rate.store(bytes_per_sec, Ordering::Relaxed);
    }

    pub fn rate(&self) -> u64 {
        self.rate.load(Ordering::Relaxed)
    }

    /// How many bytes to read at a time: small enough that a slow limit is
    /// kept smoothly by many short waits rather than a few long ones.
    pub fn chunk(&self) -> usize {
        match self.rate() {
            0 => CHUNK,
            r => ((r / 4) as usize).clamp(4096, CHUNK),
        }
    }

    /// Waits as long as needed to stay under the limit after `n` more bytes.
    pub fn take(&self, n: usize) {
        let rate = self.rate() as f64;
        if rate <= 0.0 {
            return;
        }
        let wait = {
            let mut b = self.bucket.lock().unwrap_or_else(|e| e.into_inner());
            let now = Instant::now();
            let (last, tokens) = b.get_or_insert((now, rate));
            // At most one second of burst.
            *tokens = (*tokens + now.duration_since(*last).as_secs_f64() * rate).min(rate) - n as f64;
            *last = now;
            if *tokens < 0.0 { -*tokens / rate } else { 0.0 }
        };
        if wait > 0.0 {
            // Reads of at most `chunk()` keep this short; the bound only
            // guards against a limit lowered mid-download.
            std::thread::sleep(Duration::from_secs_f64(wait.min(30.0)));
        }
    }
}

/// How a download reports back.
pub trait Report: Sync {
    /// Bytes written so far and the size, if known.
    fn progress(&self, done: u64, total: Option<u64>);
    /// The state to save, so that the download can continue after a restart.
    fn checkpoint(&self, _resume: &Resume) {}
}

/// How a download ended, when it did not fail.
#[derive(Debug)]
pub enum Outcome {
    /// Complete, at this path.
    Finished(PathBuf),
    /// Stopped on request; `Some` can be continued later.
    Stopped(Option<Resume>),
}

/// A failed download, with what is needed to continue it if possible.
#[derive(Debug)]
pub struct Failure {
    pub error: anyhow::Error,
    pub resume: Option<Resume>,
}

impl<E: Into<anyhow::Error>> From<E> for Failure {
    fn from(e: E) -> Self {
        Failure { error: e.into(), resume: None }
    }
}

pub struct Job<'a> {
    pub url: &'a str,
    /// Where the finished file goes.
    pub path: &'a Path,
    pub connections: usize,
    /// From an earlier, unfinished attempt.
    pub resume: Option<Resume>,
}

/// Downloads `job.url` to `job.path`.
pub fn download(
    job: Job,
    stop: &AtomicBool,
    limiter: &Limiter,
    report: &dyn Report,
) -> std::result::Result<Outcome, Failure> {
    // Failures before any data arrives keep the earlier resume point.
    let keep = |error: anyhow::Error| Failure { error, resume: job.resume.clone() };
    let probe = probe(job.url).map_err(keep)?;
    if stop.load(Ordering::Relaxed) {
        return Ok(Outcome::Stopped(job.resume));
    }
    if let Some(dir) = job.path.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("cannot create {}", dir.display())).map_err(keep)?;
    }
    let part = part_path(job.path);
    let outcome = match probe.size.filter(|_| probe.resumable) {
        Some(size) => {
            let target = Target { url: &probe.url, part: &part, size, validator: probe.validator.clone() };
            ranged(target, job.connections.clamp(1, 32), job.resume, stop, limiter, report)?
        }
        None => single(&probe.url, &part, probe.size, stop, limiter, report)?,
    };
    if let Step::Stopped(resume) = outcome {
        return Ok(Outcome::Stopped(resume));
    }
    // Another file of this name may have appeared meanwhile.
    let target = if job.path.exists() {
        let dir = job.path.parent().unwrap_or(Path::new("."));
        unique_path(dir, &job.path.file_name().unwrap_or_default().to_string_lossy())
    } else {
        job.path.to_path_buf()
    };
    std::fs::rename(&part, &target).with_context(|| format!("cannot move the file to {}", target.display()))?;
    Ok(Outcome::Finished(target))
}

/// How [`single`] or [`ranged`] ended, when it did not fail.
enum Step {
    Complete,
    Stopped(Option<Resume>),
}

/// One connection, no resuming: for servers without byte ranges.
fn single(
    url: &str,
    part: &Path,
    size: Option<u64>,
    stop: &AtomicBool,
    limiter: &Limiter,
    report: &dyn Report,
) -> std::result::Result<Step, Failure> {
    // Bounded so that a reader thread left behind (see below) ends some day.
    let resp = agent(Some(Duration::from_secs(6 * 3600))).get(url).call().map_err(net_error)?;
    let status = resp.status().as_u16();
    if status >= 400 {
        return Err(anyhow!("{}", status_message(status)).into());
    }
    let mut file = File::create(part).with_context(|| format!("cannot write {}", part.display()))?;
    // The body is read on its own thread: a server that stalls would
    // otherwise block a read, and with it pausing and quitting.
    let mut reader = resp.into_body().into_reader();
    let (tx, rx) = std::sync::mpsc::sync_channel::<std::io::Result<Vec<u8>>>(8);
    std::thread::spawn(move || {
        loop {
            let mut buf = vec![0; CHUNK];
            let piece = reader.read(&mut buf).map(|n| {
                buf.truncate(n);
                buf
            });
            let end = !matches!(&piece, Ok(b) if !b.is_empty());
            // A closed channel: the download was stopped.
            if tx.send(piece).is_err() || end {
                return;
            }
        }
    });
    let mut done = 0u64;
    let mut last_report = Instant::now();
    let mut last_data = Instant::now();
    loop {
        if stop.load(Ordering::Relaxed) {
            return Ok(Step::Stopped(None));
        }
        match rx.recv_timeout(Duration::from_millis(250)) {
            Ok(Ok(data)) if data.is_empty() => break,
            Ok(Ok(data)) => {
                for piece in data.chunks(limiter.chunk()) {
                    limiter.take(piece.len());
                }
                std::io::Write::write_all(&mut file, &data).context("could not write the file (is the disk full?)")?;
                done += data.len() as u64;
                last_data = Instant::now();
            }
            Ok(Err(e)) => return Err(anyhow!("the connection was lost ({e})").into()),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                if last_data.elapsed() > STALL {
                    return Err(anyhow!("the server stopped sending data").into());
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                return Err(anyhow!("the connection was lost ({})", "closed").into());
            }
        }
        if last_report.elapsed() >= Duration::from_millis(250) {
            report.progress(done, size);
            last_report = Instant::now();
        }
    }
    if size.is_some_and(|s| s != done) {
        return Err(anyhow!("the download ended before the file was complete").into());
    }
    file.sync_all().ok();
    report.progress(done, Some(done));
    Ok(Step::Complete)
}

struct Seg {
    start: u64,
    pos: u64,
    end: u64,
    taken: bool,
}

impl Seg {
    fn remaining(&self) -> u64 {
        (self.end + 1).saturating_sub(self.pos)
    }
}

struct Shared<'a> {
    url: &'a str,
    file: File,
    segs: Mutex<Vec<Seg>>,
    /// Set to stop all connections: on request or after a failure.
    halt: AtomicBool,
    stop: &'a AtomicBool,
    failure: Mutex<Option<anyhow::Error>>,
    limiter: &'a Limiter,
    validator: Option<String>,
}

impl Shared<'_> {
    fn halted(&self) -> bool {
        self.halt.load(Ordering::Relaxed) || self.stop.load(Ordering::Relaxed)
    }

    fn segs(&self) -> std::sync::MutexGuard<'_, Vec<Seg>> {
        self.segs.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn snapshot(&self, size: u64) -> Resume {
        let segments = self.segs().iter().map(|s| [s.start, s.pos, s.end]).collect();
        Resume { size, segments, validator: self.validator.clone() }
    }

    /// A segment for a free connection: an untaken one, or the second half
    /// of the largest one still running.
    fn claim(&self) -> Option<usize> {
        let mut segs = self.segs();
        if let Some(i) = segs.iter().position(|s| !s.taken && s.remaining() > 0) {
            segs[i].taken = true;
            return Some(i);
        }
        let (i, rem) = segs.iter().enumerate().map(|(i, s)| (i, s.remaining())).max_by_key(|(_, r)| *r)?;
        if rem < 2 * MIN_SPLIT {
            return None;
        }
        let mid = segs[i].pos + rem / 2;
        let end = segs[i].end;
        segs[i].end = mid - 1;
        segs.push(Seg { start: mid, pos: mid, end, taken: true });
        Some(segs.len() - 1)
    }

    /// Runs one connection until no work is left.
    fn connection(&self) {
        let mut buf = vec![0; CHUNK];
        while let Some(i) = self.claim() {
            if let Err(e) = self.segment(i, &mut buf) {
                let mut f = self.failure.lock().unwrap_or_else(|e| e.into_inner());
                f.get_or_insert(e);
                self.halt.store(true, Ordering::Relaxed);
            }
            self.segs()[i].taken = false;
            if self.halted() {
                return;
            }
        }
    }

    /// Downloads segment `i`, reconnecting as needed.
    fn segment(&self, i: usize, buf: &mut [u8]) -> Result<()> {
        let agent = agent(Some(REQUEST_SPAN));
        let mut failures = 0;
        loop {
            let (pos, end) = {
                let s = &self.segs()[i];
                (s.pos, s.end)
            };
            if pos > end || self.halted() {
                return Ok(());
            }
            let result = self.request(&agent, i, pos, end, buf);
            if self.halted() {
                return Ok(());
            }
            let e = match result {
                Ok(true) => {
                    failures = 0;
                    continue;
                }
                // Answered, but with nothing: as bad as no answer.
                Ok(false) => anyhow!("the server sent no data"),
                Err(e) => e,
            };
            let fatal = e.downcast_ref::<Fatal>().is_some();
            failures += 1;
            if fatal || failures >= MAX_FAILURES {
                return Err(e);
            }
            log::info!("connection retry {failures}: {e:#}");
            // Wait a little longer each time, but react to a stop.
            let until = Instant::now() + Duration::from_secs(1 << failures.min(4));
            while Instant::now() < until && !self.halted() {
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    }

    /// One request for bytes `pos..=end` of segment `i`. Returns whether any
    /// bytes arrived; a timeout after progress is a normal renewal.
    fn request(&self, agent: &ureq::Agent, i: usize, pos: u64, end: u64, buf: &mut [u8]) -> Result<bool> {
        let mut resp = agent.get(self.url).header("Range", format!("bytes={pos}-{end}")).call().map_err(net_error)?;
        match resp.status().as_u16() {
            206 => {}
            200 => return Err(Fatal("the server stopped allowing resumed downloads".into()).into()),
            s @ (401 | 403 | 404 | 410) => return Err(Fatal(status_message(s)).into()),
            s => bail!("{}", status_message(s)),
        }
        // The part asked for, not another one.
        let start = resp
            .headers()
            .get("content-range")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.trim().strip_prefix("bytes ")?.split('-').next()?.trim().parse::<u64>().ok());
        if start != Some(pos) {
            return Err(Fatal("the server sent the wrong part of the file".into()).into());
        }
        let chunk = self.limiter.chunk().min(buf.len());
        let buf = &mut buf[..chunk];
        let mut body = resp.body_mut().as_reader();
        let mut progressed = false;
        loop {
            if self.halted() {
                return Ok(progressed);
            }
            let n = match body.read(buf) {
                Ok(0) => return Ok(progressed),
                Ok(n) => n,
                Err(_) if progressed => return Ok(true),
                Err(e) => bail!("the connection was lost ({e})"),
            };
            self.limiter.take(n);
            // The segment may have been split meanwhile: write only up to its end.
            let (offset, len) = {
                let s = &self.segs()[i];
                (s.pos, (n as u64).min(s.remaining()) as usize)
            };
            if len > 0 {
                write_at(&self.file, &buf[..len], offset).context("could not write the file (is the disk full?)")?;
                progressed = true;
            }
            let mut segs = self.segs();
            segs[i].pos = offset + len as u64;
            if segs[i].remaining() == 0 {
                return Ok(true);
            }
        }
    }
}

/// An error that retrying will not fix.
#[derive(Debug)]
struct Fatal(String);

impl std::fmt::Display for Fatal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Fatal {}

/// The file of a ranged download.
struct Target<'a> {
    url: &'a str,
    part: &'a Path,
    size: u64,
    validator: Option<String>,
}

/// Several connections over byte ranges.
fn ranged(
    target: Target,
    connections: usize,
    resume: Option<Resume>,
    stop: &AtomicBool,
    limiter: &Limiter,
    report: &dyn Report,
) -> std::result::Result<Step, Failure> {
    let Target { url, part, size, validator } = target;
    let keep = |error: anyhow::Error| Failure { error, resume: resume.clone() };
    // An earlier attempt is only continued if its file is still there and
    // the file on the server has not changed.
    let unchanged = |r: &Resume| match (&r.validator, &validator) {
        (Some(a), Some(b)) => a == b,
        _ => true,
    };
    let earlier = resume.clone().filter(|r| {
        r.size == size
            && unchanged(r)
            && !r.segments.is_empty()
            && std::fs::metadata(part).is_ok_and(|m| m.len() == size)
    });
    let file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(earlier.is_none())
        .open(part)
        .with_context(|| format!("cannot write {}", part.display()))
        .map_err(keep)?;
    let segs = match earlier {
        Some(r) => r.segments.iter().map(|&[start, pos, end]| Seg { start, pos, end, taken: false }).collect(),
        None => {
            file.set_len(size).context("not enough disk space for this file").map_err(keep)?;
            initial_segments(size, connections)
        }
    };
    let shared = Shared {
        url,
        file,
        segs: Mutex::new(segs),
        halt: AtomicBool::new(false),
        stop,
        failure: Mutex::new(None),
        limiter,
        validator,
    };
    std::thread::scope(|scope| {
        let handles: Vec<_> = (0..connections).map(|_| scope.spawn(|| shared.connection())).collect();
        // Report while the connections run.
        let mut last_checkpoint = Instant::now();
        while !handles.iter().all(|h| h.is_finished()) {
            std::thread::sleep(Duration::from_millis(250));
            let snap = shared.snapshot(size);
            report.progress(snap.done(), Some(size));
            if last_checkpoint.elapsed() >= Duration::from_secs(3) {
                report.checkpoint(&snap);
                last_checkpoint = Instant::now();
            }
        }
    });
    let snap = shared.snapshot(size);
    report.progress(snap.done(), Some(size));
    let failure = shared.failure.lock().unwrap_or_else(|e| e.into_inner()).take();
    let complete = snap.segments.iter().all(|[_, pos, end]| pos > end);
    if complete {
        shared.file.sync_all().ok();
        return Ok(Step::Complete);
    }
    report.checkpoint(&snap);
    if let Some(error) = failure {
        return Err(Failure { error, resume: Some(snap) });
    }
    if stop.load(Ordering::Relaxed) {
        return Ok(Step::Stopped(Some(snap)));
    }
    Err(Failure { error: anyhow!("the download ended before the file was complete"), resume: Some(snap) })
}

/// `connections` equal parts, none smaller than `MIN_SPLIT`.
fn initial_segments(size: u64, connections: usize) -> Vec<Seg> {
    let n = (size / MIN_SPLIT).clamp(1, connections.max(1) as u64);
    let step = size / n;
    (0..n)
        .map(|i| {
            let start = i * step;
            let end = if i == n - 1 { size - 1 } else { start + step - 1 };
            Seg { start, pos: start, end, taken: false }
        })
        .collect()
}

#[cfg(unix)]
fn write_at(file: &File, buf: &[u8], offset: u64) -> std::io::Result<()> {
    std::os::unix::fs::FileExt::write_all_at(file, buf, offset)
}

#[cfg(windows)]
fn write_at(file: &File, mut buf: &[u8], mut offset: u64) -> std::io::Result<()> {
    use std::os::windows::fs::FileExt;
    while !buf.is_empty() {
        let n = file.seek_write(buf, offset)?;
        if n == 0 {
            return Err(std::io::ErrorKind::WriteZero.into());
        }
        buf = &buf[n..];
        offset += n as u64;
    }
    Ok(())
}

fn net_error(e: ureq::Error) -> anyhow::Error {
    match e {
        ureq::Error::Timeout(_) => anyhow!("the server did not answer in time"),
        ureq::Error::HostNotFound => anyhow!("the address could not be found (check the link and your connection)"),
        ureq::Error::ConnectionFailed => anyhow!("could not connect to the server"),
        ureq::Error::BadUri(_) => anyhow!("this is not a valid link"),
        e => anyhow!("network error: {e}"),
    }
}

fn status_message(status: u16) -> String {
    match status {
        401 | 403 => format!("the server refused access (HTTP {status}); the link may need a login or has expired"),
        404 | 410 => format!("the file is not there any more (HTTP {status})"),
        429 => "the server is limiting requests (HTTP 429); try again later".into(),
        s if s >= 500 => format!("the server has a problem (HTTP {s}); try again later"),
        s => format!("the server answered HTTP {s}"),
    }
}

/// The total of `bytes 0-0/12345`.
fn parse_content_range_total(v: &str) -> Option<u64> {
    v.rsplit('/').next()?.trim().parse().ok()
}

/// The file name in a `Content-Disposition` header (`filename*` preferred).
pub fn filename_from_disposition(cd: &str) -> Option<String> {
    let mut plain = None;
    for part in cd.split(';').map(str::trim) {
        let Some((key, value)) = part.split_once('=') else { continue };
        let value = value.trim();
        match key.trim().to_ascii_lowercase().as_str() {
            "filename*" => {
                // UTF-8''name%20here
                let encoded = value.split_once("''").map(|(_, v)| v).unwrap_or(value);
                let name = percent_decode(encoded.trim_matches('"'));
                if !name.is_empty() {
                    return Some(name);
                }
            }
            "filename" => plain = Some(value.trim_matches('"').to_string()),
            _ => {}
        }
    }
    plain.filter(|n| !n.is_empty())
}

/// The last part of the link's path, if it looks like a name.
fn filename_from_url(url: &str) -> Option<String> {
    let path = url.split(['?', '#']).next()?.split("://").nth(1)?;
    let (_, path) = path.split_once('/')?;
    let last = percent_decode(path.rsplit('/').next()?);
    (!last.is_empty()).then_some(last)
}

/// Adds the usual extension for `content_type` to a name without one.
fn with_extension_for(name: &str, content_type: &str) -> String {
    if Path::new(name).extension().is_some_and(|e| (1..=5).contains(&e.len())) {
        return name.to_string();
    }
    let mime = content_type.split(';').next().unwrap_or("").trim();
    let ext = match mime {
        "video/mp4" => "mp4",
        "video/webm" => "webm",
        "video/x-matroska" => "mkv",
        "video/quicktime" => "mov",
        "audio/mpeg" => "mp3",
        "audio/mp4" | "audio/x-m4a" => "m4a",
        "audio/ogg" => "ogg",
        "audio/wav" | "audio/x-wav" => "wav",
        "audio/flac" => "flac",
        "image/jpeg" => "jpg",
        "image/png" => "png",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "image/svg+xml" => "svg",
        "application/pdf" => "pdf",
        "application/zip" => "zip",
        "application/x-7z-compressed" => "7z",
        "application/vnd.rar" | "application/x-rar-compressed" => "rar",
        "application/gzip" => "gz",
        "application/json" => "json",
        "text/plain" => "txt",
        "text/html" => "html",
        "text/csv" => "csv",
        _ => return name.to_string(),
    };
    format!("{name}.{ext}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_from_headers_and_links() {
        assert_eq!(filename_from_disposition(r#"attachment; filename="report.pdf""#).as_deref(), Some("report.pdf"));
        assert_eq!(
            filename_from_disposition(r#"attachment; filename="a.pdf"; filename*=UTF-8''r%C3%A9sum%C3%A9.pdf"#)
                .as_deref(),
            Some("résumé.pdf")
        );
        assert_eq!(filename_from_disposition("inline"), None);
        assert_eq!(filename_from_url("https://x.org/dir/my%20file.zip?x=1").as_deref(), Some("my file.zip"));
        assert_eq!(filename_from_url("https://x.org/"), None);
        assert_eq!(with_extension_for("photo", "image/jpeg; charset=x"), "photo.jpg");
        assert_eq!(with_extension_for("a.zip", "application/octet-stream"), "a.zip");
        assert_eq!(parse_content_range_total("bytes 0-0/12345"), Some(12345));
    }

    #[test]
    fn segments_cover_the_file_and_split() {
        let segs = initial_segments(10 * MIN_SPLIT + 7, 4);
        assert_eq!(segs.len(), 4);
        assert_eq!(segs[0].start, 0);
        assert_eq!(segs.last().unwrap().end, 10 * MIN_SPLIT + 6);
        for w in segs.windows(2) {
            assert_eq!(w[0].end + 1, w[1].start);
        }
        // Small files use one connection.
        assert_eq!(initial_segments(1000, 8).len(), 1);

        let stop = AtomicBool::new(false);
        let limiter = Limiter::default();
        let shared = Shared {
            url: "",
            file: tempfile(),
            segs: Mutex::new(initial_segments(8 * MIN_SPLIT, 1)),
            halt: AtomicBool::new(false),
            stop: &stop,
            failure: Mutex::new(None),
            limiter: &limiter,
            validator: None,
        };
        assert_eq!(shared.claim(), Some(0));
        // The only segment is taken: the next connection splits it in half.
        assert_eq!(shared.claim(), Some(1));
        let segs = shared.segs();
        assert_eq!(segs[0].end + 1, segs[1].start);
        assert_eq!(segs[1].end, 8 * MIN_SPLIT - 1);
        assert_eq!(segs[0].remaining(), segs[1].remaining());
    }

    #[test]
    fn resume_counts_written_bytes() {
        let r = Resume { size: 300, segments: vec![[0, 50, 99], [100, 200, 199], [200, 250, 299]], validator: None };
        assert_eq!(r.done(), 50 + 100 + 50);
    }

    fn tempfile() -> File {
        let p = std::env::temp_dir().join(format!("swiftdm-seg-{}", std::process::id()));
        let f = File::create(&p).unwrap();
        let _ = std::fs::remove_file(&p);
        f
    }
}
