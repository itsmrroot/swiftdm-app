//! The helper programs (yt-dlp, FFmpeg, Deno) as the app shows them:
//! which are there, installing them one after the other, and keeping
//! yt-dlp up to date (sites change often, and yt-dlp follows within days).

use std::collections::VecDeque;
use std::path::PathBuf;
use std::time::Duration;

use eframe::egui::{self, RichText, Ui};
use egui_phosphor::regular as icon;
use swiftdm::tools::{self, Status, Tool};
use swiftdm::util::format_size;

use crate::jobs::Job;
use crate::theme::{self, Palette};

pub struct Helpers {
    /// Known state of each tool; `None` until first looked at.
    status: Option<Vec<Status>>,
    looking: Option<Job<Vec<Status>>>,
    installing: Option<(Tool, Job<PathBuf>)>,
    queue: VecDeque<Tool>,
    update_check: Option<Job<bool>>,
    pub error: Option<String>,
    /// Shown after the first set was installed.
    pub just_installed: bool,
}

impl Helpers {
    pub fn new(ctx: &egui::Context, keep_ytdlp_updated: bool) -> Self {
        let mut h = Helpers {
            status: None,
            looking: None,
            installing: None,
            queue: VecDeque::new(),
            update_check: None,
            error: None,
            just_installed: false,
        };
        h.refresh(ctx);
        if keep_ytdlp_updated {
            // Only SwiftDM's own copy is updated; a system copy belongs to
            // its package manager.
            h.update_check = Some(Job::spawn(ctx, |_, _| {
                let own = Tool::YtDlp.managed_path();
                if !own.is_file() {
                    return Ok(false);
                }
                let installed = tools::version(Tool::YtDlp, &own).unwrap_or_default();
                let latest = tools::latest_ytdlp()?;
                Ok(!latest.is_empty() && latest != installed)
            }));
        }
        h
    }

    /// Looks at the tools again (in the background).
    pub fn refresh(&mut self, ctx: &egui::Context) {
        if self.looking.is_none() {
            self.looking = Some(Job::spawn(ctx, |_, _| Ok(Tool::ALL.iter().map(|t| tools::status(*t)).collect())));
        }
    }

    pub fn poll(&mut self, ctx: &egui::Context) {
        if let Some(job) = &self.looking
            && let Some(r) = job.poll()
        {
            self.looking = None;
            if let Ok(s) = r {
                self.status = Some(s);
            }
        }
        if let Some(job) = &self.update_check
            && let Some(r) = job.poll()
        {
            self.update_check = None;
            match r {
                Ok(true) => self.install(ctx, &[Tool::YtDlp]),
                Ok(false) => {}
                Err(e) => log::info!("yt-dlp update check failed: {e:#}"),
            }
        }
        if let Some((tool, job)) = &self.installing {
            match job.poll() {
                None => ctx.request_repaint_after(Duration::from_millis(150)),
                Some(r) => {
                    let tool = *tool;
                    self.installing = None;
                    if let Err(e) = r {
                        if !e.to_string().contains("cancelled") {
                            self.error = Some(format!("{} could not be installed: {e:#}", tool.name()));
                        }
                        self.queue.clear();
                    } else if self.queue.is_empty() {
                        self.just_installed = true;
                    }
                    self.refresh(ctx);
                }
            }
        }
        if self.installing.is_none()
            && let Some(tool) = self.queue.pop_front()
        {
            self.installing = Some((
                tool,
                Job::spawn(ctx, move |progress, cancel| {
                    tools::install(tool, &|done, total| progress.set(done, total), cancel)
                }),
            ));
        }
    }

    /// Installs (or updates) `which`, one after the other.
    pub fn install(&mut self, ctx: &egui::Context, which: &[Tool]) {
        self.error = None;
        for t in which {
            let busy = self.installing.as_ref().is_some_and(|(x, _)| x == t);
            if !busy && !self.queue.contains(t) {
                self.queue.push_back(*t);
            }
        }
        self.poll(ctx);
    }

    /// Installs whatever is missing.
    pub fn install_missing(&mut self, ctx: &egui::Context) {
        let missing: Vec<Tool> = Tool::ALL.into_iter().filter(|t| !self.installed(*t)).collect();
        self.install(ctx, &missing);
    }

    pub fn cancel(&mut self) {
        self.queue.clear();
        if let Some((_, job)) = &self.installing {
            job.stop();
        }
    }

    pub fn busy(&self) -> bool {
        self.installing.is_some() || !self.queue.is_empty()
    }

    /// Whether `tool` is known to be there (yes until first looked at, so
    /// that nothing flashes at start).
    pub fn installed(&self, tool: Tool) -> bool {
        match &self.status {
            Some(s) => s.iter().any(|s| s.tool == tool && s.installed()),
            None => true,
        }
    }

    pub fn known(&self) -> bool {
        self.status.is_some()
    }

    /// Whether YouTube can work (Deno or Node.js present).
    pub fn has_js(&self) -> bool {
        self.installed(Tool::Deno) || (self.known() && tools::find_on_system("node").is_some())
    }

    /// Progress of the running installation, as a line for the UI.
    pub fn progress_line(&self) -> Option<(String, Option<f32>)> {
        let (tool, job) = self.installing.as_ref()?;
        let p = &job.progress;
        let size = match p.total() {
            Some(t) => format!("{} of {}", format_size(p.done()), format_size(t)),
            None => format_size(p.done()),
        };
        let more =
            if self.queue.is_empty() { String::new() } else { format!(" · {} more after this", self.queue.len()) };
        Some((format!("Installing {} — {size}{more}", tool.name()), p.fraction()))
    }

    /// The install progress, if any, as a bar with a Cancel button.
    pub fn progress_ui(&mut self, ui: &mut Ui, p: &Palette) {
        if let Some((line, fraction)) = self.progress_line() {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(RichText::new(line).color(p.weak));
                if ui.small_button("Cancel").clicked() {
                    self.cancel();
                }
            });
            theme::bar(ui, p, fraction, p.accent, ui.available_width().min(520.0));
        }
        if let Some(e) = &self.error {
            ui.label(RichText::new(format!("{}  {e}", icon::WARNING)).color(p.danger));
        }
    }

    /// The table on the Settings page.
    pub fn table(&mut self, ui: &mut Ui, p: &Palette) {
        let ctx = ui.ctx().clone();
        let Some(status) = self.status.clone() else {
            ui.spinner();
            return;
        };
        for s in status {
            ui.horizontal(|ui| {
                let (glyph, color) =
                    if s.installed() { (icon::CHECK_CIRCLE, p.success) } else { (icon::CIRCLE_DASHED, p.weak) };
                ui.label(RichText::new(glyph).size(20.0).color(color));
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    ui.horizontal(|ui| {
                        ui.label(theme::semibold(s.tool.name(), 15.0));
                        match (&s.version, s.installed()) {
                            (Some(v), true) => {
                                ui.label(RichText::new(v).color(p.weak).size(13.0));
                            }
                            (None, true) => {}
                            (_, false) => {
                                theme::pill(ui, p, "Not installed", p.warning);
                            }
                        }
                    });
                    ui.label(RichText::new(s.tool.purpose()).color(p.weak).size(13.0));
                    if let Some(path) = &s.path {
                        let source =
                            if s.managed { "SwiftDM's own copy".to_string() } else { path.display().to_string() };
                        ui.label(RichText::new(source).color(p.weak).size(12.0));
                    }
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if !tools::installable(s.tool) {
                        return;
                    }
                    let busy =
                        self.installing.as_ref().is_some_and(|(t, _)| *t == s.tool) || self.queue.contains(&s.tool);
                    if busy {
                        ui.spinner();
                    } else if s.managed {
                        if ui.button("Remove").on_hover_text("Delete SwiftDM's own copy").clicked() {
                            if let Err(e) = tools::uninstall(s.tool) {
                                self.error = Some(format!("{e:#}"));
                            }
                            self.refresh(&ctx);
                        }
                        if ui.button("Update").on_hover_text("Download the newest version").clicked() {
                            self.install(&ctx, &[s.tool]);
                        }
                    } else {
                        let label = if s.installed() { "Install own copy" } else { "Install" };
                        let tip = format!("Download from the official source (about {})", s.tool.download_size());
                        if ui.button(label).on_hover_text(tip).clicked() {
                            self.install(&ctx, &[s.tool]);
                        }
                    }
                });
            });
            ui.add_space(6.0);
        }
        self.progress_ui(ui, p);
    }
}
