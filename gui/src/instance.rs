//! One SwiftDM at a time. Two would share the download list and write the
//! same files; so a second start (a link opened with SwiftDM, a script, a
//! browser add-on) hands its links to the running one and ends.
//!
//! The running app listens on a local port written to the app's folder; a
//! later start (or the browser extension's helper) connects, sends
//! `SWIFTDM 2` and one request per line — a link, or a JSON [`Incoming`] —
//! and waits for `OK`.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, SyncSender, channel, sync_channel};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use eframe::egui;

const HELLO: &str = "SWIFTDM 2";

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

fn port_file() -> PathBuf {
    swiftdm::paths::data_dir().join("instance-port")
}

/// A hand-over waiting for the app to take it.
struct Delivery {
    requests: Vec<Incoming>,
    /// Told when the app has the requests: only then the sender hears OK.
    taken: SyncSender<()>,
}

/// How long the app has to take a hand-over. It takes them in
/// `App::logic`, which runs even while the window is minimized; an app that
/// does not answer in this time is stuck.
const TAKE_WITHIN: Duration = Duration::from_secs(4);

/// Links that later starts handed over. An empty list: just show the window.
pub struct Inbox {
    rx: Receiver<Delivery>,
    ctx: Arc<OnceLock<egui::Context>>,
}

impl Inbox {
    /// Wakes `ctx` when links arrive.
    pub fn attach(&self, ctx: &egui::Context) {
        let _ = self.ctx.set(ctx.clone());
    }

    /// The next hand-over, confirming it to its sender.
    pub fn poll(&self) -> Option<Vec<Incoming>> {
        let d = self.rx.try_recv().ok()?;
        let _ = d.taken.send(());
        Some(d.requests)
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
    /// A SwiftDM is running but does not answer (frozen); its process id.
    Stuck(Option<u32>),
}

/// Hands `requests` to the running app (none: just show its window).
/// Returns whether one was running and took them.
pub fn send(requests: &[Incoming]) -> bool {
    hand_over(requests) == Handed::Yes
}

/// Hands `links` to a running SwiftDM, or becomes the one others hand to.
pub fn start(links: &[String]) -> Start {
    let requests: Vec<Incoming> = links.iter().map(|l| Incoming::link(l)).collect();
    match hand_over(&requests) {
        Handed::Yes => return Start::Handed,
        Handed::NoOne => {}
        Handed::Stuck(pid) => {
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
            if close != rfd::MessageDialogResult::Yes {
                return Start::Handed;
            }
            if let Some(pid) = pid {
                kill(pid);
            }
        }
    }
    let (tx, rx) = channel::<Delivery>();
    let ctx: Arc<OnceLock<egui::Context>> = Arc::default();
    let inbox = Inbox { rx, ctx: ctx.clone() };
    let Ok(listener) = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)) else { return Start::First(inbox) };
    let Ok(addr) = listener.local_addr() else { return Start::First(inbox) };
    let file = port_file();
    if let Some(dir) = file.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(&file, format!("{} {}", addr.port(), std::process::id()));
    std::thread::spawn(move || listen(listener, tx, ctx));
    Start::First(inbox)
}

/// Answers hand-overs: OK once the app has taken them.
fn listen(listener: TcpListener, tx: std::sync::mpsc::Sender<Delivery>, ctx: Arc<OnceLock<egui::Context>>) {
    for mut stream in listener.incoming().flatten() {
        let Some(requests) = receive(&stream) else { continue };
        let (taken, confirmed) = sync_channel(1);
        if tx.send(Delivery { requests, taken }).is_err() {
            return;
        }
        if let Some(ctx) = ctx.get() {
            ctx.request_repaint();
        }
        // The app's loop does not run: say so, or the sender would take
        // this for an older SwiftDM and start a second one.
        let answer: &[u8] = if confirmed.recv_timeout(TAKE_WITHIN).is_ok() { b"OK\n" } else { b"STUCK\n" };
        let _ = stream.write_all(answer);
    }
}

/// Ends a frozen SwiftDM.
fn kill(pid: u32) {
    if pid == std::process::id() {
        return;
    }
    #[cfg(unix)]
    // SAFETY: plain signal to a process id.
    unsafe {
        libc::kill(pid as libc::pid_t, libc::SIGKILL);
    }
    #[cfg(windows)]
    {
        let _ = swiftdm::tools::command("taskkill")
            .args(["/F", "/PID", &pid.to_string()])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }
    std::thread::sleep(Duration::from_millis(500));
}

fn hand_over(requests: &[Incoming]) -> Handed {
    hand_over_at(&port_file(), requests)
}

fn hand_over_at(file: &std::path::Path, requests: &[Incoming]) -> Handed {
    match hand_over_as(file, requests, 2) {
        // A running SwiftDM 1.1 or older closes on version 2: it knows only
        // bare links.
        Answer::Closed => match hand_over_as(file, requests, 1) {
            Answer::Ok => Handed::Yes,
            _ => Handed::NoOne,
        },
        Answer::Ok => Handed::Yes,
        Answer::Silent(pid) => Handed::Stuck(pid),
        Answer::Nobody => Handed::NoOne,
    }
}

enum Answer {
    Ok,
    /// Connected, but closed without OK.
    Closed,
    /// Connected, but no answer in time.
    Silent(Option<u32>),
    Nobody,
}

fn hand_over_as(file: &std::path::Path, requests: &[Incoming], version: u8) -> Answer {
    let Some(text) = std::fs::read_to_string(file).ok() else { return Answer::Nobody };
    let mut parts = text.split_whitespace();
    let Some(port) = parts.next().and_then(|p| p.parse::<u16>().ok()) else { return Answer::Nobody };
    let pid = parts.next().and_then(|p| p.parse::<u32>().ok());
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    let Ok(mut s) = TcpStream::connect_timeout(&addr, Duration::from_millis(500)) else { return Answer::Nobody };
    let _ = s.set_read_timeout(Some(TAKE_WITHIN + Duration::from_secs(1)));
    let mut msg = format!("SWIFTDM {version}\n");
    for r in requests {
        if version == 1 {
            msg.push_str(&r.url);
        } else {
            let Ok(line) = serde_json::to_string(r) else { return Answer::Nobody };
            msg.push_str(&line);
        }
        msg.push('\n');
    }
    if s.write_all(msg.as_bytes()).and_then(|_| s.shutdown(std::net::Shutdown::Write)).is_err() {
        return Answer::Closed;
    }
    let mut answer = String::new();
    match s.read_to_string(&mut answer) {
        // Something else may use the port now: only SwiftDM answers OK.
        Ok(_) if answer.trim() == "OK" => Answer::Ok,
        Ok(_) if answer.trim() == "STUCK" => Answer::Silent(pid),
        Ok(_) => Answer::Closed,
        Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {
            Answer::Silent(pid)
        }
        Err(_) => Answer::Closed,
    }
}

/// The requests of one hand-over (not yet answered).
fn receive(stream: &TcpStream) -> Option<Vec<Incoming>> {
    stream.set_read_timeout(Some(Duration::from_secs(2))).ok()?;
    let mut lines = BufReader::new(stream.try_clone().ok()?).lines().map_while(Result::ok);
    // Version 1 (bare links) is still understood.
    if !matches!(lines.next()?.trim(), "SWIFTDM 1" | HELLO) {
        return None;
    }
    let requests: Vec<Incoming> = lines
        .filter_map(|l| {
            let l = l.trim();
            let r = if l.starts_with('{') { serde_json::from_str(l).ok()? } else { Incoming::link(l) };
            swiftdm::util::is_link(&r.url).then_some(r)
        })
        .take(1000)
        .collect();
    Some(requests)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    /// A running app with its port file. `answers`: whether its loop
    /// takes hand-overs (a frozen app does not).
    fn app(name: &str, answers: bool) -> PathBuf {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let file = std::env::temp_dir().join(format!("swiftdm-port-{name}-{}", std::process::id()));
        std::fs::write(&file, format!("{} 4242", listener.local_addr().unwrap().port())).unwrap();
        let (tx, rx) = channel();
        let inbox = Inbox { rx, ctx: Arc::default() };
        std::thread::spawn({
            let ctx = inbox.ctx.clone();
            move || listen(listener, tx, ctx)
        });
        std::thread::spawn(move || {
            // The app's loop, polling now and then — or frozen.
            for _ in 0..400 {
                if answers {
                    let _ = inbox.poll();
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        });
        file
    }

    #[test]
    fn a_working_app_takes_the_links() {
        let file = app("ok", true);
        let r = hand_over_at(&file, &[Incoming::link("https://example.com/a.zip")]);
        assert_eq!(r, Handed::Yes);
    }

    #[test]
    fn a_frozen_app_is_noticed() {
        let file = app("stuck", false);
        let r = hand_over_at(&file, &[Incoming::link("https://example.com/a.zip")]);
        assert_eq!(r, Handed::Stuck(Some(4242)));
    }

    #[test]
    fn nobody_running() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let file = std::env::temp_dir().join(format!("swiftdm-port-none-{}", std::process::id()));
        std::fs::write(&file, format!("{port} 1")).unwrap();
        assert_eq!(hand_over_at(&file, &[]), Handed::NoOne);
        assert_eq!(hand_over_at(Path::new("/nonexistent/port"), &[]), Handed::NoOne);
    }

    #[test]
    fn requests_are_read() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || receive(&listener.accept().unwrap().0));
        let mut s = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).unwrap();
        s.write_all(
            b"SWIFTDM 2\nhttps://example.com/a.zip\nnot a link\n{\"url\":\"https://y.org/v\",\"mode\":\"audio\"}\n",
        )
        .unwrap();
        s.shutdown(std::net::Shutdown::Write).unwrap();
        let got = server.join().unwrap().unwrap();
        assert_eq!(got.len(), 2);
        assert_eq!(got[0], Incoming::link("https://example.com/a.zip"));
        assert_eq!((got[1].url.as_str(), got[1].mode.as_str()), ("https://y.org/v", "audio"));
    }

    #[test]
    fn strangers_are_ignored() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || receive(&listener.accept().unwrap().0));
        let mut s = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).unwrap();
        s.write_all(b"GET / HTTP/1.1\r\n\r\n").unwrap();
        s.shutdown(std::net::Shutdown::Write).unwrap();
        assert_eq!(server.join().unwrap(), None);
    }
}
