//! The window: sidebar, pages, dialogs and notifications.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use eframe::egui::{
    self, Align, Color32, CornerRadius, Frame, Key, Layout, Margin, RichText, Sense, Stroke, TextureHandle, Ui, Vec2,
};
use egui_phosphor::regular as icon;
use swiftdm::manager::{Item, Manager, Status};
use swiftdm::tools::Tool;
use swiftdm::util::{FileKind, is_link};

use crate::add::{self, AddDialog};
use crate::helpers::Helpers;
use crate::i18n::{self, icon_label, tr, trl};
use crate::list::{self, RowAction};
use crate::settings::{self, Settings};
use crate::text;
use crate::theme::{self, Accent, Palette, ThemeChoice};
use crate::thumbs::Thumbs;
use crate::update::{self, Updater};

pub const DEFAULT_SIZE: [f32; 2] = [1180.0, 760.0];
pub const MIN_SIZE: [f32; 2] = [820.0, 560.0];
const SETTINGS_KEY: &str = "settings";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    Downloads,
    Settings,
    About,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Filter {
    All,
    Active,
    Done,
    Paused,
    Failed,
    Kind(FileKind),
}

impl Filter {
    fn matches(self, item: &Item) -> bool {
        match self {
            Filter::All => true,
            Filter::Active => matches!(item.status, Status::Active | Status::Queued),
            Filter::Done => item.status == Status::Done,
            Filter::Paused => item.status == Status::Paused,
            Filter::Failed => item.status == Status::Failed,
            Filter::Kind(k) => item.kind() == k,
        }
    }

    fn title(self) -> &'static str {
        match self {
            Filter::All => tr("All downloads"),
            Filter::Active => tr("Downloading"),
            Filter::Done => tr("Completed"),
            Filter::Paused => tr("Paused"),
            Filter::Failed => tr("Failed"),
            Filter::Kind(k) => list::kind_name(k),
        }
    }
}

struct Toast {
    /// Ready to display (`tr`).
    text: String,
    /// In logical order: titles, file names, messages.
    detail: String,
    glyph: &'static str,
    color: Color32,
    /// A finished download to open, or a copied link to download.
    target: ToastTarget,
    until: Instant,
}

enum ToastTarget {
    None,
    Item(u64),
    Link(String),
}

pub struct App {
    settings: Settings,
    applied: Option<(Accent, f32, ThemeChoice)>,
    /// The interface language the fonts were set up for.
    language: i18n::Lang,
    applied_manager: Option<swiftdm::manager::Settings>,
    manager: Manager,
    page: Page,
    filter: Filter,
    search: String,
    url: String,
    /// Links being added; the first one not in quick mode is shown.
    adds: Vec<AddDialog>,
    /// Look at these links again once the helpers are installed.
    waiting_for_helpers: bool,
    helpers: Helpers,
    updater: Updater,
    thumbs: Thumbs,
    logo: TextureHandle,
    clipboard: Option<arboard::Clipboard>,
    clip_last: String,
    clip_checked: Instant,
    toasts: Vec<Toast>,
    /// Last status seen of each item, to notice when one finishes.
    seen: HashMap<u64, Status>,
    confirm_delete: Option<u64>,
    /// The system's CJK font is loaded (once a title needs it).
    cjk_loaded: bool,
    cjk_checked: Instant,
    focus_url: bool,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        #[allow(unused_mut)]
        let mut settings: Settings = cc.storage.and_then(|s| eframe::get_value(s, SETTINGS_KEY)).unwrap_or_default();
        // Development aid: screenshots in each language ("ar", "de", "tr").
        #[cfg(debug_assertions)]
        if let Ok(l) = std::env::var("SWIFTDM_LANG") {
            settings.language = match l.as_str() {
                "ar" => i18n::Language::Arabic,
                "de" => i18n::Language::German,
                "tr" => i18n::Language::Turkish,
                _ => i18n::Language::English,
            };
        }
        let ctx = &cc.egui_ctx;
        let lang = i18n::set_language(settings.language);
        theme::install_fonts(ctx, lang, false);
        ctx.options_mut(|o| o.zoom_with_keyboard = false);
        let logo = {
            let img = image::load_from_memory(include_bytes!("../assets/icon.png")).map(|i| i.to_rgba8());
            let color = match img {
                Ok(i) => {
                    egui::ColorImage::from_rgba_unmultiplied([i.width() as usize, i.height() as usize], i.as_raw())
                }
                Err(_) => egui::ColorImage::filled([1, 1], Color32::TRANSPARENT),
            };
            ctx.load_texture("logo", color, egui::TextureOptions::LINEAR)
        };
        let manager = Manager::open(Some(swiftdm::paths::data_dir().join("downloads.json")));
        manager.set_settings(settings.manager());
        {
            let ctx = ctx.clone();
            manager.set_notify(move || ctx.request_repaint());
        }
        let seen = manager.items().iter().map(|i| (i.id, i.status)).collect();
        let mut clipboard = arboard::Clipboard::new().ok();
        // Links copied before the app started are not offered.
        let clip_last = clipboard.as_mut().and_then(|c| c.get_text().ok()).unwrap_or_default().trim().to_string();
        let mut updater = Updater::default();
        if settings.check_updates {
            updater.check(ctx, false);
        }
        let mut app = Self {
            helpers: Helpers::new(ctx, settings.keep_ytdlp_updated),
            settings,
            applied: None,
            language: lang,
            applied_manager: None,
            manager,
            page: Page::Downloads,
            filter: Filter::All,
            search: String::new(),
            url: String::new(),
            adds: Vec::new(),
            waiting_for_helpers: false,
            updater,
            thumbs: Thumbs::default(),
            logo,
            clipboard,
            clip_last,
            clip_checked: Instant::now(),
            toasts: Vec::new(),
            seen,
            confirm_delete: None,
            cjk_loaded: false,
            cjk_checked: Instant::now(),
            focus_url: true,
        };
        // Development aid: open a page for screenshots.
        #[cfg(debug_assertions)]
        match std::env::var("SWIFTDM_PAGE").as_deref() {
            Ok("settings") => app.page = Page::Settings,
            Ok("about") => app.page = Page::About,
            _ => {}
        }
        // Links given when starting ("Open with", scripts, browser add-ons).
        let links: Vec<String> = std::env::args().skip(1).filter(|a| is_link(a)).collect();
        if !links.is_empty() {
            app.add_links(ctx, &links.join(" "));
        }
        app
    }

    fn apply_settings(&mut self, ctx: &egui::Context) {
        // Ctrl/Cmd with +, - and 0 change the interface size.
        use egui::gui_zoom::kb_shortcuts as keys;
        let pressed = |k| ctx.input_mut(|i| i.consume_shortcut(&k));
        let scale = &mut self.settings.ui_scale;
        if pressed(keys::ZOOM_RESET) {
            *scale = 1.0;
        } else if pressed(keys::ZOOM_IN) || pressed(keys::ZOOM_IN_SECONDARY) {
            *scale = ((*scale * 10.0).round() / 10.0 + 0.1).min(1.5);
        } else if pressed(keys::ZOOM_OUT) {
            *scale = ((*scale * 10.0).round() / 10.0 - 0.1).max(0.8);
        }
        let want = (self.settings.accent, self.settings.ui_scale, self.settings.theme);
        if self.applied != Some(want) {
            theme::apply_style(ctx, self.settings.accent, self.settings.theme == ThemeChoice::Night);
            ctx.set_theme(self.settings.theme.preference());
            ctx.set_zoom_factor(self.settings.ui_scale);
            self.applied = Some(want);
        }
        let lang = i18n::set_language(self.settings.language);
        if lang != self.language {
            // Arabic needs its font first (see `i18n`).
            theme::install_fonts(ctx, lang, self.cjk_loaded);
            self.language = lang;
        }
        let m = self.settings.manager();
        if self.applied_manager.as_ref() != Some(&m) {
            self.manager.set_settings(m.clone());
            self.applied_manager = Some(m);
        }
    }

    /// Opens the dialog for `text` (one or more links).
    fn add_links(&mut self, ctx: &egui::Context, text: &str) {
        let links: Vec<&str> = text.split_whitespace().filter(|l| is_link(l)).collect();
        if links.is_empty() {
            self.toast(
                icon::WARNING,
                Color32::from_rgb(245, 158, 11),
                tr("That is not a link"),
                text.trim(),
                ToastTarget::None,
            );
            return;
        }
        // Several links at once start without questions.
        let quick = !self.settings.ask_first || links.len() > 1;
        for l in links {
            self.adds.push(AddDialog::new(ctx, l, &self.settings, quick));
        }
        self.url.clear();
        self.page = Page::Downloads;
    }

    fn start(&mut self, downloads: Vec<add::NewDownload>) {
        let n = downloads.len();
        for d in downloads {
            self.manager.add(&d.url, d.source, d.name, &d.dir, d.thumbnail);
        }
        if n > 0 && matches!(self.filter, Filter::Done | Filter::Failed | Filter::Paused) {
            self.filter = Filter::All;
        }
    }

    fn poll_adds(&mut self, ctx: &egui::Context, p: &Palette) {
        // Quick ones start on their own.
        let mut start = Vec::new();
        for d in &mut self.adds {
            if let add::Action::Start(list) = d.poll() {
                start.extend(list);
                d.url.clear();
            }
        }
        self.adds.retain(|d| !d.url.is_empty());
        self.start(start);
        if self.waiting_for_helpers && !self.helpers.busy() {
            self.waiting_for_helpers = false;
            self.helpers.refresh(ctx);
            let settings = self.settings.clone();
            for d in self.adds.iter_mut().filter(|d| d.failed()) {
                d.start(ctx, &settings);
            }
        }
        let ffmpeg = self.helpers.installed(Tool::Ffmpeg);
        let Some(i) = self.adds.iter().position(|d| !d.quick) else { return };
        match self.adds[i].show(ctx, p, &mut self.thumbs, ffmpeg) {
            add::Action::None => {}
            add::Action::Close => {
                self.adds.remove(i);
            }
            add::Action::Start(list) => {
                self.adds.remove(i);
                self.start(list);
            }
            add::Action::Retry => {
                let settings = self.settings.clone();
                self.adds[i].start(ctx, &settings);
            }
            add::Action::InstallHelpers => {
                self.helpers.install_missing(ctx);
                self.waiting_for_helpers = true;
            }
        }
    }

    fn toast(&mut self, glyph: &'static str, color: Color32, text: &str, detail: &str, target: ToastTarget) {
        self.toasts.retain(|t| t.text != text || t.detail != detail);
        self.toasts.push(Toast {
            text: text.into(),
            detail: detail.into(),
            glyph,
            color,
            target,
            until: Instant::now() + Duration::from_secs(12),
        });
        if self.toasts.len() > 4 {
            self.toasts.remove(0);
        }
    }

    /// Notices downloads that finished or failed since the last frame.
    fn watch_items(&mut self, ctx: &egui::Context, items: &[Item], p: &Palette) {
        let mut news = Vec::new();
        for item in items {
            let before = self.seen.insert(item.id, item.status);
            if before == Some(Status::Active) && item.status != Status::Active {
                news.push((item.id, item.status, item.display_name().to_string(), item.error.clone()));
            }
        }
        for (id, status, name, error) in news {
            match status {
                Status::Done => {
                    self.toast(icon::CHECK_CIRCLE, p.success, tr("Download complete"), &name, ToastTarget::Item(id))
                }
                Status::Failed => {
                    let detail = format!("{name}\n{}", i18n::engine(&error.unwrap_or_default()));
                    self.toast(icon::WARNING_CIRCLE, p.danger, tr("Download failed"), &detail, ToastTarget::None)
                }
                _ => {}
            }
        }
        if !self.cjk_loaded && self.cjk_checked.elapsed() > Duration::from_secs(1) {
            self.cjk_checked = Instant::now();
            if items.iter().any(|i| text::has_cjk(&i.name)) {
                theme::install_fonts(ctx, self.language, true);
                self.cjk_loaded = true;
            }
        }
    }

    /// Offers links copied in other programs.
    fn watch_clipboard(&mut self, ctx: &egui::Context, p: &Palette) {
        if !self.settings.watch_clipboard || self.clip_checked.elapsed() < Duration::from_millis(900) {
            return;
        }
        self.clip_checked = Instant::now();
        let Some(cb) = &mut self.clipboard else { return };
        let Ok(text) = cb.get_text() else { return };
        let text = text.trim().to_string();
        if text == self.clip_last {
            return;
        }
        self.clip_last = text.clone();
        let known = self.manager.items().iter().any(|i| i.url == text) || self.adds.iter().any(|d| d.url == text);
        if is_link(&text) && !known {
            let host = swiftdm::util::host(&text).to_string();
            self.toast(icon::CLIPBOARD_TEXT, p.accent, tr("Download the copied link?"), &host, ToastTarget::Link(text));
            if !ctx.input(|i| i.focused) {
                ctx.send_viewport_cmd(egui::ViewportCommand::RequestUserAttention(
                    egui::UserAttentionType::Informational,
                ));
            }
        }
    }

    fn row_action(&mut self, ctx: &egui::Context, a: RowAction) {
        let item = |id| self.manager.item(id);
        match a {
            RowAction::Pause(id) => self.manager.pause(id),
            RowAction::Resume(id) => self.manager.resume(id),
            RowAction::Restart(id) => {
                let m = self.manager.clone();
                std::thread::spawn(move || m.restart(id));
            }
            RowAction::Open(id) => {
                if let Some(path) = item(id).and_then(|i| i.path) {
                    if path.exists() {
                        let _ = swiftdm::util::open_path(&path);
                    } else {
                        self.toast(
                            icon::WARNING,
                            Color32::from_rgb(245, 158, 11),
                            tr("The file is not there any more"),
                            &path.display().to_string(),
                            ToastTarget::None,
                        );
                    }
                }
            }
            RowAction::Reveal(id) => {
                if let Some(i) = item(id) {
                    match &i.path {
                        Some(p) => {
                            let _ = swiftdm::util::reveal_path(p);
                        }
                        None => {
                            let _ = swiftdm::util::open_path(&i.dir);
                        }
                    }
                }
            }
            RowAction::CopyLink(id) => {
                if let Some(i) = item(id) {
                    ctx.copy_text(i.url.clone());
                    // Not offered back as a "copied link".
                    self.clip_last = i.url;
                }
            }
            RowAction::OpenLink(id) => {
                if let Some(i) = item(id) {
                    ctx.open_url(egui::OpenUrl::new_tab(&i.url));
                }
            }
            RowAction::Remove(id) => self.manager.remove(id, false),
            RowAction::Delete(id) => self.confirm_delete = Some(id),
        }
    }

    // -----------------------------------------------------------------------
    // Layout

    fn sidebar(&mut self, ui: &mut Ui, p: &Palette, items: &[Item]) {
        ui.add_space(18.0);
        ui.horizontal(|ui| {
            ui.add_space(6.0);
            ui.add(egui::Image::new(&self.logo).fit_to_exact_size(Vec2::splat(40.0)));
            ui.vertical(|ui| {
                ui.add_space(2.0);
                ui.label(theme::semibold("SwiftDM", 18.0).color(p.text));
                ui.label(RichText::new(tr("Download manager")).color(p.weak).size(12.5));
            });
        });
        // Settings and friends stay at the bottom; the filters scroll above.
        egui::Panel::bottom("sidebar-bottom").resizable(false).show_separator_line(false).frame(Frame::NONE).show(
            ui,
            |ui| {
                ui.with_layout(Layout::bottom_up(Align::Min), |ui| {
                    ui.add_space(14.0);
                    let version = i18n::trf("Version {version}", &[("version", &update::current())]);
                    ui.label(RichText::new(version).color(p.weak).size(12.0));
                    ui.add_space(4.0);
                    if nav_item(ui, p, icon::INFO, tr("About"), self.page == Page::About, None).clicked() {
                        self.page = Page::About;
                    }
                    let badge = self.updater.available().map(|_| "●");
                    if nav_item(ui, p, icon::GEAR_SIX, tr("Settings"), self.page == Page::Settings, badge).clicked() {
                        self.page = Page::Settings;
                    }
                    if nav_item(ui, p, icon::FOLDER_OPEN, tr("Download folder"), false, None).clicked() {
                        let _ = std::fs::create_dir_all(&self.settings.download_dir);
                        let _ = swiftdm::util::open_path(&self.settings.download_dir);
                    }
                });
            },
        );
        ui.add_space(14.0);
        let count = |f: Filter| items.iter().filter(|i| f.matches(i)).count();
        let on_list = self.page == Page::Downloads;
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            group_label(ui, p, tr("DOWNLOADS"));
            for (f, glyph) in [
                (Filter::All, icon::LIST),
                (Filter::Active, icon::DOWNLOAD_SIMPLE),
                (Filter::Done, icon::CHECK_CIRCLE),
                (Filter::Paused, icon::PAUSE_CIRCLE),
                (Filter::Failed, icon::WARNING_CIRCLE),
            ] {
                let n = count(f);
                let badge = (n > 0).then(|| n.to_string());
                if nav_item(ui, p, glyph, f.title(), on_list && self.filter == f, badge.as_deref()).clicked() {
                    self.filter = f;
                    self.page = Page::Downloads;
                }
            }
            ui.add_space(8.0);
            group_label(ui, p, tr("TYPE"));
            for k in [
                FileKind::Video,
                FileKind::Audio,
                FileKind::Image,
                FileKind::Archive,
                FileKind::Program,
                FileKind::Document,
            ] {
                let f = Filter::Kind(k);
                let n = count(f);
                let badge = (n > 0).then(|| n.to_string());
                if nav_item(
                    ui,
                    p,
                    list::kind_icon(k),
                    list::kind_name(k),
                    on_list && self.filter == f,
                    badge.as_deref(),
                )
                .clicked()
                {
                    self.filter = f;
                    self.page = Page::Downloads;
                }
            }
        });
    }

    fn link_bar(&mut self, ui: &mut Ui, p: &Palette) {
        let ctx = ui.ctx().clone();
        Frame::new()
            .fill(p.card)
            .stroke(Stroke::new(1.5, p.accent.gamma_multiply(0.55)))
            .corner_radius(CornerRadius::same(14))
            .inner_margin(Margin { left: 14, right: 8, top: 8, bottom: 8 })
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(icon::LINK).size(20.0).color(p.accent));
                    let mut submit = false;
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let busy = self.adds.iter().any(|d| d.quick && d.looking());
                        let label = format!("  {}  ", icon_label(icon::DOWNLOAD_SIMPLE, "Download"));
                        if theme::primary_button(ui, p, &label, !self.url.trim().is_empty()).clicked() {
                            submit = true;
                        }
                        if theme::icon_button(ui, p, icon::CLIPBOARD_TEXT, tr("Paste"), Some(p.accent)).clicked()
                            && let Some(t) = self.clipboard.as_mut().and_then(|c| c.get_text().ok())
                        {
                            self.clip_last = t.trim().to_string();
                            self.add_links(&ctx, &t);
                        }
                        if busy {
                            ui.spinner();
                        }
                        let edit = egui::TextEdit::singleline(&mut self.url)
                            .hint_text(tr("Paste a link — YouTube, TikTok, Instagram, X, any file…"))
                            .frame(egui::Frame::NONE)
                            .font(egui::FontId::proportional(16.0))
                            .desired_width(f32::INFINITY);
                        let resp = ui.add(edit);
                        if self.focus_url {
                            resp.request_focus();
                            self.focus_url = false;
                        }
                        if resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                            submit = true;
                        }
                    });
                    if submit && !self.url.trim().is_empty() {
                        let url = self.url.clone();
                        self.add_links(&ctx, &url);
                    }
                });
            });
    }

    fn helper_banner(&mut self, ui: &mut Ui, p: &Palette) {
        if !self.helpers.known() {
            return;
        }
        let ctx = ui.ctx().clone();
        let missing_ytdlp = !self.helpers.installed(Tool::YtDlp);
        let missing_other = !self.helpers.installed(Tool::Ffmpeg) || !self.helpers.has_js();
        if self.helpers.busy() || self.helpers.error.is_some() {
            theme::card(ui, p, |ui| {
                ui.set_width(ui.available_width());
                self.helpers.progress_ui(ui, p);
            });
            ui.add_space(12.0);
            return;
        }
        if missing_ytdlp || (missing_other && !self.settings.welcomed) {
            theme::card(ui, p, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    let (rect, _) = ui.allocate_exact_size(Vec2::splat(44.0), Sense::hover());
                    ui.painter().rect_filled(rect, CornerRadius::same(12), p.tint(p.accent));
                    ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, icon::SPARKLE, egui::FontId::proportional(24.0), p.accent);
                    ui.vertical(|ui| {
                        let title = if missing_ytdlp { tr("One step to download from video sites") } else { tr("Finish setting up video sites") };
                        ui.label(theme::semibold(title, 16.0));
                        theme::paragraph(ui, trl("SwiftDM uses free helpers (yt-dlp, FFmpeg, Deno) for YouTube, TikTok, Instagram and 1800+ more sites. They are downloaded once from their official sources (about 120 MB). Plain file links work without them."), 14.5, p.weak);
                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            if theme::primary_button(ui, p, &format!("  {}  ", icon_label(icon::DOWNLOAD_SIMPLE, "Install helpers")), true).clicked() {
                                self.helpers.install_missing(&ctx);
                                self.settings.welcomed = true;
                            }
                            if !missing_ytdlp && theme::secondary_button(ui, tr("Not now")).clicked() {
                                self.settings.welcomed = true;
                            }
                        });
                    });
                });
            });
            ui.add_space(12.0);
        }
    }

    fn downloads_page(&mut self, ui: &mut Ui, p: &Palette, items: &[Item]) {
        let ctx = ui.ctx().clone();
        self.link_bar(ui, p);
        ui.add_space(14.0);
        self.helper_banner(ui, p);
        list::stats(ui, p, items, self.manager.speed());
        ui.add_space(16.0);

        let shown: Vec<&Item> = items
            .iter()
            .filter(|i| self.filter.matches(i))
            .filter(|i| {
                let q = self.search.trim().to_lowercase();
                q.is_empty() || i.display_name().to_lowercase().contains(&q) || i.url.to_lowercase().contains(&q)
            })
            .collect();
        ui.horizontal(|ui| {
            ui.label(theme::semibold(self.filter.title(), 18.0));
            ui.label(RichText::new(shown.len().to_string()).color(p.weak));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let done = items.iter().filter(|i| i.status == Status::Done).count();
                if done > 0
                    && ui
                        .button(icon_label(icon::BROOM, "Clear completed"))
                        .on_hover_text(tr("Remove finished downloads from the list (the files stay)"))
                        .clicked()
                {
                    self.manager.clear_finished();
                }
                if items.iter().any(|i| i.status == Status::Paused)
                    && ui.button(icon_label(icon::PLAY, "Continue all")).clicked()
                {
                    self.manager.resume_all();
                }
                if items.iter().any(|i| matches!(i.status, Status::Active | Status::Queued))
                    && ui.button(icon_label(icon::PAUSE, "Pause all")).clicked()
                {
                    self.manager.pause_all();
                }
                ui.add(
                    egui::TextEdit::singleline(&mut self.search)
                        .hint_text(icon_label(icon::MAGNIFYING_GLASS, "Search"))
                        .desired_width(200.0),
                );
            });
        });
        ui.add_space(8.0);

        if shown.is_empty() {
            empty_state(ui, p, items.is_empty(), &self.search);
            return;
        }
        let mut actions = Vec::new();
        let spacing = 8.0;
        egui::ScrollArea::vertical().auto_shrink([false, false]).show_rows(
            ui,
            list::ROW_HEIGHT + spacing,
            shown.len(),
            |ui, range| {
                ui.spacing_mut().item_spacing.y = spacing;
                for item in &shown[range] {
                    if let Some(a) = list::row(ui, p, item, &mut self.thumbs) {
                        actions.push(a);
                    }
                }
            },
        );
        for a in actions {
            self.row_action(&ctx, a);
        }
    }

    fn about_page(&mut self, ui: &mut Ui, p: &Palette) {
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                ui.add(egui::Image::new(&self.logo).fit_to_exact_size(Vec2::splat(84.0)));
                ui.add_space(8.0);
                ui.vertical(|ui| {
                    ui.label(theme::semibold("SwiftDM", 30.0));
                    ui.label(RichText::new(i18n::trf("Version {version}", &[("version", &update::current())])).color(p.weak));
                    ui.label(RichText::new("Powered by Bashar Salmo").color(p.accent));
                });
            });
            ui.add_space(14.0);
            theme::paragraph(ui, trl("Download videos, music and files from any link: YouTube, TikTok, Instagram, X, Facebook, SoundCloud, Vimeo and 1800+ more sites, or any direct file — fast, over several connections, and resumable."), 15.0, p.text);
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                ui.hyperlink_to(icon_label(icon::GITHUB_LOGO, "Downloads and news"), update::REPO);
                ui.label(RichText::new("·").color(p.weak));
                ui.hyperlink_to(icon_label(icon::BUG, "Report a problem"), format!("{}/issues", update::REPO));
            });
            ui.add_space(8.0);
            self.updater.status(ui, p);
            ui.add_space(18.0);

            theme::card(ui, p, |ui| {
                ui.set_width(ui.available_width());
                theme::section_title(ui, p, icon::TERMINAL_WINDOW, tr("Command line"));
                theme::paragraph(ui, trl("The installation also brings the swiftdm command, for scripts and the keyboard:"), 14.5, p.weak);
                ui.add_space(4.0);
                for line in [
                    "swiftdm https://youtu.be/…              # best video",
                    "swiftdm -a -k 320 https://…             # MP3, 320 kbps",
                    "swiftdm -q 720 -o ~/Videos https://…    # 720p, to a folder",
                    "swiftdm -c 16 https://example.com/x.iso # 16 connections",
                    "swiftdm info https://…                  # what is behind a link",
                ] {
                    ui.label(RichText::new(line).monospace().size(13.0));
                }
            });
            ui.add_space(12.0);
            theme::card(ui, p, |ui| {
                ui.set_width(ui.available_width());
                theme::section_title(ui, p, icon::HEART, tr("Made with"));
                for (name, what, url) in [
                    ("yt-dlp", tr("reads the video sites (Unlicense)"), "https://github.com/yt-dlp/yt-dlp"),
                    ("FFmpeg", tr("joins and converts video and sound (GPL, downloaded separately)"), "https://ffmpeg.org"),
                    ("Deno", tr("runs YouTube's JavaScript (MIT)"), "https://deno.com"),
                    ("egui", tr("the interface (MIT / Apache 2.0)"), "https://github.com/emilk/egui"),
                    ("Noto Sans Arabic", tr("Arabic titles (SIL Open Font License)"), "https://fonts.google.com/noto"),
                ] {
                    ui.horizontal(|ui| {
                        ui.hyperlink_to(name, url);
                        ui.label(RichText::new(what).color(p.weak));
                    });
                }
            });
            ui.add_space(12.0);
            theme::paragraph(ui, trl("Download only what you have the right to. Respect the terms of the sites you use."), 12.5, p.weak);
        });
    }

    fn toasts_ui(&mut self, ctx: &egui::Context, p: &Palette) {
        self.toasts.retain(|t| t.until > Instant::now());
        if self.toasts.is_empty() {
            return;
        }
        ctx.request_repaint_after(Duration::from_millis(500));
        let mut close = None;
        let mut hovered = None;
        let mut act: Option<(usize, bool)> = None;
        egui::Area::new(egui::Id::new("toasts"))
            .anchor(egui::Align2::RIGHT_BOTTOM, Vec2::new(-18.0, -18.0))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                ui.set_max_width(360.0);
                for (i, t) in self.toasts.iter().enumerate() {
                    let resp = Frame::new()
                        .fill(p.card)
                        .stroke(Stroke::new(1.0, t.color.gamma_multiply(0.6)))
                        .corner_radius(CornerRadius::same(12))
                        .shadow(ui.visuals().popup_shadow)
                        .inner_margin(Margin::same(12))
                        .show(ui, |ui| {
                            ui.set_width(336.0);
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(t.glyph).size(22.0).color(t.color));
                                ui.vertical(|ui| {
                                    ui.spacing_mut().item_spacing.y = 2.0;
                                    ui.horizontal(|ui| {
                                        ui.label(theme::semibold(&t.text, 14.5));
                                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                            if ui.small_button(icon::X).clicked() {
                                                close = Some(i);
                                            }
                                        });
                                    });
                                    for line in t.detail.lines().take(3) {
                                        ui.add(egui::Label::new(text::rich(line).color(p.weak).size(12.5)).truncate());
                                    }
                                    ui.horizontal(|ui| match &t.target {
                                        ToastTarget::Item(_) => {
                                            if ui.button(tr("Open")).clicked() {
                                                act = Some((i, true));
                                            }
                                            if ui.button(tr("Show in folder")).clicked() {
                                                act = Some((i, false));
                                            }
                                        }
                                        ToastTarget::Link(_) => {
                                            if theme::primary_button(
                                                ui,
                                                p,
                                                &format!("  {}  ", icon_label(icon::DOWNLOAD_SIMPLE, "Download")),
                                                true,
                                            )
                                            .clicked()
                                            {
                                                act = Some((i, true));
                                            }
                                        }
                                        ToastTarget::None => {}
                                    });
                                });
                            });
                        })
                        .response;
                    if resp.contains_pointer() {
                        hovered = Some(i);
                    }
                    ui.add_space(8.0);
                }
            });
        // Stays while the mouse is on it.
        if let Some(i) = hovered {
            self.toasts[i].until = self.toasts[i].until.max(Instant::now() + Duration::from_secs(3));
        }
        if let Some((i, primary)) = act {
            let t = self.toasts.remove(i);
            match t.target {
                ToastTarget::Item(id) if primary => self.row_action(ctx, RowAction::Open(id)),
                ToastTarget::Item(id) => self.row_action(ctx, RowAction::Reveal(id)),
                ToastTarget::Link(url) => {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                    self.add_links(ctx, &url);
                }
                ToastTarget::None => {}
            }
        } else if let Some(i) = close {
            self.toasts.remove(i);
        }
    }

    fn delete_dialog(&mut self, ctx: &egui::Context, p: &Palette) {
        let Some(id) = self.confirm_delete else { return };
        let Some(item) = self.manager.item(id) else {
            self.confirm_delete = None;
            return;
        };
        let modal = egui::Modal::new(egui::Id::new("delete")).show(ctx, |ui| {
            ui.set_width(420.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new(icon::TRASH).size(24.0).color(p.danger));
                ui.label(theme::semibold(tr("Delete this file?"), 17.0));
            });
            ui.add_space(6.0);
            ui.add(egui::Label::new(text::rich(item.display_name())).wrap());
            ui.label(RichText::new(tr("The file is deleted from the disk, not moved to the trash.")).color(p.weak));
            ui.add_space(12.0);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui
                    .add(
                        egui::Button::new(theme::semibold(tr("Delete"), 14.5).color(Color32::WHITE))
                            .fill(p.danger)
                            .min_size(Vec2::new(0.0, 36.0)),
                    )
                    .clicked()
                {
                    self.manager.remove(id, true);
                    self.confirm_delete = None;
                }
                if theme::secondary_button(ui, tr("Cancel")).clicked() {
                    self.confirm_delete = None;
                }
            });
        });
        if modal.should_close() {
            self.confirm_delete = None;
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.apply_settings(&ctx);
        self.helpers.poll(&ctx);
        self.thumbs.poll(&ctx);
        if self.updater.poll(&ctx) {
            // The installer takes over and starts the new version.
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        let p = Palette::new(ui.visuals().dark_mode, self.settings.theme == ThemeChoice::Night, self.settings.accent);
        let items = self.manager.items();
        self.watch_items(&ctx, &items, &p);
        self.watch_clipboard(&ctx, &p);

        // Ctrl/Cmd+V outside a text field adds the link.
        let nothing_focused = ctx.memory(|m| m.focused().is_none());
        if nothing_focused && self.adds.iter().all(|d| d.quick) {
            let pasted = ctx.input(|i| {
                i.events.iter().find_map(|e| if let egui::Event::Paste(s) = e { Some(s.clone()) } else { None })
            });
            if let Some(text) = pasted
                && is_link(text.trim())
            {
                self.clip_last = text.trim().to_string();
                self.add_links(&ctx, &text);
            }
        }

        egui::Panel::left("sidebar")
            .exact_size(232.0)
            .resizable(false)
            .frame(
                Frame::new().fill(p.sidebar).stroke(Stroke::new(1.0, p.border)).inner_margin(Margin::symmetric(12, 0)),
            )
            .show(ui, |ui| self.sidebar(ui, &p, &items));
        egui::CentralPanel::default()
            .frame(Frame::new().fill(p.bg).inner_margin(Margin { left: 26, right: 26, top: 22, bottom: 18 }))
            .show(ui, |ui| {
                if self.updater.available().is_some() {
                    ui.with_layout(Layout::right_to_left(Align::Min), |ui| self.updater.button(ui, &p));
                }
                match self.page {
                    Page::Downloads => self.downloads_page(ui, &p, &items),
                    Page::Settings => {
                        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                            ui.set_max_width(860.0);
                            settings::page(ui, &p, &mut self.settings, &mut self.helpers, &mut self.updater);
                        });
                    }
                    Page::About => self.about_page(ui, &p),
                }
            });
        self.poll_adds(&ctx, &p);
        self.delete_dialog(&ctx, &p);
        self.updater.dialog(&ctx, &p, self.manager.busy());
        self.toasts_ui(&ctx, &p);

        if self.manager.busy() || self.helpers.busy() || self.adds.iter().any(|d| d.looking()) {
            ctx.request_repaint_after(Duration::from_millis(250));
        }
        if self.settings.watch_clipboard {
            ctx.request_repaint_after(Duration::from_secs(1));
        }
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        // The screenshot language is not the user's.
        #[cfg(debug_assertions)]
        if std::env::var_os("SWIFTDM_LANG").is_some() {
            return;
        }
        eframe::set_value(storage, SETTINGS_KEY, &self.settings);
    }

    fn on_exit(&mut self) {
        // Running downloads continue at the next start.
        self.manager.shutdown();
    }
}

fn group_label(ui: &mut Ui, p: &Palette, text: &str) {
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.add_space(14.0);
        ui.label(RichText::new(text).size(11.0).color(p.weak).strong());
    });
}

fn nav_item(ui: &mut Ui, p: &Palette, glyph: &str, label: &str, active: bool, badge: Option<&str>) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 36.0), Sense::click());
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        if active {
            painter.rect_filled(rect, CornerRadius::same(10), p.tint(p.accent));
            painter.rect_filled(
                egui::Rect::from_min_size(rect.min + Vec2::new(0.0, 8.0), Vec2::new(3.0, rect.height() - 16.0)),
                CornerRadius::same(2),
                p.accent,
            );
        } else if resp.hovered() {
            painter.rect_filled(rect, CornerRadius::same(10), p.card_alt);
        }
        let color = if active { p.text } else { p.weak };
        let y = rect.center().y;
        painter.text(
            egui::pos2(rect.left() + 14.0, y),
            egui::Align2::LEFT_CENTER,
            glyph,
            egui::FontId::proportional(18.0),
            if active { p.accent } else { color },
        );
        painter.text(
            egui::pos2(rect.left() + 42.0, y),
            egui::Align2::LEFT_CENTER,
            label,
            egui::FontId::new(14.5, egui::FontFamily::Name(theme::SEMIBOLD.into())),
            color,
        );
        if let Some(b) = badge {
            let c = if b == "●" { p.accent } else { p.weak };
            painter.text(
                egui::pos2(rect.right() - 12.0, y),
                egui::Align2::RIGHT_CENTER,
                b,
                egui::FontId::proportional(12.5),
                c,
            );
        }
    }
    ui.add_space(1.0);
    resp
}

fn empty_state(ui: &mut Ui, p: &Palette, nothing_yet: bool, search: &str) {
    ui.add_space(40.0);
    ui.vertical_centered(|ui| {
        if !search.trim().is_empty() {
            ui.label(RichText::new(icon::MAGNIFYING_GLASS).size(40.0).color(p.weak));
            ui.label(RichText::new(tr("Nothing matches the search.")).color(p.weak).size(15.0));
            return;
        }
        if !nothing_yet {
            ui.label(RichText::new(icon::TRAY).size(40.0).color(p.weak));
            ui.label(RichText::new(tr("Nothing here.")).color(p.weak).size(15.0));
            return;
        }
        ui.label(RichText::new(icon::DOWNLOAD_SIMPLE).size(54.0).color(p.accent));
        ui.add_space(6.0);
        ui.label(theme::semibold(tr("Paste a link to start"), 20.0));
        ui.label(RichText::new(tr("Or copy one anywhere — SwiftDM offers to download it.")).color(p.weak));
        ui.add_space(16.0);
        ui.horizontal_wrapped(|ui| {
            let sites = [
                "YouTube",
                "TikTok",
                "Instagram",
                "X",
                "Facebook",
                "SoundCloud",
                "Vimeo",
                "Reddit",
                "Twitch",
                tr("Any file link"),
            ];
            let total: f32 = sites.len() as f32 * 96.0;
            ui.add_space(((ui.available_width() - total) / 2.0).max(0.0));
            for s in sites {
                theme::pill(ui, p, s, p.accent);
            }
        });
    });
}
