//! One SwiftDM at a time. Two would share the download list and write the
//! same files; so a second start (a link opened with SwiftDM, a script, the
//! browser extension's helper) hands its links to the running one and ends.
//!
//! The running app listens on a local port. The port, its process id and a
//! random secret are in a file only this user can read (`instance-port`,
//! removed when the app ends). A sender proves it knows the secret, and so
//! does the app in its answer — another user who takes over the port after
//! SwiftDM ended gets neither links nor cookies:
//!
//! ```text
//! → SWIFTDM 3 <nonce> <sha256(secret, nonce, "ask")>
//! → one request per line (a JSON `Incoming`)
//! ← OK <sha256(secret, nonce, "ok")> <seconds since the app's loop last ran> <1 if in front>
//! ```
//!
//! The app takes the links whenever its loop runs (also while minimized);
//! the answer comes at once, so nothing is handed over twice. When its loop
//! has not run for a while although its window was in front, it is frozen:
//! a new start offers to close it.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use eframe::egui;
use sha2::{Digest, Sha256};

/// A link to add, with what came along from the browser.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Incoming {
    pub url: String,
    /// `auto`, `video`, `audio` or `file`.
    #[serde(default)]
    pub mode: String,
    /// Cookie, Referer, User-Agent.
    #[serde(default)]
    pub headers: Vec<(String, String)>,
    /// The file name the browser chose.
    #[serde(default)]
    pub name: Option<String>,
    /// Show the choices first; `None`: as in Settings.
    #[serde(default)]
    pub ask: Option<bool>,
}

impl Incoming {
    pub fn link(url: &str) -> Self {
        Incoming { url: url.to_string(), mode: "auto".into(), ..Default::default() }
    }
}

/// The app's loop has not run for this long while in front: frozen.
const FROZEN_AFTER: i64 = 15;
/// At most this much is read from one sender.
const MAX_MESSAGE: u64 = 4 * 1024 * 1024;

fn port_file() -> PathBuf {
    swiftdm::paths::data_dir().join("instance-port")
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Random hex (from the system's random hash keys, the time and the pid).
fn random_hex() -> String {
    use std::hash::{BuildHasher, Hasher};
    let mut h = Sha256::new();
    for _ in 0..4 {
        let mut s = std::collections::hash_map::RandomState::new().build_hasher();
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
        s.write_u128(nanos.as_nanos());
        s.write_u32(std::process::id());
        h.update(s.finish().to_le_bytes());
    }
    hex(&h.finalize())
}

fn proof(secret: &str, nonce: &str, what: &str) -> String {
    hex(&Sha256::digest(format!("{secret}\n{nonce}\n{what}")))
}

/// What the app tells its listener.
#[derive(Default)]
struct Life {
    /// When the app's loop last ran (seconds since 1970).
    beat: AtomicI64,
    /// Whether its window was in front then.
    in_front: AtomicBool,
}

/// Links that later starts handed over. An empty list: just show the window.
pub struct Inbox {
    rx: Receiver<Vec<Incoming>>,
    ctx: Arc<OnceLock<egui::Context>>,
    life: Arc<Life>,
    file: PathBuf,
}

impl Inbox {
    /// Wakes `ctx` when links arrive.
    pub fn attach(&self, ctx: &egui::Context) {
        let _ = self.ctx.set(ctx.clone());
    }

    /// The next hand-over. Called from the app's loop, which so tells the
    /// listener that the app works (`in_front`: its window is in front).
    pub fn poll(&self, in_front: bool) -> Option<Vec<Incoming>> {
        self.life.beat.store(swiftdm::util::now(), Ordering::Relaxed);
        self.life.in_front.store(in_front, Ordering::Relaxed);
        self.rx.try_recv().ok()
    }
}

impl Drop for Inbox {
    /// The app ends: nothing may answer on its port any more.
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.file);
    }
}

pub enum Start {
    /// This is the only SwiftDM.
    First(Inbox),
    /// Another SwiftDM is running and has the links now.
    Handed,
}

/// What became of a hand-over.
#[derive(Debug, PartialEq)]
enum Handed {
    Yes,
    /// No SwiftDM is running (or something else uses the port now).
    NoOne,
    /// A SwiftDM took the links but its loop does not run: frozen.
    Frozen(u32),
}

/// Hands `requests` to the running app (none: just show its window).
/// Returns whether one was running and took them.
pub fn send(requests: &[Incoming]) -> bool {
    hand_over_at(&port_file(), requests) != Handed::NoOne
}

/// Hands `links` to a running SwiftDM, or becomes the one others hand to.
pub fn start(links: &[String]) -> Start {
    let requests: Vec<Incoming> = links.iter().map(|l| Incoming::link(l)).collect();
    match hand_over_at(&port_file(), &requests) {
        Handed::Yes => return Start::Handed,
        Handed::NoOne => {}
        Handed::Frozen(pid) => {
            // A frozen SwiftDM would otherwise take every start silently.
            crate::i18n::set_language(crate::i18n::Language::System);
            let close = rfd::MessageDialog::new()
                .set_level(rfd::MessageLevel::Warning)
                .set_title("SwiftDM")
                .set_description(crate::i18n::trl(
                    "SwiftDM is already running but does not respond. Close it and start again?",
                ))
                .set_buttons(rfd::MessageButtons::YesNo)
                .show();
            if close != rfd::MessageDialogResult::Yes || !kill_swiftdm(pid) {
                return Start::Handed;
            }
        }
    }
    serve(&port_file())
}

/// Listens for hand-overs, announced in `file`.
fn serve(file: &Path) -> Start {
    let (tx, rx) = channel();
    let life = Arc::new(Life { beat: AtomicI64::new(swiftdm::util::now()), in_front: AtomicBool::new(false) });
    let ctx: Arc<OnceLock<egui::Context>> = Arc::default();
    let inbox = Inbox { rx, ctx: ctx.clone(), life: life.clone(), file: file.to_path_buf() };
    let Ok(listener) = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)) else { return Start::First(inbox) };
    let Ok(addr) = listener.local_addr() else { return Start::First(inbox) };
    let secret = random_hex();
    if let Some(dir) = file.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = write_private(file, &format!("{} {} {secret}", addr.port(), std::process::id()));
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let (tx, ctx, life, secret) = (tx.clone(), ctx.clone(), life.clone(), secret.clone());
            // One at a time would let a slow sender hold up all others.
            std::thread::spawn(move || answer(stream, &secret, &tx, &ctx, &life));
        }
    });
    Start::First(inbox)
}

/// Writes `text` to `file`, readable by this user only.
fn write_private(file: &Path, text: &str) -> std::io::Result<()> {
    let _ = std::fs::remove_file(file);
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(0o600);
    }
    o.open(file)?.write_all(text.as_bytes())
}

/// One hand-over: checks the sender, queues its links and answers.
fn answer(
    mut stream: TcpStream,
    secret: &str,
    tx: &Sender<Vec<Incoming>>,
    ctx: &OnceLock<egui::Context>,
    life: &Life,
) -> Option<()> {
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok()?;
    let mut text = String::new();
    (&stream).take(MAX_MESSAGE).read_to_string(&mut text).ok()?;
    let (nonce, requests) = parse(&text, secret)?;
    tx.send(requests).ok()?;
    if let Some(ctx) = ctx.get() {
        ctx.request_repaint();
    }
    let idle = swiftdm::util::now() - life.beat.load(Ordering::Relaxed);
    let front = u8::from(life.in_front.load(Ordering::Relaxed));
    stream.write_all(format!("OK {} {idle} {front}\n", proof(secret, &nonce, "ok")).as_bytes()).ok()
}

/// The nonce and requests of a hand-over from a sender that knows `secret`.
fn parse(text: &str, secret: &str) -> Option<(String, Vec<Incoming>)> {
    let mut lines = text.lines();
    let mut hello = lines.next()?.split_whitespace();
    let (Some("SWIFTDM"), Some("3"), Some(nonce), Some(p)) = (hello.next(), hello.next(), hello.next(), hello.next())
    else {
        return None;
    };
    if p != proof(secret, nonce, "ask") {
        return None;
    }
    let requests = lines
        .filter_map(|l| serde_json::from_str::<Incoming>(l.trim()).ok())
        .filter(|r| swiftdm::util::is_link(&r.url))
        .take(1000)
        .collect();
    Some((nonce.to_string(), requests))
}

/// Ends a frozen SwiftDM — only if `pid` really is one. False: it could
/// not be ended.
fn kill_swiftdm(pid: u32) -> bool {
    if pid == std::process::id() {
        return false;
    }
    #[cfg(unix)]
    {
        let name = std::process::Command::new("ps")
            .args(["-p", &pid.to_string(), "-o", "comm="])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).to_lowercase())
            .unwrap_or_default();
        if !name.contains("swiftdm") {
            return true; // gone already
        }
        // SAFETY: a plain signal to the process checked above.
        if unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL) } != 0 {
            return false;
        }
    }
    #[cfg(windows)]
    {
        let list = swiftdm::tools::command("tasklist")
            .args(["/FI", &format!("PID eq {pid}"), "/FO", "CSV", "/NH"])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).to_lowercase())
            .unwrap_or_default();
        if !list.contains("swiftdm") {
            return true;
        }
        let ended = swiftdm::tools::command("taskkill")
            .args(["/F", "/PID", &pid.to_string()])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|s| s.success());
        if !ended {
            return false;
        }
    }
    std::thread::sleep(Duration::from_millis(500));
    true
}

fn hand_over_at(file: &Path, requests: &[Incoming]) -> Handed {
    let Ok(text) = std::fs::read_to_string(file) else { return Handed::NoOne };
    let mut parts = text.split_whitespace();
    let (Some(port), Some(pid), Some(secret)) = (
        parts.next().and_then(|p| p.parse::<u16>().ok()),
        parts.next().and_then(|p| p.parse::<u32>().ok()),
        parts.next(),
    ) else {
        return Handed::NoOne;
    };
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    let Ok(mut s) = TcpStream::connect_timeout(&addr, Duration::from_millis(500)) else { return Handed::NoOne };
    let _ = s.set_read_timeout(Some(Duration::from_secs(5)));
    let nonce = random_hex();
    let mut msg = format!("SWIFTDM 3 {nonce} {}\n", proof(secret, &nonce, "ask"));
    for r in requests {
        let Ok(line) = serde_json::to_string(r) else { return Handed::NoOne };
        msg.push_str(&line);
        msg.push('\n');
    }
    if s.write_all(msg.as_bytes()).and_then(|_| s.shutdown(std::net::Shutdown::Write)).is_err() {
        return Handed::NoOne;
    }
    let mut reply = String::new();
    if BufReader::new(s).take(4096).read_line(&mut reply).is_err() {
        return Handed::NoOne;
    }
    let mut r = reply.split_whitespace();
    let (Some("OK"), Some(p)) = (r.next(), r.next()) else { return Handed::NoOne };
    // Only the SwiftDM that wrote the file can answer this.
    if p != proof(secret, &nonce, "ok") {
        return Handed::NoOne;
    }
    let idle: i64 = r.next().and_then(|v| v.parse().ok()).unwrap_or(0);
    let in_front = r.next() == Some("1");
    if idle > FROZEN_AFTER && in_front { Handed::Frozen(pid) } else { Handed::Yes }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("swiftdm-port-{name}-{}", std::process::id()))
    }

    #[test]
    fn links_reach_the_running_app_once() {
        let file = temp("ok");
        let Start::First(inbox) = serve(&file) else { panic!() };
        let link = Incoming::link("https://example.com/a.zip");
        assert_eq!(hand_over_at(&file, std::slice::from_ref(&link)), Handed::Yes);
        assert_eq!(inbox.poll(true), Some(vec![link]));
        assert_eq!(inbox.poll(true), None);
        drop(inbox);
        assert!(!file.exists(), "the port file goes with the app");
    }

    #[test]
    fn a_frozen_app_in_front_is_noticed_but_not_a_minimized_one() {
        let file = temp("frozen");
        let Start::First(inbox) = serve(&file) else { panic!() };
        inbox.life.beat.store(swiftdm::util::now() - 60, Ordering::Relaxed);
        inbox.life.in_front.store(true, Ordering::Relaxed);
        assert!(matches!(hand_over_at(&file, &[]), Handed::Frozen(_)));
        // Minimized (some systems pause the loop then): fine.
        inbox.life.in_front.store(false, Ordering::Relaxed);
        assert_eq!(hand_over_at(&file, &[]), Handed::Yes);
    }

    #[test]
    fn nobody_running() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let file = temp("none");
        std::fs::write(&file, format!("{port} 1 abc")).unwrap();
        assert_eq!(hand_over_at(&file, &[]), Handed::NoOne);
        let _ = std::fs::remove_file(&file);
        assert_eq!(hand_over_at(Path::new("/nonexistent/port"), &[]), Handed::NoOne);
    }

    #[test]
    fn an_impostor_on_the_port_gets_nothing() {
        // Someone else listens on the port the file names: it cannot prove
        // it knows the secret, so nothing counts as handed over.
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let file = temp("impostor");
        std::fs::write(&file, format!("{port} 1 {}", random_hex())).unwrap();
        std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let mut buf = String::new();
            let _ = (&s).take(1 << 20).read_to_string(&mut buf);
            let _ = s.write_all(b"OK x 0 0\n");
        });
        assert_eq!(hand_over_at(&file, &[Incoming::link("https://example.com/a.zip")]), Handed::NoOne);
        let _ = std::fs::remove_file(&file);
    }

    #[test]
    fn senders_without_the_secret_are_ignored() {
        let secret = random_hex();
        assert!(parse("GET / HTTP/1.1\r\n\r\n", &secret).is_none());
        assert!(parse("SWIFTDM 3 n wrong\n{\"url\":\"https://x.org/a\"}\n", &secret).is_none());
        let good = format!(
            "SWIFTDM 3 n {}\n{{\"url\":\"https://x.org/a\",\"mode\":\"audio\"}}\nnot json\n",
            proof(&secret, "n", "ask")
        );
        let (nonce, r) = parse(&good, &secret).unwrap();
        assert_eq!((nonce.as_str(), r.len(), r[0].mode.as_str()), ("n", 1, "audio"));
    }
}
