//! One SwiftDM at a time. Two would share the download list and write the
//! same files; so a second start (a link opened with SwiftDM, a script, a
//! browser add-on) hands its links to the running one and ends.
//!
//! The running app listens on a local port written to the app's folder; a
//! later start connects, sends `SWIFTDM 1` and its links, one per line, and
//! waits for `OK`.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, channel};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use eframe::egui;

const HELLO: &str = "SWIFTDM 1";

fn port_file() -> PathBuf {
    swiftdm::paths::data_dir().join("instance-port")
}

/// Links that later starts handed over. An empty list: just show the window.
pub struct Inbox {
    rx: Receiver<Vec<String>>,
    ctx: Arc<OnceLock<egui::Context>>,
}

impl Inbox {
    /// Wakes `ctx` when links arrive.
    pub fn attach(&self, ctx: &egui::Context) {
        let _ = self.ctx.set(ctx.clone());
    }

    pub fn poll(&self) -> Option<Vec<String>> {
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
    if hand_over(links).is_some() {
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
fn hand_over(links: &[String]) -> Option<()> {
    let port: u16 = std::fs::read_to_string(port_file()).ok()?.trim().parse().ok()?;
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    let mut s = TcpStream::connect_timeout(&addr, Duration::from_millis(500)).ok()?;
    s.set_read_timeout(Some(Duration::from_secs(2))).ok()?;
    let mut msg = format!("{HELLO}\n");
    for l in links {
        msg.push_str(l.trim());
        msg.push('\n');
    }
    s.write_all(msg.as_bytes()).ok()?;
    s.shutdown(std::net::Shutdown::Write).ok()?;
    let mut answer = String::new();
    s.read_to_string(&mut answer).ok()?;
    // Something else may use the port now: only SwiftDM answers OK.
    (answer.trim() == "OK").then_some(())
}

/// The links of one hand-over, answering it.
fn receive(mut stream: TcpStream) -> Option<Vec<String>> {
    stream.set_read_timeout(Some(Duration::from_secs(2))).ok()?;
    let mut lines = BufReader::new(stream.try_clone().ok()?).lines().map_while(Result::ok);
    if lines.next()?.trim() != HELLO {
        return None;
    }
    let links: Vec<String> = lines.filter(|l| swiftdm::util::is_link(l)).take(1000).collect();
    stream.write_all(b"OK\n").ok()?;
    Some(links)
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
        s.write_all(b"SWIFTDM 1\nhttps://example.com/a.zip\nnot a link\n").unwrap();
        s.shutdown(std::net::Shutdown::Write).unwrap();
        let mut answer = String::new();
        s.read_to_string(&mut answer).unwrap();
        assert_eq!(answer.trim(), "OK");
        assert_eq!(server.join().unwrap(), Some(vec!["https://example.com/a.zip".to_string()]));
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
