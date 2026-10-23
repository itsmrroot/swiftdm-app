//! Regression tests for bugs found by review: each failed before its fix.

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use swiftdm::http::{self, Limiter, Report};
use swiftdm::manager::{Manager, Source, Status};

fn content(size: usize, seed: usize) -> Vec<u8> {
    (0..size).map(|i| ((i * 7 + seed) % 251) as u8).collect()
}

#[derive(Clone, Default)]
struct Cfg {
    /// Delay before answering the probe (bytes=0-0).
    probe_delay_ms: u64,
    /// Per 64K chunk sleep.
    chunk_ms: u64,
    /// 503 everything.
    fail: Arc<AtomicBool>,
    /// 206 with empty body for non-probe ranges.
    empty_206: bool,
    /// No ranges; send 1000 bytes then stall forever.
    stall_single: bool,
    /// More ranged requests than this at once get 429 (0: no limit).
    at_once: usize,
    /// Never answer a ranged request (but accept the connection).
    silent_ranges: bool,
    /// The size really sent differs from the one announced to ranged probes:
    /// no ranges, and a body of this length (0: normal).
    dynamic_len: usize,
}

struct Server {
    url: String,
    requests: Arc<AtomicUsize>,
    /// Requests answered with 429.
    limited: Arc<AtomicUsize>,
}

fn serve(data: Vec<u8>, cfg: Cfg) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/file.bin", listener.local_addr().unwrap());
    let data = Arc::new(data);
    let requests = Arc::new(AtomicUsize::new(0));
    let limited = Arc::new(AtomicUsize::new(0));
    let in_flight = Arc::new(AtomicUsize::new(0));
    let (r, l) = (requests.clone(), limited.clone());
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let (data, cfg, r, l, f) = (data.clone(), cfg.clone(), r.clone(), l.clone(), in_flight.clone());
            std::thread::spawn(move || {
                r.fetch_add(1, Ordering::Relaxed);
                let _ = handle(stream, &data, &cfg, &l, &f);
            });
        }
    });
    Server { url, requests, limited }
}

fn handle(
    mut s: TcpStream,
    data: &[u8],
    cfg: &Cfg,
    limited: &AtomicUsize,
    in_flight: &AtomicUsize,
) -> std::io::Result<()> {
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
    if cfg.silent_ranges && range.is_some_and(|r| r != (0, 0)) {
        std::thread::sleep(Duration::from_secs(3600));
        return Ok(());
    }
    if cfg.dynamic_len > 0 {
        // A page made for each request: the first look and the real answer
        // differ in length, and it offers no ranges.
        let n = if range.is_some() { cfg.dynamic_len + 27 } else { cfg.dynamic_len };
        write!(s, "HTTP/1.1 200 OK\r\nContent-Length: {n}\r\nConnection: close\r\n\r\n")?;
        s.write_all(&data[..n.min(data.len())])?;
        return Ok(());
    }
    // Too many ranged requests at once (not the probe): HTTP 429.
    let counted = range.is_some_and(|r| r != (0, 0));
    if counted && cfg.at_once > 0 {
        let now = in_flight.fetch_add(1, Ordering::SeqCst) + 1;
        if now > cfg.at_once {
            in_flight.fetch_sub(1, Ordering::SeqCst);
            limited.fetch_add(1, Ordering::Relaxed);
            write!(
                s,
                "HTTP/1.1 429 Too Many Requests\r\nRetry-After: 0\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            )?;
            return Ok(());
        }
    }
    let result = answer(&mut s, data, cfg, range);
    if counted && cfg.at_once > 0 {
        in_flight.fetch_sub(1, Ordering::SeqCst);
    }
    result
}

fn answer(s: &mut TcpStream, data: &[u8], cfg: &Cfg, range: Option<(usize, usize)>) -> std::io::Result<()> {
    if cfg.fail.load(Ordering::Relaxed) {
        write!(s, "HTTP/1.1 503 Busy\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")?;
        return Ok(());
    }
    if cfg.stall_single {
        write!(s, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", data.len())?;
        if range.is_none() {
            s.write_all(&data[..1000])?;
            std::thread::sleep(Duration::from_secs(3600));
        }
        return Ok(());
    }
    if range == Some((0, 0)) && cfg.probe_delay_ms > 0 {
        std::thread::sleep(Duration::from_millis(cfg.probe_delay_ms));
    }
    let (status, body, extra) = match range {
        Some((a, b)) => {
            let body: &[u8] = if cfg.empty_206 && (a, b) != (0, 0) { &[] } else { &data[a..=b] };
            ("206 Partial Content", body, format!("Content-Range: bytes {a}-{b}/{}\r\n", data.len()))
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
        if cfg.chunk_ms > 0 {
            std::thread::sleep(Duration::from_millis(cfg.chunk_ms));
        }
    }
    Ok(())
}

fn temp_dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("swiftdm-bug-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn wait_for(m: &Manager, id: u64, secs: u64, f: impl Fn(&swiftdm::manager::Item) -> bool) -> bool {
    let until = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < until {
        if m.item(id).is_some_and(|i| f(&i)) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    false
}

fn ls(dir: &PathBuf) -> Vec<(String, u64)> {
    let mut v: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .flatten()
        .map(|e| (e.file_name().to_string_lossy().into_owned(), e.metadata().unwrap().len()))
        .collect();
    v.sort();
    v
}

struct Nop;
impl Report for Nop {
    fn progress(&self, _: u64, _: Option<u64>) {}
}

/// Pausing and continuing quickly must not start a second worker that a
/// later pause cannot stop.
#[test]
fn pause_after_quick_resume_still_stops() {
    let data = content(20_000_000, 1);
    let server = serve(data, Cfg { chunk_ms: 50, ..Default::default() });
    let dir = temp_dir("double");
    let m = Manager::open(None);
    let id = m.add(&server.url, Source::File, None, &dir, None);
    assert!(wait_for(&m, id, 10, |i| i.done > 500_000));
    m.pause(id);
    m.resume(id);
    std::thread::sleep(Duration::from_millis(1500));
    m.pause(id);
    std::thread::sleep(Duration::from_millis(1500));
    let before = m.item(id).unwrap().done;
    std::thread::sleep(Duration::from_secs(2));
    let it = m.item(id).unwrap();
    assert_eq!(it.status, Status::Paused);
    assert!(it.done <= before + 300_000, "still downloading after pause: {before} -> {}", it.done);
}

/// A passing server error must not throw away where the download was.
#[test]
fn a_temporary_error_keeps_the_resume_point() {
    let data = content(20_000_000, 2);
    let fail = Arc::new(AtomicBool::new(false));
    let server = serve(data, Cfg { chunk_ms: 50, fail: fail.clone(), ..Default::default() });
    let dir = temp_dir("lose");
    let m = Manager::open(None);
    let id = m.add(&server.url, Source::File, None, &dir, None);
    assert!(wait_for(&m, id, 10, |i| i.done > 2_000_000));
    m.pause(id);
    assert!(wait_for(&m, id, 10, |i| i.resume.is_some() && i.status == Status::Paused));
    fail.store(true, Ordering::Relaxed);
    m.resume(id);
    assert!(wait_for(&m, id, 10, |i| i.status == Status::Failed));
    assert!(m.item(id).unwrap().resume.is_some_and(|r| r.done() > 2_000_000));
}

/// Removing a download while it asks the server leaves nothing behind.
#[test]
fn removing_while_starting_leaves_no_file() {
    let data = content(5_000_000, 3);
    let server = serve(data, Cfg { probe_delay_ms: 800, chunk_ms: 50, ..Default::default() });
    let dir = temp_dir("leftover");
    let m = Manager::open(None);
    let id = m.add(&server.url, Source::File, None, &dir, None);
    std::thread::sleep(Duration::from_millis(300));
    m.remove(id, true);
    std::thread::sleep(Duration::from_secs(4));
    assert_eq!(ls(&dir), Vec::<(String, u64)>::new());
}

/// Two downloads of the same name get two files, both intact.
#[test]
fn same_names_get_separate_files() {
    let a = content(6_000_000, 4);
    let b = content(6_000_000, 99);
    let sa = serve(a.clone(), Cfg { probe_delay_ms: 300, chunk_ms: 20, ..Default::default() });
    let sb = serve(b.clone(), Cfg { probe_delay_ms: 300, chunk_ms: 20, ..Default::default() });
    let dir = temp_dir("collide");
    let m = Manager::open(None);
    let ia = m.add(&sa.url, Source::File, Some("same.bin".into()), &dir, None);
    let ib = m.add(&sb.url, Source::File, Some("same.bin".into()), &dir, None);
    let fin = |i: &swiftdm::manager::Item| matches!(i.status, Status::Done | Status::Failed);
    assert!(wait_for(&m, ia, 30, fin) && wait_for(&m, ib, 30, fin));
    for (id, want) in [(ia, &a), (ib, &b)] {
        let it = m.item(id).unwrap();
        assert_eq!(it.status, Status::Done, "{:?}", it.error);
        assert_eq!(&std::fs::read(it.path.unwrap()).unwrap(), want);
    }
    assert_eq!(ls(&dir).len(), 2);
}

/// Empty answers are retried with pauses, not thousands of times a second.
#[test]
fn empty_answers_are_not_hammered() {
    let server = serve(content(3_000_000, 5), Cfg { empty_206: true, ..Default::default() });
    let dir = temp_dir("empty206");
    let path = dir.join("x.bin");
    let stop = Arc::new(AtomicBool::new(false));
    let s2 = stop.clone();
    let url = server.url.clone();
    let h = std::thread::spawn(move || {
        let job = http::Job { url: &url, path: &path, connections: 2, resume: None, net: Default::default() };
        let _ = http::download(job, &s2, &Limiter::default(), &Nop);
    });
    std::thread::sleep(Duration::from_secs(5));
    let reqs = server.requests.load(Ordering::Relaxed);
    stop.store(true, Ordering::Relaxed);
    h.join().unwrap();
    assert!(reqs < 30, "{reqs} requests in 5 s");
}

/// A server that stops sending does not keep a download from stopping.
#[test]
fn a_stalled_server_does_not_block_stopping() {
    let server = serve(content(3_000_000, 6), Cfg { stall_single: true, ..Default::default() });
    let dir = temp_dir("stall");
    let path = dir.join("x.bin");
    let stop = Arc::new(AtomicBool::new(false));
    let s2 = stop.clone();
    let url = server.url.clone();
    let h = std::thread::spawn(move || {
        let job = http::Job { url: &url, path: &path, connections: 2, resume: None, net: Default::default() };
        let _ = http::download(job, &s2, &Limiter::default(), &Nop);
    });
    std::thread::sleep(Duration::from_secs(1));
    stop.store(true, Ordering::Relaxed);
    std::thread::sleep(Duration::from_secs(1));
    assert!(h.is_finished(), "still running 1 s after stop");
}

/// Low speed limits are kept.
#[test]
fn low_speed_limits_are_kept() {
    let l = Limiter::default();
    l.set_rate(20_000);
    let t = Instant::now();
    let mut total = 0u64;
    while t.elapsed() < Duration::from_secs(6) {
        let n = l.chunk();
        l.take(n);
        total += n as u64;
    }
    let rate = total as f64 / t.elapsed().as_secs_f64();
    assert!(rate < 26_000.0, "limit 20000 B/s, got {rate:.0} B/s");
}

/// A server that says "too many requests" to the 8 connections: the
/// download goes on with fewer instead of retrying all of them, and says so.
#[test]
fn too_many_requests_means_fewer_connections() {
    struct Seen(AtomicBool);
    impl Report for Seen {
        fn progress(&self, _: u64, _: Option<u64>) {}
        fn limited(&self, on: bool) {
            if on {
                self.0.store(true, Ordering::Relaxed);
            }
        }
    }
    let data = content(6_000_000, 11);
    let server = serve(data.clone(), Cfg { at_once: 2, chunk_ms: 60, ..Default::default() });
    let dir = temp_dir("limited");
    let path = dir.join("x.bin");
    let seen = Seen(AtomicBool::new(false));
    let job = http::Job { url: &server.url, path: &path, connections: 8, resume: None, net: Default::default() };
    let t = Instant::now();
    let out = http::download(job, &AtomicBool::new(false), &Limiter::default(), &seen);
    assert!(out.is_ok(), "{:?}", out.err().map(|e| (format!("{:#}", e.error), e.resume)));
    assert_eq!(std::fs::read(&path).unwrap(), data, "file differs");
    assert!(server.limited.load(Ordering::Relaxed) >= 1, "the test server never limited");
    assert!(seen.0.load(Ordering::Relaxed), "the download did not say that the server limits requests");
    assert!(t.elapsed() < Duration::from_secs(30), "took {:?}", t.elapsed());
}

/// Stopping does not wait for a server that never answers a request.
#[test]
fn stopping_does_not_wait_for_a_silent_server() {
    let server = serve(content(4_000_000, 12), Cfg { silent_ranges: true, ..Default::default() });
    let dir = temp_dir("silent");
    let path = dir.join("x.bin");
    let stop = Arc::new(AtomicBool::new(false));
    let s2 = stop.clone();
    let url = server.url.clone();
    let h = std::thread::spawn(move || {
        let job = http::Job { url: &url, path: &path, connections: 4, resume: None, net: Default::default() };
        let _ = http::download(job, &s2, &Limiter::default(), &Nop);
    });
    std::thread::sleep(Duration::from_secs(2));
    stop.store(true, Ordering::Relaxed);
    let t = Instant::now();
    while !h.is_finished() && t.elapsed() < Duration::from_secs(5) {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(h.is_finished(), "still running 5 s after stop");
}

/// An answer that is not the size of the first look (an API, a page made
/// for each request) is a complete download, not "ended before complete".
#[test]
fn the_size_of_the_answer_counts_not_the_first_look() {
    let data = content(500, 13);
    let server = serve(data.clone(), Cfg { dynamic_len: 392, ..Default::default() });
    let dir = temp_dir("dynamic");
    let path = dir.join("x.bin");
    let job = http::Job { url: &server.url, path: &path, connections: 4, resume: None, net: Default::default() };
    let out = http::download(job, &AtomicBool::new(false), &Limiter::default(), &Nop);
    assert!(out.is_ok(), "{:?}", out.err().map(|e| format!("{:#}", e.error)));
    assert_eq!(std::fs::read(&path).unwrap().len(), 392);
}
