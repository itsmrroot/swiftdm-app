//! Background work (looking up a link, installing a helper, downloading an
//! update) with progress that the UI can read at any time.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, channel};

use anyhow::{Result, anyhow};
use eframe::egui;

/// Bytes done and the total (0: unknown).
#[derive(Default)]
pub struct Progress {
    done: AtomicU64,
    total: AtomicU64,
}

impl Progress {
    pub fn set(&self, done: u64, total: Option<u64>) {
        self.done.store(done, Ordering::Relaxed);
        self.total.store(total.unwrap_or(0), Ordering::Relaxed);
    }

    pub fn done(&self) -> u64 {
        self.done.load(Ordering::Relaxed)
    }

    pub fn total(&self) -> Option<u64> {
        Some(self.total.load(Ordering::Relaxed)).filter(|t| *t > 0)
    }

    pub fn fraction(&self) -> Option<f32> {
        self.total().map(|t| (self.done() as f64 / t as f64).clamp(0.0, 1.0) as f32)
    }
}

/// A unit of background work producing a `T`.
pub struct Job<T> {
    pub progress: Arc<Progress>,
    cancel: Arc<AtomicBool>,
    rx: Receiver<Result<T>>,
}

impl<T: Send + 'static> Job<T> {
    pub fn spawn(ctx: &egui::Context, work: impl FnOnce(&Progress, &AtomicBool) -> Result<T> + Send + 'static) -> Self {
        let progress = Arc::new(Progress::default());
        let cancel = Arc::new(AtomicBool::new(false));
        let (tx, rx) = channel();
        let (p, c, ctx) = (progress.clone(), cancel.clone(), ctx.clone());
        std::thread::spawn(move || {
            let result = catch_unwind(AssertUnwindSafe(|| work(&p, &c)))
                .unwrap_or_else(|_| Err(anyhow!("an internal error occurred (please report it)")));
            let _ = tx.send(result);
            ctx.request_repaint();
        });
        Self { progress, cancel, rx }
    }

    /// The result, once the work has finished.
    pub fn poll(&self) -> Option<Result<T>> {
        self.rx.try_recv().ok()
    }

    pub fn stop(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    pub fn stopping(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }
}
