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
use std::sync::mpsc::{Receiver, channel};
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

/// Hands `requests` to the running app (none: just show its window).
/// Returns whether one was running and took them.
pub fn send(requests: &[Incoming]) -> bool {
    hand_over(requests).is_some()
}

fn port_file() -> PathBuf {
    swiftdm::paths::data_dir().join("instance-port")
}

/// Links that later starts handed over. An empty list: just show the window.
pub struct Inbox {
    rx: Receiver<Vec<Incoming>>,
    ctx: Arc<OnceLock<egui::Context>>,
}

impl Inbox {
    /// Wakes `ctx` when links arrive.
    pub fn attach(&self, ctx: &egui::Context) {
        let _ = self.ctx.set(ctx.clone());
    }

    pub fn poll(&self) -> Option<Vec<Incoming>> {
        self.rx.try_recv().ok()
    }
}

pub enum Start {
    /// This is the only SwiftDM.
    First(Inbox),
    /// Another SwiftDM is running and has the links now.
    Handed,
}

/// Hands `links` to a running SwiftDM, or becomes the one others hand to.
pub fn start(links: &[String]) -> Start {
    let requests: Vec<Incoming> = links.iter().map(|l| Incoming::link(l)).collect();
    if hand_over(&requests).is_some() {
        return Start::Handed;
    }
    let (tx, rx) = channel();
    let ctx: Arc<OnceLock<egui::Context>> = Arc::default();
    let inbox = Inbox { rx, ctx: ctx.clone() };
    let Ok(listener) = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)) else { return Start::First(inbox) };
    let Ok(addr) = listener.local_addr() else { return Start::First(inbox) };
    let file = port_file();
    if let Some(dir) = file.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(&file, addr.port().to_string());
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            if let Some(links) = receive(stream) {
                if tx.send(links).is_err() {
                    return;
                }
                if let Some(ctx) = ctx.get() {
                    ctx.request_repaint();
                }
            }
        }
    });
    Start::First(inbox)
}

/// Sends `links` to the running SwiftDM; `None` if there is none.
fn hand_over(requests: &[Incoming]) -> Option<()> {
    hand_over_as(requests, 2).or_else(|| {
        // A running SwiftDM 1.1 or older knows only bare links.
        hand_over_as(requests, 1)
    })
}

fn hand_over_as(requests: &[Incoming], version: u8) -> Option<()> {
    let port: u16 = std::fs::read_to_string(port_file()).ok()?.trim().parse().ok()?;
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    let mut s = TcpStream::connect_timeout(&addr, Duration::from_millis(500)).ok()?;
    s.set_read_timeout(Some(Duration::from_secs(2))).ok()?;
    let mut msg = format!("SWIFTDM {version}\n");
    for r in requests {
        if version == 1 {
            msg.push_str(&r.url);
        } else {
            msg.push_str(&serde_json::to_string(r).ok()?);
        }
        msg.push('\n');
    }
    s.write_all(msg.as_bytes()).ok()?;
    s.shutdown(std::net::Shutdown::Write).ok()?;
    let mut answer = String::new();
    s.read_to_string(&mut answer).ok()?;
    // Something else may use the port now: only SwiftDM answers OK.
    (answer.trim() == "OK").then_some(())
}

/// The requests of one hand-over, answering it.
fn receive(mut stream: TcpStream) -> Option<Vec<Incoming>> {
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
    stream.write_all(b"OK\n").ok()?;
    Some(requests)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_travel_to_the_running_app() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || receive(listener.accept().unwrap().0));
        let mut s = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).unwrap();
        s.write_all(
            b"SWIFTDM 2\nhttps://example.com/a.zip\nnot a link\n{\"url\":\"https://y.org/v\",\"mode\":\"audio\"}\n",
        )
        .unwrap();
        s.shutdown(std::net::Shutdown::Write).unwrap();
        let mut answer = String::new();
        s.read_to_string(&mut answer).unwrap();
        assert_eq!(answer.trim(), "OK");
        let got = server.join().unwrap().unwrap();
        assert_eq!(got.len(), 2);
        assert_eq!(got[0], Incoming::link("https://example.com/a.zip"));
        assert_eq!((got[1].url.as_str(), got[1].mode.as_str()), ("https://y.org/v", "audio"));
    }

    #[test]
    fn strangers_are_ignored() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || receive(listener.accept().unwrap().0));
        let mut s = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).unwrap();
        s.write_all(b"GET / HTTP/1.1\r\n\r\n").unwrap();
        s.shutdown(std::net::Shutdown::Write).unwrap();
        assert_eq!(server.join().unwrap(), None);
    }
}
