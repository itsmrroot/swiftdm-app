//! Downloads from a small local server: several connections, pausing and
//! continuing, and servers without byte ranges.

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use swiftdm::http::{self, Limiter, Outcome, Report, Resume};

fn content(size: usize) -> Vec<u8> {
    (0..size).map(|i| (i * 7 % 251) as u8).collect()
}

struct Server {
    url: String,
    ranged_requests: Arc<AtomicUsize>,
}

/// Serves `data` at `/file.bin`; `ranges` decides whether byte ranges work.
/// `slow` pauses after every chunk so a download can be stopped midway.
fn serve(data: Vec<u8>, ranges: bool, slow: bool) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/file.bin", listener.local_addr().unwrap());
    let data = Arc::new(data);
    let ranged_requests = Arc::new(AtomicUsize::new(0));
    let counter = ranged_requests.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let (data, counter) = (data.clone(), counter.clone());
            std::thread::spawn(move || {
                let _ = handle(stream, &data, ranges, slow, &counter);
            });
        }
    });
    Server { url, ranged_requests }
}

fn handle(mut s: TcpStream, data: &[u8], ranges: bool, slow: bool, counter: &AtomicUsize) -> std::io::Result<()> {
    let mut reader = BufReader::new(s.try_clone()?);
    let mut range = None;
    let mut line = String::new();
    reader.read_line(&mut line)?;
    loop {
        line.clear();
        reader.read_line(&mut line)?;
        if line.trim().is_empty() {
            break;
        }
        if let Some(v) = line.to_ascii_lowercase().strip_prefix("range: bytes=") {
            let (a, b) = v.trim().split_once('-').unwrap();
            let end = if b.is_empty() { data.len() - 1 } else { b.parse::<usize>().unwrap().min(data.len() - 1) };
            range = Some((a.parse::<usize>().unwrap(), end));
        }
    }
    let (status, body, extra) = match range.filter(|_| ranges) {
        Some((a, b)) => {
            counter.fetch_add(1, Ordering::Relaxed);
            ("206 Partial Content", &data[a..=b], format!("Content-Range: bytes {a}-{b}/{}\r\n", data.len()))
        }
        None => ("200 OK", data, String::new()),
    };
    write!(
        s,
        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nContent-Type: application/octet-stream\r\nContent-Disposition: attachment; filename=\"served.bin\"\r\n{extra}Connection: close\r\n\r\n",
        body.len()
    )?;
    for chunk in body.chunks(64 * 1024) {
        s.write_all(chunk)?;
        if slow {
            std::thread::sleep(Duration::from_millis(50));
        }
    }
    Ok(())
}

#[derive(Default)]
struct Log {
    last: Mutex<(u64, Option<u64>)>,
}

impl Report for Log {
    fn progress(&self, done: u64, total: Option<u64>) {
        *self.last.lock().unwrap() = (done, total);
    }
}

fn temp_dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("swiftdm-test-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn probe_reads_name_size_and_ranges() {
    let server = serve(content(3_000_000), true, false);
    let p = http::probe(&server.url).unwrap();
    assert_eq!(p.name, "served.bin");
    assert_eq!(p.size, Some(3_000_000));
    assert!(p.resumable);
    let server = serve(content(1000), false, false);
    let p = http::probe(&server.url).unwrap();
    assert_eq!(p.size, Some(1000));
    assert!(!p.resumable);
}

#[test]
fn several_connections_give_the_same_file() {
    let data = content(9_000_123);
    let server = serve(data.clone(), true, false);
    let dir = temp_dir("multi");
    let path = dir.join("out.bin");
    let log = Log::default();
    let job = http::Job { url: &server.url, path: &path, connections: 6, resume: None, net: Default::default() };
    let outcome = http::download(job, &AtomicBool::new(false), &Limiter::default(), &log).unwrap();
    let Outcome::Finished(saved) = outcome else { panic!("not finished") };
    assert_eq!(saved, path);
    assert_eq!(std::fs::read(&path).unwrap(), data);
    assert!(server.ranged_requests.load(Ordering::Relaxed) >= 6);
    assert_eq!(log.last.lock().unwrap().0, data.len() as u64);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_stopped_download_continues_where_it_was() {
    let data = content(6_000_000);
    let server = serve(data.clone(), true, true);
    let dir = temp_dir("resume");
    let path = dir.join("out.bin");
    let stop = Arc::new(AtomicBool::new(false));
    let s = stop.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(300));
        s.store(true, Ordering::Relaxed);
    });
    let job = http::Job { url: &server.url, path: &path, connections: 4, resume: None, net: Default::default() };
    let outcome = http::download(job, &stop, &Limiter::default(), &Log::default()).unwrap();
    let Outcome::Stopped(Some(resume)) = outcome else { panic!("expected a stop with resume state") };
    let done = resume.done();
    assert!(done > 0 && done < data.len() as u64, "{done}");
    assert!(!path.exists());

    // The saved state survives a round trip through JSON.
    let resume: Resume = serde_json::from_str(&serde_json::to_string(&resume).unwrap()).unwrap();
    let before = server.ranged_requests.load(Ordering::Relaxed);
    let job =
        http::Job { url: &server.url, path: &path, connections: 4, resume: Some(resume), net: Default::default() };
    let outcome = http::download(job, &AtomicBool::new(false), &Limiter::default(), &Log::default()).unwrap();
    assert!(matches!(outcome, Outcome::Finished(_)));
    assert_eq!(std::fs::read(&path).unwrap(), data);
    assert!(server.ranged_requests.load(Ordering::Relaxed) > before);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn servers_without_ranges_use_one_connection() {
    let data = content(2_500_000);
    let server = serve(data.clone(), false, false);
    let dir = temp_dir("single");
    let path = dir.join("out.bin");
    let job = http::Job { url: &server.url, path: &path, connections: 8, resume: None, net: Default::default() };
    let outcome = http::download(job, &AtomicBool::new(false), &Limiter::default(), &Log::default()).unwrap();
    assert!(matches!(outcome, Outcome::Finished(_)));
    assert_eq!(std::fs::read(&path).unwrap(), data);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn the_list_downloads_and_names_files() {
    use swiftdm::manager::{Manager, Source, Status};
    let data = content(4_000_000);
    let server = serve(data.clone(), true, false);
    let dir = temp_dir("manager");
    std::fs::write(dir.join("served.bin"), b"existing").unwrap();
    let m = Manager::open(None);
    let id = m.add(&server.url, Source::File, None, &dir, None);
    for _ in 0..200 {
        if m.item(id).is_some_and(|i| matches!(i.status, Status::Done | Status::Failed)) {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let item = m.item(id).unwrap();
    assert_eq!(item.status, Status::Done, "{:?}", item.error);
    // The existing file is kept; the new one gets a number.
    assert_eq!(item.name, "served (2).bin");
    assert_eq!(std::fs::read(dir.join("served (2).bin")).unwrap(), data);
    assert_eq!(std::fs::read(dir.join("served.bin")).unwrap(), b"existing");
    std::fs::remove_dir_all(&dir).unwrap();
}
