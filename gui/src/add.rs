//! Adding a link: finding out what it is (a video, a playlist, a file),
//! and the choices before downloading it.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

use eframe::egui::{self, Align, CornerRadius, Layout, RichText, Ui, Vec2};
use egui_phosphor::regular as icon;
use swiftdm::http::{self, Probe};
use swiftdm::manager::{self, Source};
use swiftdm::media::{self, AudioFormat, Container, ErrorKind, Found, Info, Mode};
use swiftdm::util::{FileKind, format_eta, format_size, sanitize_filename};

use crate::i18n::{self, icon_label, tr, trf, trl, trlf};
use crate::instance::Incoming;
use crate::jobs::Job;
use crate::list::kind_icon;
use crate::settings::{self, Settings};
use crate::text;
use crate::theme::{self, Palette};
use crate::thumbs::Thumbs;

/// What a link turned out to be.
enum Looked {
    Media(Found),
    File(Probe),
}

/// Why looking failed.
struct LookError {
    kind: ErrorKind,
    message: String,
}

enum Stage {
    /// Not looked up yet: only a few links are looked at at once.
    Waiting,
    Looking(Job<std::result::Result<Looked, LookError>>),
    Media(Info),
    Playlist {
        title: String,
        entries: Vec<media::Entry>,
        selected: Vec<bool>,
    },
    File(Probe),
    Failed(LookError),
}

/// A download the dialog asks for.
pub struct NewDownload {
    pub url: String,
    pub source: Source,
    pub name: Option<String>,
    pub dir: PathBuf,
    pub thumbnail: Option<String>,
    /// From the browser: its cookies and page.
    pub headers: Vec<(String, String)>,
}

pub enum Action {
    None,
    Close,
    Start(Vec<NewDownload>),
    /// Install the missing helpers, then look again.
    InstallHelpers,
    /// Look at the link again.
    Retry,
}

pub struct AddDialog {
    pub url: String,
    stage: Stage,
    /// Start right away with the usual choices, without showing them.
    pub quick: bool,
    dir: PathBuf,
    /// Video and music choices.
    options: media::Options,
    file_name: String,
    /// Playlists: into a folder named after it.
    own_folder: bool,
    /// From the browser: its cookies and page.
    headers: Vec<(String, String)>,
    /// The browser asked for the link as a plain file.
    as_file: bool,
    /// The file name the browser chose.
    name_hint: Option<String>,
    /// "Only a part": from and to, as typed.
    part: Option<(String, String)>,
    /// Downloaded before: its name and when.
    pub before: Option<(String, i64)>,
}

/// Finds out what `url` is. Tries the logins of the usual browsers when a
/// site refuses, like downloads do.
fn look(
    url: &str,
    net: &http::Net,
    as_file: bool,
    s: &manager::Settings,
    stop: &AtomicBool,
) -> std::result::Result<Looked, LookError> {
    let other = |e: anyhow::Error| LookError { kind: ErrorKind::Other, message: capitalize(&format!("{e:#}")) };
    if as_file {
        return http::probe_with(url, net).map(Looked::File).map_err(other);
    }
    let by_site = media::is_media_site(url);
    if !by_site {
        // A plain file needs no yt-dlp; a web page may hold a video.
        match http::probe_with(url, net) {
            Ok(p) if !p.is_web_page() => return Ok(Looked::File(p)),
            Ok(_) => {}
            Err(e) if media::looks_like_file(url) => return Err(other(e)),
            Err(_) => {}
        }
    }
    let mut browsers: Vec<Option<&str>> = vec![s.cookies_from.as_deref()];
    // Only for video sites (see manager.rs).
    if s.auto_cookies && s.cookies_from.is_none() && by_site {
        browsers.extend(manager::browsers().iter().map(|b| Some(*b)));
    }
    let mut last = None;
    for cookies in browsers {
        if stop.load(Ordering::Relaxed) {
            break;
        }
        match media::inspect(url, cookies, s.proxy.as_deref(), stop) {
            Ok(found) => return Ok(Looked::Media(found)),
            Err(e) if matches!(e.kind, ErrorKind::Login | ErrorKind::Forbidden) => {
                last.get_or_insert(e);
            }
            // This browser's login cannot be read: the next one may do.
            Err(e) if e.kind == ErrorKind::Cookies && last.is_some() => {}
            Err(e) if e.kind == ErrorKind::Unsupported => {
                // A page without a video: perhaps still worth saving.
                return http::probe_with(url, net).map(Looked::File).map_err(|_| LookError {
                    kind: ErrorKind::Unsupported,
                    message: trl("No video, music or file was found at this link.").into(),
                });
            }
            Err(e) => return Err(LookError { kind: e.kind, message: e.message }),
        }
    }
    let e = last.unwrap_or(media::MediaError { kind: ErrorKind::Other, message: trl("Cancelled").into() });
    Err(LookError { kind: e.kind, message: e.message })
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
}

/// `1:30`, `90` or `1:02:03` in seconds.
pub fn parse_time(t: &str) -> Option<u32> {
    let t = t.trim();
    if t.is_empty() {
        return None;
    }
    t.split(':').try_fold(0u32, |acc, part| Some(acc.checked_mul(60)? + part.trim().parse::<u32>().ok()?))
}

impl AddDialog {
    /// A link from the browser (or another start of the app): `request`'s
    /// mode, headers and file name come along.
    pub fn new(request: Incoming, settings: &Settings, quick: bool) -> Self {
        let mut options = settings.media_options();
        match request.mode.as_str() {
            "audio" => options.mode = Mode::Audio,
            "video" => options.mode = Mode::Video,
            _ => {}
        }
        AddDialog {
            url: request.url.trim().to_string(),
            stage: Stage::Waiting,
            quick,
            dir: settings.download_dir.clone(),
            options,
            file_name: String::new(),
            own_folder: true,
            headers: request.headers,
            as_file: request.mode == "file",
            name_hint: request.name.filter(|n| !n.trim().is_empty()),
            part: None,
            before: None,
        }
    }

    /// Looks up the link (again).
    pub fn start(&mut self, ctx: &egui::Context, settings: &Settings) {
        let (url, s, as_file) = (self.url.clone(), settings.manager(), self.as_file);
        let net = http::Net { proxy: s.proxy.clone(), headers: self.headers.clone() }.sanitized();
        self.stage = Stage::Looking(Job::spawn(ctx, move |_, stop| Ok(look(&url, &net, as_file, &s, stop))));
    }

    /// Being looked up, or waiting for that.
    pub fn looking(&self) -> bool {
        matches!(self.stage, Stage::Looking(_) | Stage::Waiting)
    }

    pub fn waiting(&self) -> bool {
        matches!(self.stage, Stage::Waiting)
    }

    /// Takes in the result of looking. In quick mode, returns the download
    /// to start when no choice is needed.
    pub fn poll(&mut self) -> Action {
        let Stage::Looking(job) = &self.stage else { return Action::None };
        let Some(result) = job.poll() else { return Action::None };
        self.stage = match result {
            Ok(Ok(Looked::Media(Found::Single(info)))) => Stage::Media(info),
            Ok(Ok(Looked::Media(Found::Playlist { title, entries }))) => {
                let selected = vec![true; entries.len()];
                Stage::Playlist { title, entries, selected }
            }
            Ok(Ok(Looked::File(probe))) => {
                self.file_name = self.name_hint.clone().unwrap_or_else(|| probe.name.clone());
                Stage::File(probe)
            }
            Ok(Err(e)) => Stage::Failed(e),
            Err(e) => Stage::Failed(LookError { kind: ErrorKind::Other, message: format!("{e:#}") }),
        };
        if self.quick {
            match &self.stage {
                Stage::Media(_) | Stage::File(_) => return Action::Start(self.downloads()),
                // A playlist or a problem: show the dialog after all.
                _ => self.quick = false,
            }
        }
        Action::None
    }

    fn options(&self) -> media::Options {
        let mut o = self.options.clone();
        o.section = self.part.as_ref().and_then(|(a, b)| Some((parse_time(a)?, parse_time(b)?))).filter(|(a, b)| b > a);
        o
    }

    fn downloads(&self) -> Vec<NewDownload> {
        match &self.stage {
            Stage::Media(info) => vec![NewDownload {
                url: self.url.clone(),
                source: Source::Media(self.options()),
                name: Some(info.title.clone()).filter(|t| !t.is_empty()),
                dir: self.dir.clone(),
                thumbnail: info.thumbnail.clone(),
                headers: self.headers.clone(),
            }],
            Stage::File(_) => vec![NewDownload {
                url: self.url.clone(),
                source: Source::File,
                name: Some(sanitize_filename(&self.file_name)),
                dir: self.dir.clone(),
                thumbnail: None,
                headers: self.headers.clone(),
            }],
            Stage::Playlist { title, entries, selected } => {
                let dir = if self.own_folder && !title.is_empty() {
                    self.dir.join(sanitize_filename(title))
                } else {
                    self.dir.clone()
                };
                entries
                    .iter()
                    .zip(selected)
                    .filter(|(_, on)| **on)
                    .map(|(e, _)| NewDownload {
                        url: e.url.clone(),
                        source: Source::Media(self.options()),
                        name: Some(e.title.clone()),
                        dir: dir.clone(),
                        thumbnail: None,
                        headers: Vec::new(),
                    })
                    .collect()
            }
            _ => Vec::new(),
        }
    }

    /// The dialog. `ffmpeg`: whether FFmpeg is installed.
    pub fn show(&mut self, ctx: &egui::Context, p: &Palette, thumbs: &mut Thumbs, ffmpeg: bool) -> Action {
        if self.quick {
            return Action::None;
        }
        let mut action = Action::None;
        let modal = egui::Modal::new(egui::Id::new("add")).show(ctx, |ui| {
            ui.set_width(560.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new(icon::LINK).size(22.0).color(p.accent));
                ui.label(theme::semibold(tr("New download"), 18.0));
            });
            ui.add(egui::Label::new(RichText::new(&self.url).color(p.weak).size(12.5)).truncate());
            ui.add_space(10.0);
            if let Some((name, when)) = &self.before {
                let when = crate::list::date(*when);
                theme::notice(
                    ui,
                    p,
                    p.warning,
                    icon::COPY,
                    &trlf("Downloaded before ({when}): {name}", &[("when", &when), ("name", name)]),
                );
                ui.add_space(8.0);
            }
            match &mut self.stage {
                stage @ (Stage::Looking(_) | Stage::Waiting) => {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(tr("Looking at the link…"));
                    });
                    ui.add_space(8.0);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if theme::secondary_button(ui, tr("Cancel")).clicked() {
                            if let Stage::Looking(job) = stage {
                                job.stop();
                            }
                            action = Action::Close;
                        }
                    });
                    return;
                }
                Stage::Failed(e) => {
                    let (glyph, color) = match e.kind {
                        ErrorKind::NoYtDlp | ErrorKind::NeedsJs | ErrorKind::NeedsFfmpeg => {
                            (icon::PUZZLE_PIECE, p.accent)
                        }
                        _ => (icon::WARNING_CIRCLE, p.danger),
                    };
                    theme::notice(ui, p, color, glyph, &i18n::engine(&e.message));
                    ui.add_space(10.0);
                    let needs_helper =
                        matches!(e.kind, ErrorKind::NoYtDlp | ErrorKind::NeedsJs | ErrorKind::NeedsFfmpeg);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if needs_helper {
                            if theme::primary_button(ui, p, &format!("  {}  ", tr("Install helpers")), true).clicked() {
                                action = Action::InstallHelpers;
                            }
                        } else if theme::primary_button(ui, p, &format!("  {}  ", tr("Try again")), true).clicked() {
                            action = Action::Retry;
                        }
                        if e.kind != ErrorKind::Unsupported
                            && !needs_helper
                            && theme::secondary_button(ui, tr("Download as a file"))
                                .on_hover_text(tr("Save whatever the link gives"))
                                .clicked()
                        {
                            action = Action::Start(vec![NewDownload {
                                url: self.url.clone(),
                                source: Source::File,
                                name: None,
                                dir: self.dir.clone(),
                                thumbnail: None,
                                headers: self.headers.clone(),
                            }]);
                        }
                        if theme::secondary_button(ui, tr("Close")).clicked() {
                            action = Action::Close;
                        }
                    });
                    return;
                }
                Stage::Media(info) => media_card(ui, p, info, thumbs),
                Stage::Playlist { title, entries, selected } => playlist_card(ui, p, title, entries, selected),
                Stage::File(probe) => file_card(ui, p, probe, &mut self.file_name),
            }
            ui.add_space(12.0);

            if !matches!(self.stage, Stage::File(_)) {
                let heights: Vec<u32> = match &self.stage {
                    Stage::Media(info) if !info.heights.is_empty() => {
                        settings::HEIGHTS.iter().copied().filter(|h| info.heights.iter().any(|x| x >= h)).collect()
                    }
                    _ => settings::HEIGHTS[2..].to_vec(),
                };
                ui.horizontal(|ui| {
                    ui.label(RichText::new(tr("Keep")).color(p.weak));
                    ui.add_space(30.0);
                    theme::segmented(
                        ui,
                        p,
                        &mut self.options.mode,
                        &[(Mode::Video, tr("Video")), (Mode::Audio, tr("Sound only"))],
                    );
                });
                ui.add_space(4.0);
                let o = &mut self.options;
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new(tr("Quality")).color(p.weak));
                    ui.add_space(16.0);
                    if o.mode == Mode::Video {
                        if o.max_height.is_some_and(|h| !heights.contains(&h)) {
                            o.max_height = None;
                        }
                        let sizes = match &self.stage {
                            Stage::Media(info) => Some(info),
                            _ => None,
                        };
                        let mut chip = |ui: &mut Ui, h: Option<u32>| {
                            let size = sizes.and_then(|i| estimate(i, h)).map(|s| format!("  ~{}", format_size(s)));
                            let label = format!("{}{}", settings::height_label(h), size.unwrap_or_default());
                            if ui.selectable_label(o.max_height == h, label).clicked() {
                                o.max_height = h;
                            }
                        };
                        chip(ui, None);
                        for h in &heights {
                            chip(ui, Some(*h));
                        }
                    } else if o.audio_format.lossy() {
                        for k in settings::KBPS {
                            if ui.selectable_label(o.audio_kbps == k, settings::kbps_label(k)).clicked() {
                                o.audio_kbps = k;
                            }
                        }
                    } else {
                        ui.label(RichText::new(tr("Lossless")).color(p.weak));
                    }
                });
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new(tr("Format")).color(p.weak));
                    ui.add_space(18.0);
                    if o.mode == Mode::Video {
                        for c in Container::ALL {
                            if ui.selectable_label(o.container == c, c.name().to_uppercase()).clicked() {
                                o.container = c;
                            }
                        }
                    } else {
                        for f in AudioFormat::ALL {
                            if ui.selectable_label(o.audio_format == f, f.name().to_uppercase()).clicked() {
                                o.audio_format = f;
                            }
                        }
                    }
                });
                egui::CollapsingHeader::new(tr("More choices")).id_salt("more").show(ui, |ui| {
                    if o.mode == Mode::Video {
                        ui.horizontal(|ui| {
                            let mut on = !o.subtitles.is_empty();
                            if ui.checkbox(&mut on, tr("Subtitles")).changed() {
                                o.subtitles = if on { settings::default_subtitles() } else { String::new() };
                            }
                            if on {
                                ui.add(egui::TextEdit::singleline(&mut o.subtitles).desired_width(120.0))
                                    .on_hover_text(tr("Languages, e.g. en,ar — or all"));
                            }
                        });
                    }
                    ui.checkbox(&mut o.sponsorblock, tr("Cut out sponsor segments (SponsorBlock)"));
                    ui.horizontal(|ui| {
                        let mut on = self.part.is_some();
                        if ui.checkbox(&mut on, tr("Only a part, from")).changed() {
                            self.part = on.then(|| ("0:00".to_string(), "1:00".to_string()));
                        }
                        if let Some((from, to)) = &mut self.part {
                            ui.add(egui::TextEdit::singleline(from).desired_width(60.0));
                            ui.label(tr("to"));
                            ui.add(egui::TextEdit::singleline(to).desired_width(60.0));
                            let valid = matches!((parse_time(from), parse_time(to)), (Some(a), Some(b)) if b > a);
                            if !valid {
                                ui.label(RichText::new(icon::WARNING).color(p.warning))
                                    .on_hover_text(tr("Like 1:30 to 2:45"));
                            }
                        }
                    });
                });
                if !ffmpeg {
                    ui.add_space(6.0);
                    let msg = if self.options.mode == Mode::Audio {
                        trl("FFmpeg is not installed: the sound is saved as it is (M4A or WebM), not as MP3.")
                    } else {
                        trl("FFmpeg is not installed: only qualities with sound in one file can be saved (often 360p).")
                    };
                    theme::notice(ui, p, p.warning, icon::INFO, msg);
                    if ui.link(tr("Install FFmpeg")).clicked() {
                        action = Action::InstallHelpers;
                    }
                }
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new(tr("Save to")).color(p.weak));
                ui.add_space(14.0);
                let shown = self.dir.display().to_string();
                ui.add(egui::Label::new(RichText::new(shown)).truncate());
                if ui.button(icon::FOLDER_OPEN).on_hover_text(tr("Choose a folder")).clicked()
                    && let Some(d) = rfd::FileDialog::new().set_directory(&self.dir).pick_folder()
                {
                    self.dir = d;
                }
            });
            if let Stage::Playlist { .. } = self.stage {
                ui.checkbox(&mut self.own_folder, tr("In a folder named after the playlist"));
            }
            ui.add_space(14.0);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let count = self.downloads().len();
                // "Only a part" with times that make no sense: not silently the whole video.
                let part_ok = self.part.is_none() || self.options().section.is_some();
                let label = if count > 1 {
                    format!("  {}  {}  ", icon::DOWNLOAD_SIMPLE, trf("Download {n}", &[("n", &count)]))
                } else {
                    format!("  {}  ", icon_label(icon::DOWNLOAD_SIMPLE, "Download"))
                };
                if theme::primary_button(ui, p, &label, count > 0 && part_ok).clicked() {
                    action = Action::Start(self.downloads());
                }
                if theme::secondary_button(ui, tr("Cancel")).clicked() {
                    action = Action::Close;
                }
            });
        });
        if modal.should_close() && matches!(action, Action::None) {
            if let Stage::Looking(job) = &self.stage {
                job.stop();
            }
            action = Action::Close;
        }
        action
    }

    /// Titles shown in the dialog (to load fonts for their script).
    pub fn texts(&self) -> Vec<&str> {
        match &self.stage {
            Stage::Media(info) => vec![info.title.as_str(), info.uploader.as_str()],
            Stage::Playlist { title, entries, .. } => {
                std::iter::once(title.as_str()).chain(entries.iter().map(|e| e.title.as_str())).collect()
            }
            Stage::File(_) => vec![self.file_name.as_str()],
            _ => Vec::new(),
        }
    }

    /// Looking failed (to look again once helpers are installed).
    pub fn failed(&self) -> bool {
        matches!(self.stage, Stage::Failed(_))
    }
}

/// Estimated size for quality `h` (best: the largest).
fn estimate(info: &Info, h: Option<u32>) -> Option<u64> {
    let i = match h {
        None => 0,
        Some(h) => info.heights.iter().position(|x| *x <= h)?,
    };
    info.sizes.get(i).copied().flatten()
}

fn media_card(ui: &mut Ui, p: &Palette, info: &Info, thumbs: &mut Thumbs) {
    ui.horizontal(|ui| {
        let size = Vec2::new(176.0, 99.0);
        let tex = info.thumbnail.as_deref().and_then(|u| thumbs.get(ui.ctx(), u));
        match tex {
            Some(t) => {
                ui.add(egui::Image::new(&t).fit_to_exact_size(size).corner_radius(CornerRadius::same(8)));
            }
            None => {
                let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
                ui.painter().rect_filled(rect, CornerRadius::same(8), p.card_alt);
                ui.painter().text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    icon::FILM_STRIP,
                    egui::FontId::proportional(30.0),
                    p.weak,
                );
            }
        }
        ui.vertical(|ui| {
            ui.add(egui::Label::new(text::rich(&info.title).size(16.0).strong()).wrap());
            let mut meta = Vec::new();
            if !info.uploader.is_empty() {
                meta.push(info.uploader.clone());
            }
            if let Some(d) = info.duration {
                meta.push(format_eta(d as u64));
            }
            if !info.site.is_empty() {
                meta.push(info.site.clone());
            }
            ui.label(text::rich(&meta.join("  ·  ")).color(p.weak).size(13.0));
            if info.is_live {
                theme::pill(ui, p, tr("Live — recorded until you stop it"), p.danger);
            }
        });
    });
}

fn playlist_card(ui: &mut Ui, p: &Palette, title: &str, entries: &[media::Entry], selected: &mut [bool]) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(icon::PLAYLIST).size(26.0).color(p.accent));
        ui.vertical(|ui| {
            ui.add(
                egui::Label::new(
                    text::rich(if title.is_empty() { trl("Playlist") } else { title }).size(16.0).strong(),
                )
                .wrap(),
            );
            let n = selected.iter().filter(|s| **s).count();
            ui.label(
                RichText::new(trf("{n} of {total} selected", &[("n", &n), ("total", &entries.len())]))
                    .color(p.weak)
                    .size(13.0),
            );
        });
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui.small_button(tr("None")).clicked() {
                selected.iter_mut().for_each(|s| *s = false);
            }
            if ui.small_button(tr("All")).clicked() {
                selected.iter_mut().for_each(|s| *s = true);
            }
        });
    });
    ui.add_space(6.0);
    egui::Frame::new().fill(p.card_alt).corner_radius(CornerRadius::same(10)).inner_margin(8).show(ui, |ui| {
        egui::ScrollArea::vertical().max_height(240.0).auto_shrink([false, true]).show_rows(
            ui,
            24.0,
            entries.len(),
            |ui, range| {
                for i in range {
                    ui.horizontal(|ui| {
                        ui.checkbox(&mut selected[i], "");
                        ui.label(RichText::new(format!("{}.", i + 1)).color(p.weak));
                        if let Some(d) = entries[i].duration {
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                ui.label(RichText::new(format_eta(d as u64)).color(p.weak).size(12.5));
                                ui.add(egui::Label::new(text::rich(&entries[i].title)).truncate());
                            });
                        } else {
                            ui.add(egui::Label::new(text::rich(&entries[i].title)).truncate());
                        }
                    });
                }
            },
        );
    });
}

fn file_card(ui: &mut Ui, p: &Palette, probe: &Probe, name: &mut String) {
    ui.horizontal(|ui| {
        let kind = FileKind::of(name);
        ui.label(RichText::new(kind_icon(kind)).size(34.0).color(p.accent));
        ui.vertical(|ui| {
            ui.add(egui::TextEdit::singleline(name).desired_width(440.0).hint_text(tr("File name")));
            let size = probe.size.map(format_size).unwrap_or_else(|| tr("Size unknown").into());
            let how = if probe.resumable {
                tr("Fast: several connections, can be paused")
            } else {
                tr("One connection, cannot be paused")
            };
            ui.horizontal(|ui| {
                ui.label(RichText::new(size).color(p.weak));
                ui.label(RichText::new("·").color(p.weak));
                let color = if probe.resumable { p.success } else { p.weak };
                ui.label(RichText::new(how).color(color).size(13.0));
            });
        });
    });
    if probe.is_web_page() {
        ui.add_space(6.0);
        theme::notice(
            ui,
            p,
            p.warning,
            icon::INFO,
            trl("This link opens a web page, not a file. No video was found on it either."),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times() {
        assert_eq!(parse_time("1:30"), Some(90));
        assert_eq!(parse_time("90"), Some(90));
        assert_eq!(parse_time("1:00:05"), Some(3605));
        assert_eq!(parse_time("x"), None);
        assert_eq!(parse_time(""), None);
    }
}
