//! A log of what happened, in SwiftDM's folder (`swiftdm.log`): starting,
//! the graphics in use, errors and crashes. Kept small; useful to send
//! along with a problem report.

use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

pub fn path() -> PathBuf {
    swiftdm::paths::data_dir().join("swiftdm.log")
}

struct FileLog(Mutex<std::fs::File>);

impl log::Log for FileLog {
    fn enabled(&self, m: &log::Metadata) -> bool {
        // Our own messages from info up; other crates' only when they matter.
        let ours = m.target().starts_with("swiftdm");
        let graphics = m.target().starts_with("eframe") || m.target().starts_with("egui_wgpu");
        m.level() <= if ours || graphics { log::Level::Info } else { log::Level::Warn }
    }

    fn log(&self, r: &log::Record) {
        if self.enabled(r.metadata()) {
            let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
            if let Ok(mut f) = self.0.lock() {
                let _ = writeln!(f, "{now} {:<5} {}: {}", r.level(), r.target(), r.args());
            }
        }
    }

    fn flush(&self) {
        if let Ok(mut f) = self.0.lock() {
            let _ = f.flush();
        }
    }
}

/// Starts logging to the file, and records crashes there too.
pub fn start() {
    let file = path();
    if let Some(dir) = file.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    // A new log once the old one grows past 1 MB.
    if std::fs::metadata(&file).is_ok_and(|m| m.len() > 1024 * 1024) {
        let _ = std::fs::rename(&file, file.with_extension("old.log"));
    }
    let Ok(f) = std::fs::OpenOptions::new().create(true).append(true).open(&file) else { return };
    if log::set_boxed_logger(Box::new(FileLog(Mutex::new(f)))).is_ok() {
        log::set_max_level(log::LevelFilter::Info);
    }
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        log::error!("crash: {info}");
        log::logger().flush();
        previous(info);
    }));
    log::info!("SwiftDM {} starting on {} {}", env!("CARGO_PKG_VERSION"), std::env::consts::OS, std::env::consts::ARCH);
}
