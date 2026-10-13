//! The download list: runs downloads in the background, a few at a time,
//! and remembers them (with where to continue) across restarts.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::http::{self, Limiter, Outcome, Resume};
use crate::media::{self, ErrorKind, Mode};
use crate::util::{FileKind, now, sanitize_filename, unique_path};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Status {
    Queued,
    Active,
    Paused,
    Done,
    Failed,
}

/// How an item is downloaded.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Source {
    /// A plain file, over HTTP.
    File,
    /// A video or song from a site, through yt-dlp.
    Media(media::Options),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Item {
    pub id: u64,
    pub url: String,
    pub source: Source,
    /// File name, or the title until the file exists; empty while unknown.
    pub name: String,
    pub dir: PathBuf,
    /// The file: chosen when a plain download starts, known when a media
    /// download ends.
    pub path: Option<PathBuf>,
    pub status: Status,
    pub error: Option<String>,
    pub total: Option<u64>,
    pub done: u64,
    /// Seconds since 1970.
    pub added: i64,
    pub finished: Option<i64>,
    pub thumbnail: Option<String>,
    /// Where a plain download continues.
    #[serde(default)]
    pub resume: Option<Resume>,
    /// Bytes per second, while running.
    #[serde(skip)]
    pub speed: f64,
    #[serde(skip)]
    pub eta: Option<u64>,
    /// What is happening besides downloading ("Converting to MP3").
    #[serde(skip)]
    pub stage: Option<String>,
}

impl Item {
    pub fn kind(&self) -> FileKind {
        let by_name = FileKind::of(&self.name);
        match &self.source {
            Source::Media(o) if o.mode == Mode::Audio => FileKind::Audio,
            Source::Media(_) if by_name == FileKind::Other => FileKind::Video,
            _ => by_name,
        }
    }

    pub fn fraction(&self) -> Option<f32> {
        match self.status {
            Status::Done => Some(1.0),
            _ => self.total.filter(|t| *t > 0).map(|t| (self.done as f64 / t as f64).clamp(0.0, 1.0) as f32),
        }
    }

    /// What to show as its name.
    pub fn display_name(&self) -> &str {
        if self.name.is_empty() { &self.url } else { &self.name }
    }

    pub fn is_media(&self) -> bool {
        matches!(self.source, Source::Media(_))
    }
}

/// How the list runs. Changed from the app's settings.
#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    /// Downloads running at the same time.
    pub max_active: usize,
    /// Connections per file.
    pub connections: usize,
    /// Bytes per second for everything together; 0 is unlimited.
    pub speed_limit: u64,
    /// Browser whose login is used for sites that need one.
    pub cookies_from: Option<String>,
    /// When a site refuses, try the logins of the usual browsers.
    pub auto_cookies: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self { max_active: 3, connections: 8, speed_limit: 0, cookies_from: None, auto_cookies: true }
    }
}

/// Browsers tried for a login, most likely first.
pub fn browsers() -> &'static [&'static str] {
    if cfg!(target_os = "macos") {
        &["chrome", "safari", "firefox", "brave", "edge"]
    } else if cfg!(windows) {
        &["firefox", "chrome", "edge", "brave"]
    } else {
        &["firefox", "chrome", "chromium", "brave"]
    }
}

#[derive(Serialize, Deserialize)]
struct Saved {
    version: u32,
    items: Vec<Item>,
}

struct State {
    items: Vec<Item>,
    next_id: u64,
    settings: Settings,
    /// Stop flags of the running downloads.
    running: HashMap<u64, Arc<AtomicBool>>,
    /// Changed since the list was last saved.
    dirty: bool,
    /// Closing: downloads stopped now continue at the next start.
    closing: bool,
}

type Notify = Box<dyn Fn() + Send + Sync>;

struct Inner {
    state: Mutex<State>,
    limiter: Limiter,
    notify: Mutex<Option<Notify>>,
    store: Option<PathBuf>,
}

impl Inner {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn notify(&self) {
        if let Some(f) = self.notify.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
            f();
        }
    }

    fn update(&self, id: u64, f: impl FnOnce(&mut Item)) {
        {
            let mut s = self.lock();
            if let Some(item) = s.items.iter_mut().find(|i| i.id == id) {
                f(item);
            }
        }
        self.notify();
    }

    fn save(&self) {
        let Some(path) = &self.store else { return };
        let items = {
            let mut s = self.lock();
            if !s.dirty {
                return;
            }
            s.dirty = false;
            s.items.clone()
        };
        let json = match serde_json::to_vec_pretty(&Saved { version: 1, items }) {
            Ok(j) => j,
            Err(e) => return log::warn!("cannot save the download list: {e}"),
        };
        let tmp = path.with_extension("json.tmp");
        let result = path
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|_| std::fs::write(&tmp, json))
            .and_then(|_| std::fs::rename(&tmp, path));
        if let Err(e) = result {
            log::warn!("cannot save the download list: {e}");
        }
    }
}

/// The download list. Cheap to clone; all clones are the same list.
#[derive(Clone)]
pub struct Manager {
    inner: Arc<Inner>,
}

impl Manager {
    /// The list saved at `store` (or a new one); `None` keeps nothing.
    /// Downloads that were running continue.
    pub fn open(store: Option<PathBuf>) -> Manager {
        let mut items: Vec<Item> = store
            .as_deref()
            .and_then(|p| std::fs::read(p).ok())
            .and_then(|b| serde_json::from_slice::<Saved>(&b).ok())
            .map(|s| s.items)
            .unwrap_or_default();
        for item in &mut items {
            if item.status == Status::Active {
                item.status = Status::Queued;
            }
        }
        let next_id = items.iter().map(|i| i.id).max().unwrap_or(0) + 1;
        let inner = Arc::new(Inner {
            state: Mutex::new(State {
                items,
                next_id,
                settings: Settings::default(),
                running: HashMap::new(),
                dirty: false,
                closing: false,
            }),
            limiter: Limiter::default(),
            notify: Mutex::new(None),
            store,
        });
        // Saves the list now and then while it changes.
        let weak: Weak<Inner> = Arc::downgrade(&inner);
        std::thread::spawn(move || {
            loop {
                std::thread::sleep(Duration::from_secs(2));
                let Some(inner) = weak.upgrade() else { return };
                inner.save();
            }
        });
        let m = Manager { inner };
        m.schedule();
        m
    }

    /// Called whenever something changes (to redraw the window).
    pub fn set_notify(&self, f: impl Fn() + Send + Sync + 'static) {
        *self.inner.notify.lock().unwrap_or_else(|e| e.into_inner()) = Some(Box::new(f));
    }

    pub fn settings(&self) -> Settings {
        self.inner.lock().settings.clone()
    }

    pub fn set_settings(&self, settings: Settings) {
        self.inner.limiter.set_rate(settings.speed_limit);
        self.inner.lock().settings = settings;
        self.schedule();
    }

    /// All items, newest first.
    pub fn items(&self) -> Vec<Item> {
        let mut items = self.inner.lock().items.clone();
        items.sort_by(|a, b| b.added.cmp(&a.added).then(b.id.cmp(&a.id)));
        items
    }

    pub fn item(&self, id: u64) -> Option<Item> {
        self.inner.lock().items.iter().find(|i| i.id == id).cloned()
    }

    /// Adds a download; it starts when a place is free. `name` is the file
    /// name or title if already known.
    pub fn add(&self, url: &str, source: Source, name: Option<String>, dir: &Path, thumbnail: Option<String>) -> u64 {
        let id = {
            let mut s = self.inner.lock();
            let id = s.next_id;
            s.next_id += 1;
            // Empty until known: the server or the site names it.
            let name = name.map(|n| n.trim().to_string()).unwrap_or_default();
            s.items.push(Item {
                id,
                url: url.trim().to_string(),
                source,
                name,
                dir: dir.to_path_buf(),
                path: None,
                status: Status::Queued,
                error: None,
                total: None,
                done: 0,
                added: now(),
                finished: None,
                thumbnail,
                resume: None,
                speed: 0.0,
                eta: None,
                stage: None,
            });
            s.dirty = true;
            id
        };
        self.schedule();
        self.inner.notify();
        id
    }

    pub fn pause(&self, id: u64) {
        {
            let mut s = self.inner.lock();
            if let Some(stop) = s.running.get(&id) {
                stop.store(true, Ordering::Relaxed);
            }
            if let Some(item) = s.items.iter_mut().find(|i| i.id == id)
                && matches!(item.status, Status::Queued | Status::Active)
            {
                item.status = Status::Paused;
                item.speed = 0.0;
                item.eta = None;
            }
            s.dirty = true;
        }
        self.inner.notify();
    }

    /// Continues a paused or failed download (from where it stopped, if
    /// possible).
    pub fn resume(&self, id: u64) {
        {
            let mut s = self.inner.lock();
            if let Some(item) = s.items.iter_mut().find(|i| i.id == id)
                && matches!(item.status, Status::Paused | Status::Failed)
            {
                item.status = Status::Queued;
                item.error = None;
            }
            s.dirty = true;
        }
        self.schedule();
        self.inner.notify();
    }

    /// Downloads again from the start.
    pub fn restart(&self, id: u64) {
        self.pause(id);
        self.wait_stopped(id);
        if let Some(item) = self.item(id)
            && let Some(p) = &item.path
            && item.status != Status::Done
        {
            let _ = std::fs::remove_file(crate::util::part_path(p));
        }
        self.inner.update(id, |i| {
            i.resume = None;
            i.done = 0;
            i.error = None;
            i.finished = None;
            if i.status != Status::Done || i.path.as_ref().is_some_and(|p| !p.exists()) {
                i.path = None;
            }
            i.status = Status::Queued;
        });
        self.inner.lock().dirty = true;
        self.schedule();
    }

    /// Removes an item from the list; `delete_file` also deletes what was
    /// downloaded.
    pub fn remove(&self, id: u64, delete_file: bool) {
        self.pause(id);
        let item = {
            let mut s = self.inner.lock();
            let pos = s.items.iter().position(|i| i.id == id);
            s.dirty = true;
            pos.map(|p| s.items.remove(p))
        };
        if let Some(item) = item {
            let path = item.path.clone();
            let done = item.status == Status::Done;
            let inner = self.inner.clone();
            // Unfinished parts are deleted once the download has stopped.
            std::thread::spawn(move || {
                let until = Instant::now() + Duration::from_secs(10);
                while inner.lock().running.contains_key(&id) && Instant::now() < until {
                    std::thread::sleep(Duration::from_millis(100));
                }
                if let Some(p) = path {
                    if !done {
                        let _ = std::fs::remove_file(crate::util::part_path(&p));
                    }
                    if delete_file && done {
                        let _ = std::fs::remove_file(&p);
                    }
                }
            });
        }
        self.inner.notify();
    }

    pub fn pause_all(&self) {
        let ids: Vec<u64> = self
            .inner
            .lock()
            .items
            .iter()
            .filter(|i| matches!(i.status, Status::Queued | Status::Active))
            .map(|i| i.id)
            .collect();
        for id in ids {
            self.pause(id);
        }
    }

    pub fn resume_all(&self) {
        {
            let mut s = self.inner.lock();
            for item in s.items.iter_mut().filter(|i| i.status == Status::Paused) {
                item.status = Status::Queued;
            }
            s.dirty = true;
        }
        self.schedule();
        self.inner.notify();
    }

    /// Removes finished downloads from the list (the files stay).
    pub fn clear_finished(&self) {
        {
            let mut s = self.inner.lock();
            s.items.retain(|i| i.status != Status::Done);
            s.dirty = true;
        }
        self.inner.notify();
    }

    /// Total speed of all running downloads, in bytes per second.
    pub fn speed(&self) -> f64 {
        self.inner.lock().items.iter().filter(|i| i.status == Status::Active).map(|i| i.speed).sum()
    }

    /// Whether anything is downloading or waiting to.
    pub fn busy(&self) -> bool {
        self.inner.lock().items.iter().any(|i| matches!(i.status, Status::Active | Status::Queued))
    }

    /// Stops all downloads so that they continue at the next start, and
    /// saves the list.
    pub fn shutdown(&self) {
        {
            let mut s = self.inner.lock();
            s.closing = true;
            for stop in s.running.values() {
                stop.store(true, Ordering::Relaxed);
            }
        }
        let until = Instant::now() + Duration::from_secs(5);
        while !self.inner.lock().running.is_empty() && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(50));
        }
        self.inner.lock().dirty = true;
        self.inner.save();
    }

    fn wait_stopped(&self, id: u64) {
        let until = Instant::now() + Duration::from_secs(10);
        while self.inner.lock().running.contains_key(&id) && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// Starts waiting downloads while there is room.
    fn schedule(&self) {
        let mut s = self.inner.lock();
        if s.closing {
            return;
        }
        // Oldest first.
        let mut waiting: Vec<(i64, u64)> =
            s.items.iter().filter(|i| i.status == Status::Queued).map(|i| (i.added, i.id)).collect();
        waiting.sort();
        for (_, id) in waiting {
            if s.running.len() >= s.settings.max_active.max(1) {
                break;
            }
            let stop = Arc::new(AtomicBool::new(false));
            s.running.insert(id, stop.clone());
            if let Some(item) = s.items.iter_mut().find(|i| i.id == id) {
                item.status = Status::Active;
                item.error = None;
                item.stage = None;
            }
            s.dirty = true;
            let m = self.clone();
            std::thread::spawn(move || {
                m.run(id, &stop);
                {
                    let mut s = m.inner.lock();
                    s.running.remove(&id);
                    s.dirty = true;
                }
                m.schedule();
                m.inner.notify();
            });
        }
    }

    /// Runs one download to its end.
    fn run(&self, id: u64, stop: &AtomicBool) {
        let Some(item) = self.item(id) else { return };
        match &item.source {
            Source::File => self.run_file(id, stop),
            Source::Media(options) => self.run_media(id, options.clone(), stop),
        }
    }

    /// The status a download stopped on request gets.
    fn stopped_status(&self) -> Status {
        if self.inner.lock().closing { Status::Queued } else { Status::Paused }
    }

    fn fail(&self, id: u64, message: String) {
        self.inner.update(id, |i| {
            i.status = Status::Failed;
            i.error = Some(message);
            i.speed = 0.0;
            i.eta = None;
            i.stage = None;
        });
        self.inner.lock().dirty = true;
    }

    fn finish(&self, id: u64, path: PathBuf) {
        let size = std::fs::metadata(&path).map(|m| m.len()).ok();
        self.inner.update(id, |i| {
            i.status = Status::Done;
            i.name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| i.name.clone());
            i.path = Some(path);
            if let Some(s) = size {
                i.total = Some(s);
                i.done = s;
            }
            i.finished = Some(now());
            i.resume = None;
            i.speed = 0.0;
            i.eta = None;
            i.stage = None;
            i.error = None;
        });
        self.inner.lock().dirty = true;
    }

    fn run_file(&self, id: u64, stop: &AtomicBool) {
        let Some(item) = self.item(id) else { return };
        // The file name is chosen once, so that a paused download continues
        // into the same file.
        let path = match &item.path {
            Some(p) => p.clone(),
            None => {
                let name = if !item.name.is_empty() {
                    sanitize_filename(&item.name)
                } else {
                    match http::probe(&item.url) {
                        Ok(p) => p.name,
                        Err(e) => return self.fail(id, capitalize(&format!("{e:#}"))),
                    }
                };
                if let Err(e) = std::fs::create_dir_all(&item.dir) {
                    return self.fail(id, format!("Cannot create {}: {e}", item.dir.display()));
                }
                let p = unique_path(&item.dir, &name);
                self.inner.update(id, |i| {
                    i.name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    i.path = Some(p.clone());
                });
                p
            }
        };
        let connections = self.settings().connections;
        let report = Progress::new(self.inner.clone(), id);
        let job = http::Job { url: &item.url, path: &path, connections, resume: item.resume.clone() };
        match http::download(job, stop, &self.inner.limiter, &report) {
            Ok(Outcome::Finished(p)) => self.finish(id, p),
            Ok(Outcome::Stopped(resume)) => {
                let status = self.stopped_status();
                self.inner.update(id, |i| {
                    i.resume = resume;
                    if i.status == Status::Active {
                        i.status = status;
                    }
                    i.speed = 0.0;
                    i.eta = None;
                });
            }
            Err(f) => {
                self.inner.update(id, |i| i.resume = f.resume);
                self.fail(id, capitalize(&format!("{:#}", f.error)));
            }
        }
    }

    fn run_media(&self, id: u64, mut options: media::Options, stop: &AtomicBool) {
        let Some(item) = self.item(id) else { return };
        let settings = self.settings();
        if options.cookies_from.is_none() {
            options.cookies_from = settings.cookies_from.clone();
        }
        // Browsers whose login is tried when the site refuses.
        let mut fallbacks: Vec<&str> =
            if settings.auto_cookies && options.cookies_from.is_none() { browsers().to_vec() } else { Vec::new() };
        fallbacks.reverse();
        let report = Progress::new(self.inner.clone(), id);
        loop {
            let job = media::Job {
                url: &item.url,
                dir: &item.dir,
                options: &options,
                rate_limit: settings.speed_limit,
                fragments: settings.connections.min(8),
            };
            match media::download(job, stop, &report) {
                Ok(Some(path)) => return self.finish(id, path),
                Ok(None) => {
                    let status = self.stopped_status();
                    return self.inner.update(id, |i| {
                        if i.status == Status::Active {
                            i.status = status;
                        }
                        i.speed = 0.0;
                        i.eta = None;
                        i.stage = None;
                    });
                }
                Err(e) if e.kind == ErrorKind::Unsupported => {
                    // Not a site yt-dlp knows: perhaps a plain file.
                    self.inner.update(id, |i| {
                        i.source = Source::File;
                        i.name.clear();
                        i.stage = None;
                    });
                    return self.run_file(id, stop);
                }
                Err(e) if matches!(e.kind, ErrorKind::Login | ErrorKind::Forbidden) && !fallbacks.is_empty() => {
                    let browser = fallbacks.pop().expect("not empty");
                    options.cookies_from = Some(browser.to_string());
                    self.inner.update(id, |i| i.stage = Some(format!("Trying with your {browser} login")));
                }
                Err(e) => return self.fail(id, e.message),
            }
        }
    }
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().chain(c).collect(),
        None => String::new(),
    }
}

/// Feeds a download's progress into its item, with a smoothed speed.
struct Progress {
    inner: Arc<Inner>,
    id: u64,
    rate: Mutex<(Instant, u64, f64)>,
}

impl Progress {
    fn new(inner: Arc<Inner>, id: u64) -> Self {
        Progress { inner, id, rate: Mutex::new((Instant::now(), 0, 0.0)) }
    }

    /// Speed from the bytes done, smoothed.
    fn speed(&self, done: u64) -> f64 {
        let mut r = self.rate.lock().unwrap_or_else(|e| e.into_inner());
        let elapsed = r.0.elapsed().as_secs_f64();
        if elapsed >= 0.5 {
            let inst = done.saturating_sub(r.1) as f64 / elapsed;
            r.2 = if r.2 == 0.0 { inst } else { 0.6 * r.2 + 0.4 * inst };
            r.0 = Instant::now();
            r.1 = done;
        } else if done < r.1 {
            // Started over.
            *r = (Instant::now(), done, 0.0);
        }
        r.2
    }
}

impl http::Report for Progress {
    fn progress(&self, done: u64, total: Option<u64>) {
        let speed = self.speed(done);
        self.inner.update(self.id, |i| {
            i.done = done;
            i.total = total;
            i.speed = speed;
            i.eta = match total {
                Some(t) if speed > 0.0 && t > done => Some(((t - done) as f64 / speed) as u64),
                _ => None,
            };
        });
    }

    fn checkpoint(&self, resume: &Resume) {
        let r = resume.clone();
        self.inner.update(self.id, |i| i.resume = Some(r));
        self.inner.lock().dirty = true;
    }
}

impl media::Report for Progress {
    fn progress(&self, done: u64, total: Option<u64>, speed: Option<f64>, eta: Option<u64>) {
        let speed = speed.unwrap_or_else(|| self.speed(done));
        self.inner.update(self.id, |i| {
            i.done = done;
            i.total = total;
            i.speed = speed;
            i.eta = eta;
            i.stage = None;
        });
    }

    fn stage(&self, stage: &str) {
        let s = stage.to_string();
        self.inner.update(self.id, |i| {
            i.stage = Some(s);
            i.speed = 0.0;
            i.eta = None;
        });
    }

    fn title(&self, title: &str) {
        if !title.is_empty() && title != "NA" {
            let t = title.to_string();
            self.inner.update(self.id, |i| i.name = t);
        }
    }
}
