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

use crate::i18n::{self, tr, trf, trlf};
use crate::jobs::Job;
use crate::text;
use crate::theme::{self, Palette};

pub struct Helpers {
    /// Known state of each tool; `None` until first looked at.
    status: Option<Vec<Status>>,
    looking: Option<Job<Vec<Status>>>,
    /// Something changed while looking: look again afterwards.
    look_again: bool,
    installing: Option<(Tool, Job<PathBuf>)>,
    queue: VecDeque<Tool>,
    update_check: Option<Job<bool>>,
    /// "Update" was clicked: looking for a newer version first.
    checking: Option<(Tool, String, Job<tools::Check>)>,
    /// An update the person asked for that is being installed: the tool, the
    /// version it had and the one it gets (when known).
    updating: Option<(Tool, String, Option<String>)>,
    /// What an update did, for the person (in logical order).
    pub notice: Option<String>,
    /// In logical order (see `theme::paragraph`).
    pub error: Option<String>,
    /// Shown after the first set was installed.
    pub just_installed: bool,
}

impl Helpers {
    pub fn new(ctx: &egui::Context, keep_ytdlp_updated: bool) -> Self {
        let mut h = Helpers {
            status: None,
            looking: None,
            look_again: false,
            installing: None,
            queue: VecDeque::new(),
            update_check: None,
            checking: None,
            updating: None,
            notice: None,
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
        if self.looking.is_some() {
            // What it finds may be from before the change.
            self.look_again = true;
            return;
        }
        self.looking = Some(Job::spawn(ctx, |_, _| Ok(Tool::ALL.iter().map(|t| tools::status(*t)).collect())));
    }

    pub fn poll(&mut self, ctx: &egui::Context) {
        if let Some(job) = &self.looking
            && let Some(r) = job.poll()
        {
            self.looking = None;
            if let Ok(s) = r {
                self.status = Some(s);
            }
            if std::mem::take(&mut self.look_again) {
                self.refresh(ctx);
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
        if let Some((tool, installed, job)) = &self.checking
            && let Some(r) = job.poll()
        {
            let (tool, installed) = (*tool, installed.clone());
            self.checking = None;
            match r {
                Ok(tools::Check::Current) => {
                    self.notice = Some(match installed.as_str() {
                        "" => trlf("{tool} is up to date.", &[("tool", &tool.name())]),
                        v => trlf("{tool} is up to date ({version}).", &[("tool", &tool.name()), ("version", &v)]),
                    });
                }
                Ok(tools::Check::Newer(latest)) => {
                    self.updating = Some((tool, installed, latest));
                    self.install(ctx, &[tool]);
                }
                Err(e) => {
                    self.error = Some(trlf(
                        "{tool} could not be checked for updates: {error}",
                        &[("tool", &tool.name()), ("error", &i18n::engine(&format!("{e:#}")))],
                    ));
                }
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
                            self.error = Some(trlf(
                                "{tool} could not be installed: {error}",
                                &[("tool", &tool.name()), ("error", &i18n::engine(&format!("{e:#}")))],
                            ));
                        }
                        self.queue.clear();
                        self.updating = None;
                    } else if let Some((t, old, new)) = self.updating.take_if(|(t, ..)| *t == tool) {
                        self.notice = Some(match new {
                            Some(new) => trlf(
                                "{tool} updated from {old} to {new}. No restart is needed.",
                                &[("tool", &t.name()), ("old", &old), ("new", &new)],
                            ),
                            None => trlf("{tool} was updated. No restart is needed.", &[("tool", &t.name())]),
                        });
                        if self.queue.is_empty() {
                            self.just_installed = true;
                        }
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

    /// "Update" for `tool`: looks for a newer version, installs it and says
    /// what happened (see `notice`).
    pub fn update(&mut self, ctx: &egui::Context, tool: Tool) {
        if self.checking.is_some() {
            return;
        }
        self.error = None;
        self.notice = None;
        let installed =
            self.status.iter().flatten().find(|s| s.tool == tool).and_then(|s| s.version.clone()).unwrap_or_default();
        let known = installed.clone();
        self.checking = Some((tool, installed, Job::spawn(ctx, move |_, _| tools::check_update(tool, &known))));
    }

    pub fn cancel(&mut self) {
        self.queue.clear();
        if let Some((_, job)) = &self.installing {
            job.stop();
        }
    }

    pub fn busy(&self) -> bool {
        self.installing.is_some() || !self.queue.is_empty() || self.checking.is_some()
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

    /// Progress of the running installation, as a line for the UI (in
    /// logical order).
    pub fn progress_line(&self) -> Option<(String, Option<f32>)> {
        let (tool, job) = self.installing.as_ref()?;
        let p = &job.progress;
        let size = match p.total() {
            Some(t) => trlf("{done} of {total}", &[("done", &format_size(p.done())), ("total", &format_size(t))]),
            None => format_size(p.done()),
        };
        let more = if self.queue.is_empty() {
            String::new()
        } else {
            format!(" · {}", trlf("{n} more after this", &[("n", &self.queue.len())]))
        };
        Some((
            format!("{}{more}", trlf("Installing {tool} — {size}", &[("tool", &tool.name()), ("size", &size)])),
            p.fraction(),
        ))
    }

    /// The install progress, if any, as a bar with a Cancel button.
    pub fn progress_ui(&mut self, ui: &mut Ui, p: &Palette) {
        if let Some((line, fraction)) = self.progress_line() {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(text::rich(&line).color(p.weak));
                if ui.small_button(tr("Cancel")).clicked() {
                    self.cancel();
                }
            });
            theme::bar(ui, p, fraction, p.accent, ui.available_width().min(520.0));
        }
        let mut dismiss = false;
        if let Some(e) = &self.error {
            ui.horizontal(|ui| {
                ui.label(RichText::new(icon::WARNING).color(p.danger));
                if ui.small_button(icon::X).on_hover_text(tr("Close")).clicked() {
                    dismiss = true;
                }
                theme::paragraph(ui, e, 14.0, p.danger);
            });
        }
        if dismiss {
            self.error = None;
        }
        let mut close_notice = false;
        if let Some(n) = &self.notice {
            ui.horizontal(|ui| {
                ui.label(RichText::new(icon::CHECK_CIRCLE).color(p.success));
                if ui.small_button(icon::X).on_hover_text(tr("Close")).clicked() {
                    close_notice = true;
                }
                theme::paragraph(ui, n, 14.0, p.success);
            });
        }
        if close_notice {
            self.notice = None;
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
                ui.label(RichText::new(glyph).size(theme::scale::ICON_LG).color(color));
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    ui.horizontal(|ui| {
                        ui.label(theme::semibold(s.tool.name(), theme::scale::BODY));
                        match (&s.version, s.installed()) {
                            (Some(v), true) => {
                                ui.label(RichText::new(v).color(p.weak).size(theme::scale::CAPTION));
                            }
                            (None, true) => {}
                            (_, false) => {
                                theme::pill(ui, p, tr("Not installed"), p.warning);
                            }
                        }
                    });
                    ui.label(RichText::new(purpose(s.tool)).color(p.weak).size(theme::scale::CAPTION));
                    if let Some(path) = &s.path {
                        let source =
                            if s.managed { tr("SwiftDM's own copy").to_string() } else { path.display().to_string() };
                        ui.label(text::rich(&source).color(p.weak).size(theme::scale::CAPTION));
                    }
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if !tools::installable(s.tool) {
                        return;
                    }
                    let busy = self.installing.as_ref().is_some_and(|(t, _)| *t == s.tool)
                        || self.queue.contains(&s.tool)
                        || self.checking.as_ref().is_some_and(|(t, ..)| *t == s.tool);
                    if busy {
                        ui.spinner();
                    } else if s.managed {
                        if ui.button(tr("Remove")).on_hover_text(tr("Delete SwiftDM's own copy")).clicked() {
                            if let Err(e) = tools::uninstall(s.tool) {
                                self.error = Some(i18n::engine(&format!("{e:#}")));
                            }
                            self.refresh(&ctx);
                        }
                        if ui.button(tr("Update")).on_hover_text(tr("Download the newest version")).clicked() {
                            self.update(&ctx, s.tool);
                        }
                    } else {
                        let label = if s.installed() { tr("Install own copy") } else { tr("Install") };
                        let tip = trf(
                            "Download from the official source (about {size})",
                            &[("size", &s.tool.download_size())],
                        );
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

/// What `tool` is for, in the interface language.
pub fn purpose(tool: Tool) -> &'static str {
    match tool {
        Tool::YtDlp => tr("Reads videos and music from 1800+ sites"),
        Tool::Ffmpeg => tr("Joins video with sound and converts to MP3"),
        Tool::Deno => tr("Runs the JavaScript YouTube needs"),
    }
}
