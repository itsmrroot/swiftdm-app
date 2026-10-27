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
use crate::help::HelpAction;
use crate::helpers::Helpers;
use crate::i18n::{self, icon_label, tr, trl};
use crate::instance::{Inbox, Incoming};
use crate::jobs::Job;
use crate::list::{self, RowAction};
use crate::menu::{Command, MenuBar};
use crate::power::{AfterDone, Due, Power};
use crate::settings::{self, Settings};
use crate::text;
use crate::theme::{self, Accent, Palette, ThemeChoice};
use crate::thumbs::Thumbs;
use crate::update::{self, Updater};

/// The first window: small enough that no system maximizes it on a small
/// screen; [`App::fit_to_screen`] makes it larger on large ones.
pub const DEFAULT_SIZE: [f32; 2] = [1000.0, 640.0];
const LARGE_SIZE: [f32; 2] = [1180.0, 760.0];
pub const MIN_SIZE: [f32; 2] = [820.0, 560.0];
const SETTINGS_KEY: &str = "settings";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    Downloads,
    Settings,
    Help,
    About,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Filter {
    All,
    Active,
    Done,
    Paused,
    Failed,
}

impl Filter {
    fn matches(self, item: &Item) -> bool {
        match self {
            Filter::All => true,
            Filter::Active => matches!(item.status, Status::Active | Status::Queued),
            Filter::Done => item.status == Status::Done,
            Filter::Paused => item.status == Status::Paused,
            Filter::Failed => item.status == Status::Failed,
        }
    }

    fn title(self) -> &'static str {
        match self {
            Filter::All => tr("All downloads"),
            Filter::Active => tr("Downloading"),
            Filter::Done => tr("Completed"),
            Filter::Paused => tr("Paused"),
            Filter::Failed => tr("Failed"),
        }
    }
}

/// What a [`Prompt`] is for.
#[derive(Clone, Copy, PartialEq, Eq)]
enum PromptKind {
    /// A new address for download `id`.
    ChangeLink(u64),
    /// A checksum to compare the file of download `id` with.
    Verify(u64),
}

struct Prompt {
    kind: PromptKind,
    text: String,
    /// The line has the focus (set once, when the dialog opens).
    focused: bool,
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
    applied: Option<(Accent, f32, ThemeChoice, bool)>,
    /// The interface language the fonts were set up for.
    language: i18n::Lang,
    applied_manager: Option<swiftdm::manager::Settings>,
    manager: Manager,
    page: Page,
    filter: Filter,
    /// Only this kind of file (besides the filter above).
    kind_filter: Option<FileKind>,
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
    /// A question with a line to type in ("Change link…", "Verify checksum…").
    prompt: Option<Prompt>,
    /// The system's CJK font is loaded (once a title needs it).
    cjk_loaded: bool,
    cjk_checked: Instant,
    focus_url: bool,
    /// Links from later starts of the app.
    inbox: Option<Inbox>,
    /// When this run began: items added since get notifications even if
    /// they finished between two frames.
    session_start: i64,
    /// The language choice the interface was last set to.
    applied_language: Option<i18n::Language>,
    /// Whether copied links were watched in the last frame.
    clip_watching: bool,
    menu: MenuBar,
    power: Power,
    settings_page: settings::Page,
    /// When the browser extension last talked to SwiftDM.
    browser_seen: Option<i64>,
    browser_checked: Instant,
    /// SHA-256 of finished files being computed (to copy).
    /// File name, the checksum it is to be compared with (none: copy it).
    hashing: Vec<(String, Option<String>, Job<String>)>,
    /// The window was fitted to the screen.
    fitted: bool,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, inbox: Inbox, links: Vec<String>) -> Self {
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
        let update_failed = update::take_failure();
        let mut app = Self {
            helpers: Helpers::new(ctx, settings.keep_ytdlp_updated),
            settings,
            applied: None,
            language: lang,
            applied_manager: None,
            manager,
            page: Page::Downloads,
            filter: Filter::All,
            kind_filter: None,
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
            prompt: None,
            cjk_loaded: false,
            cjk_checked: Instant::now(),
            focus_url: true,
            inbox: None,
            session_start: swiftdm::util::now(),
            applied_language: None,
            clip_watching: false,
            menu: MenuBar::default(),
            power: Power::default(),
            settings_page: settings::Page::default(),
            browser_seen: None,
            browser_checked: Instant::now() - Duration::from_secs(60),
            hashing: Vec::new(),
            fitted: false,
        };
        // Development aid: open a page for screenshots.
        #[cfg(debug_assertions)]
        match std::env::var("SWIFTDM_PAGE").as_deref() {
            Ok("settings") => app.page = Page::Settings,
            Ok("about") => app.page = Page::About,
            Ok("help") => app.page = Page::Help,
            Ok("browser") => {
                app.page = Page::Settings;
                app.settings_page.tab = settings::Tab::Browser;
            }
            Ok("downloads") => {
                app.page = Page::Settings;
                app.settings_page.tab = settings::Tab::Downloads;
            }
            _ => {}
        }
        // Development aid: open one of the small dialogs for the first download.
        #[cfg(debug_assertions)]
        if let Some(id) = app.manager.items().first().map(|i| i.id) {
            match std::env::var("SWIFTDM_PROMPT").as_deref() {
                Ok("change") => {
                    app.prompt =
                        Some(Prompt { kind: PromptKind::ChangeLink(id), text: "https://".into(), focused: false })
                }
                Ok("verify") => {
                    app.prompt = Some(Prompt { kind: PromptKind::Verify(id), text: String::new(), focused: false })
                }
                _ => {}
            }
        }
        inbox.attach(ctx);
        // Connects the browser extension (and keeps it connected after an
        // update or a move of the app).
        // Not for a separate test or portable copy (its own data folder):
        // the browsers would then talk to it instead of the installed app.
        let separate = std::env::var_os("SWIFTDM_DATA_DIR").is_some();
        std::thread::spawn(move || {
            if separate {
                return;
            }
            if let Err(e) = crate::browser::register() {
                log::warn!("browser helper not registered: {e:#}");
            }
            crate::browser::refresh_extension_files();
        });
        app.inbox = Some(inbox);
        if let Some(version) = update_failed {
            let detail = i18n::trlf(
                "The update to version {version} was not installed. Download it from the releases page and run it.",
                &[("version", &version)],
            );
            app.toast(
                icon::WARNING,
                Color32::from_rgb(220, 38, 38),
                tr("The update could not be installed"),
                &detail,
                ToastTarget::None,
            );
        }
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
        let want = (self.settings.accent, self.settings.ui_scale, self.settings.theme, self.settings.glass);
        theme::set_glass(self.settings.glass);
        // Not while the mouse is down: the slider itself changes size with
        // the zoom, which would make it jump under the pointer.
        let dragging = self.page == Page::Settings && ctx.input(|i| i.pointer.any_down());
        let only_scale = self.applied.is_some_and(|a| (a.0, a.2, a.3) == (want.0, want.2, want.3));
        if self.applied != Some(want) && !(dragging && only_scale) {
            theme::apply_style(ctx, self.settings.accent, self.settings.theme == ThemeChoice::Night);
            ctx.set_theme(self.settings.theme.preference());
            ctx.set_zoom_factor(self.settings.ui_scale);
            self.applied = Some(want);
        }
        // The system language is looked up only when the choice changes.
        if self.applied_language != Some(self.settings.language) {
            self.applied_language = Some(self.settings.language);
            i18n::set_language(self.settings.language);
        }
        let lang = i18n::current();
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
    fn add_links(&mut self, ctx: &egui::Context, text: &str) -> bool {
        let mut links: Vec<String> = Vec::new();
        let mut too_many = false;
        for word in text.split_whitespace().filter(|l| is_link(l)) {
            // "https://site/photo[01-30].jpg" is thirty links.
            match swiftdm::util::expand_pattern(word) {
                Ok(Some(many)) => links.extend(many),
                Ok(None) => links.push(word.to_string()),
                Err(count) => {
                    too_many = true;
                    let detail = i18n::trlf(
                        "That would be {n} links; at most {max} at once.",
                        &[("n", &count), ("max", &swiftdm::util::MAX_PATTERN_LINKS)],
                    );
                    let amber = Color32::from_rgb(245, 158, 11);
                    self.toast(icon::WARNING, amber, tr("Too many links in the pattern"), &detail, ToastTarget::None);
                }
            }
        }
        if links.len() > 1000 {
            links.truncate(1000);
            let amber = Color32::from_rgb(245, 158, 11);
            self.toast(icon::WARNING, amber, tr("Too many links in the pattern"), "1000", ToastTarget::None);
        }
        if links.is_empty() {
            if !too_many {
                self.toast(
                    icon::WARNING,
                    Color32::from_rgb(245, 158, 11),
                    tr("That is not a link"),
                    &text.trim().chars().take(200).collect::<String>(),
                    ToastTarget::None,
                );
            }
            return false;
        }
        // Their "download the copied link?" offers are answered.
        self.toasts.retain(|t| !matches!(&t.target, ToastTarget::Link(u) if links.iter().any(|l| l == u)));
        let requests = links.iter().map(|l| Incoming::link(l)).collect();
        self.add_requests(ctx, requests);
        true
    }

    /// Opens the dialog for links with what came along (from the browser).
    fn add_requests(&mut self, ctx: &egui::Context, requests: Vec<Incoming>) {
        let several = requests.len() > 1;
        let items = self.manager.items();
        for r in requests {
            // Several links at once start without questions; the browser
            // may ask for either.
            let quick = !r.ask.unwrap_or(self.settings.ask_first && !several)
                // "Downloads from the browser start at once."
                || (self.settings.browser_quiet && r.from_browser);
            let before = items
                .iter()
                .filter(|i| i.url == r.url && i.status == Status::Done)
                .max_by_key(|i| i.finished)
                .map(|i| (i.display_name().to_string(), i.finished.unwrap_or(i.added)));
            let mut dialog = AddDialog::new(r, &self.settings, quick);
            if before.is_some() {
                // Downloaded before: ask, whatever the setting.
                dialog.quick = false;
                dialog.before = before;
            }
            self.adds.push(dialog);
        }
        self.page = Page::Downloads;
        // The lookups start in the next frame (a few at once).
        ctx.request_repaint();
    }

    fn start(&mut self, downloads: Vec<add::NewDownload>) {
        let n = downloads.len();
        for d in downloads {
            let id = self.manager.add_with(&d.url, d.source, d.name, &d.dir, d.thumbnail, d.headers);
            if d.expected_hash.is_some() {
                self.manager.set_expected_hash(id, d.expected_hash);
            }
        }
        if n > 0 {
            // Whatever hid it: the status filter, a kind, a search.
            if matches!(self.filter, Filter::Done | Filter::Failed | Filter::Paused) {
                self.filter = Filter::All;
            }
            self.kind_filter = None;
            self.search.clear();
        }
    }

    /// The part of the add dialogs that is not drawing: lookups are started
    /// (a few at once), finished ones start their downloads. Runs from
    /// `logic()`, so that it goes on while the window is minimized.
    fn advance_adds(&mut self, ctx: &egui::Context) {
        // A few links are looked at at once (an imported list may have
        // hundreds); the one shown first.
        const AT_ONCE: usize = 4;
        let mut running = self.adds.iter().filter(|d| d.looking() && !d.waiting()).count();
        let shown = self.adds.iter().position(|d| !d.quick);
        let order = shown.into_iter().chain(0..self.adds.len());
        let settings = self.settings.clone();
        for i in order {
            if self.adds[i].waiting() && (running < AT_ONCE || Some(i) == shown) {
                self.adds[i].start(ctx, &settings);
                running += 1;
            }
        }
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
    }

    /// The dialog that is shown (the first one that is not quick).
    fn poll_adds(&mut self, ctx: &egui::Context, p: &Palette) {
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
            // Also when it went by between two frames (or while the window
            // was hidden), and for downloads of this run seen first finished.
            let ended = matches!(item.status, Status::Done | Status::Failed);
            let new = match before {
                Some(b) => b != item.status,
                None => item.added >= self.session_start,
            };
            if ended && new {
                news.push((item.id, item.status, item.display_name().to_string(), item.error.clone()));
            }
        }
        // Many failing together (no network, say) are one notice, not many.
        let failed = news.iter().filter(|n| n.1 == Status::Failed).count();
        if failed > 1 {
            let first = news.iter().find(|n| n.1 == Status::Failed).and_then(|n| n.3.clone()).unwrap_or_default();
            let detail = list::friendly(&first);
            let title = i18n::trf("{n} downloads failed", &[("n", &failed)]);
            self.toast(icon::WARNING_CIRCLE, p.danger, &title, &detail, ToastTarget::None);
            news.retain(|n| n.1 != Status::Failed);
        }
        for (id, status, name, error) in news {
            // In the background: the system tells too.
            if self.settings.notify && !ctx.input(|i| i.focused) {
                let title = if status == Status::Done { trl("Download complete") } else { trl("Download failed") };
                crate::power::notify(title, &name);
                if cfg!(windows) {
                    ctx.send_viewport_cmd(egui::ViewportCommand::RequestUserAttention(
                        egui::UserAttentionType::Informational,
                    ));
                }
            }
            match status {
                Status::Done => {
                    self.toast(icon::CHECK_CIRCLE, p.success, tr("Download complete"), &name, ToastTarget::Item(id));
                    if let Some(path) = items.iter().find(|i| i.id == id).and_then(|i| i.path.clone()) {
                        self.run_after(&path);
                    }
                }
                Status::Failed => {
                    let detail = format!("{name}\n{}", list::friendly(&error.unwrap_or_default()));
                    self.toast(icon::WARNING_CIRCLE, p.danger, tr("Download failed"), &detail, ToastTarget::None)
                }
                _ => {}
            }
        }
        if !self.cjk_loaded && self.cjk_checked.elapsed() > Duration::from_secs(1) {
            self.cjk_checked = Instant::now();
            let titles = items.iter().map(|i| i.name.as_str());
            let elsewhere =
                self.adds.iter().flat_map(|d| d.texts()).chain(self.toasts.iter().map(|t| t.detail.as_str()));
            if titles.chain(elsewhere).any(text::has_cjk) {
                theme::install_fonts(ctx, self.language, true);
                self.cjk_loaded = true;
            }
        }
    }

    /// The program of the settings, for a file that has just been finished
    /// (a virus scan, a copy to somewhere else, …). `{file}` is replaced by
    /// its path, or the path is the last argument. No shell is involved.
    fn run_after(&self, path: &std::path::Path) {
        let mut parts = swiftdm::util::split_command(&self.settings.after_command);
        if parts.is_empty() {
            return;
        }
        let file = path.to_string_lossy().into_owned();
        let program = parts.remove(0);
        // Never the program itself: that would run the downloaded file.
        if program.contains("{file}") {
            return;
        }
        let named = parts.iter().any(|a| a.contains("{file}"));
        for a in &mut parts {
            *a = a.replace("{file}", &file);
        }
        if !named {
            parts.push(file);
        }
        match swiftdm::tools::command(&program)
            .args(&parts)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
        {
            // Waited for in a thread: no zombie stays behind.
            Ok(mut child) => {
                std::thread::spawn(move || {
                    let _ = child.wait();
                });
            }
            Err(e) => log::warn!("the program for finished files ({program}) did not start: {e}"),
        }
    }

    /// While a file is held over the window: where to let it go.
    fn drop_hint(&self, ctx: &egui::Context, p: &Palette) {
        if ctx.input(|i| i.raw.hovered_files.is_empty()) {
            return;
        }
        let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("drop-hint")));
        let rect = ctx.content_rect().shrink(10.0);
        painter.rect_filled(rect, CornerRadius::same(16), p.tint(p.accent).gamma_multiply(2.0));
        painter.rect_stroke(rect, CornerRadius::same(16), Stroke::new(2.0, p.accent), egui::StrokeKind::Inside);
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            tr("Drop a file with links here"),
            egui::FontId::new(theme::scale::TITLE, egui::FontFamily::Name(theme::SEMIBOLD.into())),
            p.text,
        );
    }

    /// Files dropped on the window: lists of links are imported.
    fn take_dropped_files(&mut self, ctx: &egui::Context) {
        let files = ctx.input(|i| i.raw.dropped_files.clone());
        for f in files {
            // Only what can hold links; not a movie dropped by mistake.
            let path = f.path().to_path_buf();
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or_default().to_ascii_lowercase();
            if matches!(ext.as_str(), "txt" | "csv" | "list" | "md" | "html" | "htm" | "url" | "json") && path.is_file()
            {
                self.import_file(ctx, &path);
            }
        }
    }

    /// Offers links copied in other programs.
    fn watch_clipboard(&mut self, ctx: &egui::Context, p: &Palette) {
        let watching = self.settings.watch_clipboard;
        let just_turned_on = watching && !self.clip_watching;
        self.clip_watching = watching;
        if just_turned_on {
            // What was copied while it was off is not offered.
            self.clip_last =
                self.clipboard.as_mut().and_then(|c| c.get_text().ok()).unwrap_or_default().trim().to_string();
        }
        if !watching || self.clip_checked.elapsed() < Duration::from_millis(900) {
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
        let items = self.manager.items();
        let known = |l: &str| items.iter().any(|i| i.url == l) || self.adds.iter().any(|d| d.url == l);
        let new: Vec<&str> = text.split_whitespace().filter(|l| is_link(l) && !known(l)).collect();
        if !new.is_empty() {
            let host = if new.len() == 1 {
                swiftdm::util::host(new[0]).to_string()
            } else {
                if i18n::is_rtl() {
                    i18n::ar_plural(new.len() as u64, ["رابط واحد", "رابطان", "{n} روابط", "{n} رابطًا"])
                } else {
                    i18n::trlf("{n} links", &[("n", &new.len())])
                }
            };
            let text = new.join("\n");
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
            RowAction::StartNext(id) => self.manager.start_next(id),
            RowAction::CopyHash(id) => {
                if let Some(path) = item(id).and_then(|i| i.path) {
                    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    self.hashing.push((
                        name,
                        None,
                        Job::spawn(ctx, move |progress, stop| sha256(&path, progress, stop)),
                    ));
                }
            }
            RowAction::Remove(id) => self.manager.remove(id, false),
            RowAction::Delete(id) => self.confirm_delete = Some(id),
            RowAction::ChangeLink(id) => {
                let text = item(id).map(|i| i.url).unwrap_or_default();
                self.prompt = Some(Prompt { kind: PromptKind::ChangeLink(id), text, focused: false });
            }
            RowAction::Verify(id) => {
                // A checksum that was copied is taken at once.
                let text = self.clipboard.as_mut().and_then(|c| c.get_text().ok()).unwrap_or_default();
                let text = if swiftdm::util::parse_checksum(&text).is_some() { text } else { String::new() };
                self.prompt = Some(Prompt { kind: PromptKind::Verify(id), text, focused: false });
            }
        }
    }

    /// Once the screen is known: a window that fits it (at most 88% of it),
    /// centred and not maximized — on small screens some systems maximize a
    /// window that is larger than the screen.
    fn fit_to_screen(&mut self, ctx: &egui::Context) {
        if self.fitted {
            return;
        }
        // Development aid: a size given by SWIFTDM_SIZE stays.
        #[cfg(debug_assertions)]
        if std::env::var_os("SWIFTDM_SIZE").is_some() {
            self.fitted = true;
            return;
        }
        let (monitor, outer) = ctx.input(|i| (i.viewport().monitor_size, i.viewport().outer_rect));
        let Some(monitor) = monitor.filter(|m| m.x > 100.0 && m.y > 100.0) else { return };
        self.fitted = true;
        ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(false));
        // Started at sign-in: no window in the way.
        if std::env::args().any(|a| a == crate::autostart::ARG) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
        }
        let zoom = ctx.zoom_factor();
        // In points (the window's size is in logical points, before zoom).
        // About 70% of the screen, between the first size and a large one,
        // and never more than 88%.
        let want = (monitor * 0.7)
            .clamp(Vec2::from(DEFAULT_SIZE), Vec2::from(LARGE_SIZE))
            .min(monitor * 0.88)
            .max(Vec2::from(MIN_SIZE).min(monitor * 0.95));
        let size = ctx.input(|i| i.viewport().inner_rect.map(|r| r.size())).unwrap_or(want * zoom) / zoom;
        if (size - want).abs().max_elem() > 1.0 {
            log::info!("swiftdm: fitting the window to a {}×{} screen", monitor.x, monitor.y);
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(want));
            if let Some(outer) = outer {
                let left_top = outer.min + (outer.size() - want) / 2.0;
                ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(left_top.max(egui::Pos2::ZERO)));
            }
        }
    }

    /// Runs a command of the menu bar, a shortcut or a button.
    fn run(&mut self, ctx: &egui::Context, c: Command) {
        match c {
            Command::NewDownload => {
                self.page = Page::Downloads;
                self.focus_url = true;
            }
            Command::PasteLink => {
                if let Some(t) = self.clipboard.as_mut().and_then(|c| c.get_text().ok()) {
                    self.clip_last = t.trim().to_string();
                    self.add_links(ctx, &t);
                }
            }
            Command::ImportLinks => self.import_links(ctx),
            Command::OpenFolder => {
                let _ = std::fs::create_dir_all(&self.settings.download_dir);
                let _ = swiftdm::util::open_path(&self.settings.download_dir);
            }
            Command::Show(n) => {
                self.page = Page::Downloads;
                self.filter = [Filter::All, Filter::Active, Filter::Done, Filter::Paused, Filter::Failed][n.min(4)];
            }
            Command::Settings => self.page = Page::Settings,
            Command::Help => self.page = Page::Help,
            Command::About => self.page = Page::About,
            Command::AddToBrowser => {
                self.page = Page::Settings;
                self.settings_page.tab = settings::Tab::Browser;
            }
            Command::PauseAll => self.manager.pause_all(),
            Command::ResumeAll => self.manager.resume_all(),
            Command::ClearCompleted => self.manager.clear_finished(),
            Command::AfterDone(a) => self.power.after_done = a,
            Command::CheckUpdates => {
                self.updater.check(ctx, true);
                self.page = Page::About;
            }
            Command::ZoomIn => self.settings.ui_scale = ((self.settings.ui_scale * 10.0).round() / 10.0 + 0.1).min(1.5),
            Command::ZoomOut => {
                self.settings.ui_scale = ((self.settings.ui_scale * 10.0).round() / 10.0 - 0.1).max(0.8)
            }
            Command::ZoomReset => self.settings.ui_scale = 1.0,
            Command::Theme(t) => self.settings.theme = t,
            Command::Open(url) => ctx.open_url(egui::OpenUrl::new_tab(url)),
            Command::WhatsNew => {
                // This version's own page, with what is new in it.
                let url = format!("https://github.com/itsmrroot/SwiftDM-release/releases/tag/v{}", update::current());
                ctx.open_url(egui::OpenUrl::new_tab(url));
            }
            Command::Minimize => ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true)),
            Command::FullScreen => {
                let full = ctx.input(|i| i.viewport().fullscreen.unwrap_or(false));
                ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(!full));
            }
            Command::Quit => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            Command::Undo | Command::Redo | Command::Cut | Command::Copy | Command::Paste | Command::SelectAll => {
                crate::menu::edit(ctx, c)
            }
        }
    }

    fn on_due(&mut self, ctx: &egui::Context, due: Due) {
        match due {
            Due::None => {}
            Due::StartAll => self.manager.resume_all(),
            Due::PauseAll => self.manager.pause_all(),
            Due::Act(AfterDone::Quit) => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            Due::Act(_) => {}
        }
    }

    /// Links from a text file (one or more per line).
    fn import_links(&mut self, ctx: &egui::Context) {
        let Some(path) = crate::instance::in_dialog(|| {
            rfd::FileDialog::new()
                .add_filter(trl("Text files"), &["txt", "csv", "list", "md", "html", "htm"])
                .add_filter(trl("All files"), &["*"])
                .pick_file()
        }) else {
            return;
        };
        self.import_file(ctx, &path);
    }

    /// The links in a text, HTML or CSV file are added.
    fn import_file(&mut self, ctx: &egui::Context, path: &std::path::Path) {
        if std::fs::metadata(path).is_ok_and(|m| m.len() > 5_000_000) {
            let amber = Color32::from_rgb(245, 158, 11);
            self.toast(icon::WARNING, amber, tr("That is not a link"), &path.display().to_string(), ToastTarget::None);
            return;
        }
        match std::fs::read(path) {
            Ok(bytes) => {
                let text = String::from_utf8_lossy(&bytes);
                // Links inside quotes or brackets (HTML) and lists (CSV)
                // count too; commas inside a link stay.
                let spaced: String =
                    text.chars().map(|c| if matches!(c, '"' | '\'' | '<' | '>') { ' ' } else { c }).collect();
                let spaced = spaced.replace(",http", " http").replace(";http", " http");
                let cleaned = spaced
                    .split_whitespace()
                    .map(|w| w.trim_matches(|c| matches!(c, ',' | ';' | '(' | ')' | '[' | ']')))
                    .collect::<Vec<_>>()
                    .join("\n");
                self.add_links(ctx, &cleaned);
            }
            Err(e) => {
                let detail = format!("{}\n{e}", path.display());
                self.toast(
                    icon::WARNING,
                    Color32::from_rgb(220, 38, 38),
                    tr("The file could not be read"),
                    &detail,
                    ToastTarget::None,
                )
            }
        }
    }

    fn poll_hashing(&mut self, ctx: &egui::Context, p: &Palette) {
        let mut done = Vec::new();
        self.hashing.retain(|(name, want, job)| match job.poll() {
            Some(r) => {
                done.push((name.clone(), want.clone(), r));
                false
            }
            None => true,
        });
        if !self.hashing.is_empty() {
            ctx.request_repaint_after(Duration::from_millis(200));
        }
        for (name, want, r) in done {
            match (r, want) {
                // A checksum to compare with: it matches or it does not.
                (Ok(got), Some(want)) if got == want => {
                    self.toast(icon::SEAL_CHECK, p.success, tr("The checksum matches"), &name, ToastTarget::None)
                }
                (Ok(got), Some(_)) => {
                    let detail = format!("{name}\n{got}");
                    self.toast(
                        icon::SEAL_WARNING,
                        p.danger,
                        tr("The checksum does not match"),
                        &detail,
                        ToastTarget::None,
                    )
                }
                (Ok(hash), None) => {
                    ctx.copy_text(hash.clone());
                    self.clip_last = hash.clone();
                    let detail = format!("{name}\n{hash}");
                    self.toast(icon::FINGERPRINT, p.success, tr("SHA-256 copied"), &detail, ToastTarget::None);
                }
                (Err(e), _) => {
                    let detail = format!("{name}\n{e:#}");
                    self.toast(icon::WARNING, p.danger, tr("The file could not be read"), &detail, ToastTarget::None)
                }
            }
        }
    }

    /// The small dialog with one line to type in.
    fn prompt_dialog(&mut self, ctx: &egui::Context, p: &Palette) {
        let Some(prompt) = &mut self.prompt else { return };
        let (title, hint, ok) = match prompt.kind {
            PromptKind::ChangeLink(_) => (
                tr("Change the link"),
                trl("Paste the new address of the file. What was downloaded so far is kept."),
                tr("Change"),
            ),
            PromptKind::Verify(_) => (
                tr("Verify the checksum"),
                trl("Paste the checksum the site gives for this file (SHA-256 or SHA-512)."),
                tr("Check"),
            ),
        };
        let mut result = None;
        let modal = egui::Modal::new(egui::Id::new("prompt")).show(ctx, |ui| {
            ui.set_width(520.0_f32.min(ctx.content_rect().width() - 48.0));
            ui.label(theme::semibold(title, theme::scale::LEAD));
            ui.add_space(4.0);
            theme::paragraph(ui, hint, theme::scale::BODY, p.weak);
            ui.add_space(8.0);
            let line = ui.add(egui::TextEdit::singleline(&mut prompt.text).desired_width(ui.available_width()));
            if !prompt.focused {
                line.request_focus();
                prompt.focused = true;
            }
            let enter = line.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            let usable = match prompt.kind {
                PromptKind::ChangeLink(_) => is_link(prompt.text.trim()),
                PromptKind::Verify(_) => swiftdm::util::parse_checksum(&prompt.text).is_some(),
            };
            ui.add_space(12.0);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if (theme::primary_button(ui, p, &format!("  {ok}  "), usable).clicked() || enter) && usable {
                    result = Some(true);
                }
                if theme::secondary_button(ui, tr("Cancel")).clicked() {
                    result = Some(false);
                }
            });
        });
        if modal.should_close() {
            result = Some(false);
        }
        let Some(go) = result else { return };
        let Some(prompt) = self.prompt.take() else { return };
        if !go {
            return;
        }
        match prompt.kind {
            PromptKind::ChangeLink(id) => match self.manager.change_url(id, &prompt.text) {
                Ok(()) => self.toast(icon::LINK_SIMPLE, p.success, tr("The link was changed"), "", ToastTarget::None),
                Err(e) => self.toast(
                    icon::WARNING,
                    p.danger,
                    tr("The link was not changed"),
                    &i18n::engine(e),
                    ToastTarget::None,
                ),
            },
            PromptKind::Verify(id) => {
                let Some(want) = swiftdm::util::parse_checksum(&prompt.text) else { return };
                if let Some(path) = self.manager.item(id).and_then(|i| i.path) {
                    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    let w = want.clone();
                    self.hashing.push((
                        name,
                        Some(want),
                        Job::spawn(ctx, move |_progress, _stop| Ok(swiftdm::util::hash_file(&path, &w)?)),
                    ));
                }
            }
        }
    }

    fn header(&mut self, ui: &mut Ui, p: &Palette) {
        // Narrow (a small window, a large zoom): the chips are symbols only
        // (their tooltips say the rest) and the name goes.
        let compact = ui.available_width() < 900.0;
        // Right to left: the name on the right, the chips on the left.
        let rtl = i18n::is_rtl();
        let (outer, inner) = if rtl {
            (Layout::right_to_left(Align::Center), Layout::left_to_right(Align::Center))
        } else {
            (Layout::left_to_right(Align::Center), Layout::right_to_left(Align::Center))
        };
        ui.with_layout(outer, |ui| {
            ui.add(egui::Image::new(&self.logo).fit_to_exact_size(Vec2::splat(30.0)));
            ui.add_space(2.0);
            if !compact {
                ui.label(theme::semibold("SwiftDM", theme::scale::LEAD).color(p.text));
            }
            ui.with_layout(inner, |ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                let chip = |text: String, on: bool| {
                    egui::Button::new(RichText::new(text).size(theme::scale::CAPTION).color(if on {
                        p.accent_text
                    } else {
                        p.text
                    }))
                    .fill(if on { p.tint(p.accent) } else { p.card })
                    .stroke(Stroke::new(1.0, if on { p.accent.gamma_multiply(0.6) } else { p.control_border }))
                    .corner_radius(CornerRadius::same(9))
                    .min_size(Vec2::new(0.0, 32.0))
                };
                // Quick settings: the ones changed now and then, without
                // leaving the list.
                let mut open_settings = false;
                // Stays open while sliders and boxes are used.
                let keep_open = egui::containers::menu::MenuConfig::new()
                    .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside);
                EndMenu::from_button(chip(icon::SLIDERS_HORIZONTAL.to_string(), false))
                    .config(keep_open)
                    .ui(ui, |ui| {
                        ui.set_min_width(300.0);
                        let s = &mut self.settings;
                        ui.label(theme::semibold(tr("Quick settings"), theme::scale::BODY));
                        ui.add_space(4.0);
                        ui.label(RichText::new(tr("Download")).color(p.weak).size(theme::scale::CAPTION));
                        ui.horizontal(|ui| {
                            ui.selectable_value(&mut s.mode, swiftdm::media::Mode::Video, tr("Video"));
                            ui.selectable_value(&mut s.mode, swiftdm::media::Mode::Audio, tr("Sound only"));
                        });
                        if s.mode == swiftdm::media::Mode::Video {
                            ui.label(RichText::new(tr("Video quality")).color(p.weak).size(theme::scale::CAPTION));
                            ui.horizontal_wrapped(|ui| {
                                ui.selectable_value(&mut s.max_height, None, tr("Best"));
                                for h in [1080, 720, 480] {
                                    ui.selectable_value(&mut s.max_height, Some(h), settings::height_label(Some(h)));
                                }
                            });
                        }
                        ui.separator();
                        ui.checkbox(&mut s.ask_first, tr("Show the choices (quality, file name, folder)"));
                        ui.checkbox(&mut s.watch_clipboard, tr("Offer to download links I copy"));
                        ui.checkbox(&mut s.notify, tr("Show a notification of the system"));
                        ui.checkbox(&mut s.keep_awake, tr("Keep the computer awake while downloading"));
                        ui.checkbox(&mut s.sort_by_kind, tr("Sort into folders: Video, Music, Documents, Programs, …"));
                        ui.separator();
                        ui.label(RichText::new(tr("Save to")).color(p.weak).size(theme::scale::CAPTION));
                        ui.add(
                            egui::Label::new(
                                RichText::new(s.download_dir.display().to_string()).size(theme::scale::CAPTION),
                            )
                            .truncate(),
                        );
                        ui.horizontal(|ui| {
                            if ui.button(icon_label(icon::FOLDER_OPEN, "Change…")).clicked()
                                && let Some(d) = crate::instance::in_dialog(|| {
                                    rfd::FileDialog::new().set_directory(&s.download_dir).pick_folder()
                                })
                            {
                                s.download_dir = d;
                            }
                            if ui.button(tr("Open")).clicked() {
                                let _ = std::fs::create_dir_all(&s.download_dir);
                                let _ = swiftdm::util::open_path(&s.download_dir);
                            }
                        });
                        ui.separator();
                        if ui.button(icon_label(icon::GEAR_SIX, "All settings…")).clicked() {
                            open_settings = true;
                            ui.close();
                        }
                    })
                    .0
                    .on_hover_text(tr("Quick settings"));
                if open_settings {
                    self.page = Page::Settings;
                }
                // Language.
                let lang = self.settings.language;
                EndMenu::from_button(chip(icon::GLOBE.to_string(), false))
                    .ui(ui, |ui| {
                        for l in i18n::Language::ALL {
                            if ui.selectable_label(lang == l, l.label().as_ref()).clicked() {
                                self.settings.language = l;
                            }
                        }
                    })
                    .0
                    .on_hover_text(tr("Language"));
                // Theme.
                let theme_icon = if ui.visuals().dark_mode { icon::MOON } else { icon::SUN };
                EndMenu::from_button(chip(theme_icon.to_string(), false))
                    .ui(ui, |ui| {
                        for t in ThemeChoice::ALL {
                            if ui.selectable_label(self.settings.theme == t, t.name()).clicked() {
                                self.settings.theme = t;
                            }
                        }
                    })
                    .0
                    .on_hover_text(tr("Theme"));
                // When done.
                let a = self.power.after_done;
                EndMenu::from_button(chip(
                    if compact {
                        a.icon().to_string()
                    } else {
                        i18n::icon_text(a.icon(), if a == AfterDone::Nothing { tr("When done") } else { a.name() })
                    },
                    a != AfterDone::Nothing,
                ))
                .ui(ui, |ui| {
                    ui.label(RichText::new(tr("When downloads are done")).color(p.weak).size(theme::scale::CAPTION));
                    for x in AfterDone::ALL {
                        if ui.selectable_label(a == x, i18n::icon_text(x.icon(), x.name())).clicked() {
                            self.power.after_done = x;
                        }
                    }
                });
                // Downloads at once.
                let n = self.settings.max_active;
                EndMenu::from_button(chip(
                    if compact {
                        format!("{}  {n}", icon::STACK)
                    } else {
                        i18n::icon_text(icon::STACK, &i18n::trf("{n} at once", &[("n", &n)]))
                    },
                    false,
                ))
                .ui(ui, |ui| {
                    ui.label(RichText::new(tr("Downloads at the same time")).color(p.weak).size(theme::scale::CAPTION));
                    for x in 1..=6 {
                        if ui.selectable_label(n == x, x.to_string()).clicked() {
                            self.settings.max_active = x;
                        }
                    }
                })
                .0
                .on_hover_text(tr("Downloads at the same time"));
                // Speed limit.
                let kb = self.settings.speed_limit_kb;
                let shown = if kb == 0 {
                    tr("Unlimited").to_string()
                } else {
                    format!("{}/s", swiftdm::util::format_size(kb * 1024))
                };
                EndMenu::from_button(chip(
                    if compact && kb == 0 {
                        icon::LIGHTNING.to_string()
                    } else {
                        format!("{}  {shown}", icon::LIGHTNING)
                    },
                    kb > 0,
                ))
                .ui(ui, |ui| {
                    ui.label(RichText::new(tr("Speed limit")).color(p.weak).size(theme::scale::CAPTION));
                    for x in [0u64, 256, 512, 1024, 2048, 5120, 10240] {
                        let label = if x == 0 {
                            tr("Unlimited").to_string()
                        } else {
                            format!("{}/s", swiftdm::util::format_size(x * 1024))
                        };
                        if ui.selectable_label(kb == x, label).clicked() {
                            self.settings.speed_limit_kb = x;
                        }
                    }
                })
                .0
                .on_hover_text(tr("Speed limit"));
                // The browser extension.
                let connected = self.browser_seen.is_some();
                let label = match (connected, compact) {
                    (true, true) => icon::CHECK_CIRCLE.to_string(),
                    (false, true) => icon::PUZZLE_PIECE.to_string(),
                    (true, false) => i18n::icon_text(icon::CHECK_CIRCLE, tr("Browser")),
                    (false, false) => i18n::icon_text(icon::PUZZLE_PIECE, tr("Add to browser")),
                };
                let tip = if connected {
                    tr("The browser extension is connected")
                } else {
                    tr("Add SwiftDM to your browser")
                };
                if ui.add(chip(label, !connected)).on_hover_text(tip).clicked() {
                    self.run(&ui.ctx().clone(), Command::AddToBrowser);
                }
                if self.updater.available().is_some() {
                    self.updater.button(ui, p);
                }
            });
        });
    }

    // -----------------------------------------------------------------------
    // Layout

    fn sidebar(&mut self, ui: &mut Ui, p: &Palette, items: &[Item]) {
        ui.add_space(6.0);
        // Room for the lists and the pages below them: the pages stay at the
        // bottom. In a short window (or a large zoom) they follow the lists
        // instead, so that nothing is hidden: it all scrolls.
        let short = ui.available_height() < 560.0;
        if !short {
            egui::Panel::bottom("sidebar-bottom")
                .resizable(false)
                .show_separator_line(false)
                .frame(Frame::NONE)
                .show(ui, |ui| self.sidebar_pages(ui, p));
        }
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
            if short {
                ui.add_space(8.0);
                self.sidebar_pages(ui, p);
            }
        });
    }

    /// The line, then Download folder, Settings, Help, About and the version.
    fn sidebar_pages(&mut self, ui: &mut Ui, p: &Palette) {
        ui.add(egui::Separator::default().spacing(8.0));
        ui.add_space(4.0);
        if nav_item(ui, p, icon::FOLDER_OPEN, tr("Download folder"), false, None).clicked() {
            let _ = std::fs::create_dir_all(&self.settings.download_dir);
            let _ = swiftdm::util::open_path(&self.settings.download_dir);
        }
        let badge = self.updater.available().map(|_| "●");
        if nav_item(ui, p, icon::GEAR_SIX, tr("Settings"), self.page == Page::Settings, badge).clicked() {
            self.page = Page::Settings;
        }
        if nav_item(ui, p, icon::QUESTION, tr("Help"), self.page == Page::Help, None).clicked() {
            self.page = Page::Help;
        }
        if nav_item(ui, p, icon::INFO, tr("About"), self.page == Page::About, None).clicked() {
            self.page = Page::About;
        }
        ui.add_space(4.0);
        let version = i18n::trf("Version {version}", &[("version", &update::current())]);
        ui.label(RichText::new(version).color(p.weak).size(theme::scale::CAPTION));
        ui.add_space(14.0);
    }

    fn link_bar(&mut self, ui: &mut Ui, p: &Palette) {
        let ctx = ui.ctx().clone();
        Frame::new()
            .fill(p.surface())
            .stroke(Stroke::new(1.5, p.accent.gamma_multiply(0.55)))
            .corner_radius(CornerRadius::same(14))
            .inner_margin(Margin { left: 14, right: 8, top: 8, bottom: 8 })
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(icon::LINK).size(theme::scale::ICON_LG).color(p.accent));
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
                            .font(egui::FontId::proportional(theme::scale::LEAD))
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
                        // What was typed here is added; what someone else
                        // sent (the browser, a copied link) leaves it alone.
                        // A text that gave nothing stays, to be corrected.
                        if self.add_links(&ctx, &url) {
                            self.url.clear();
                        }
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
            if self.helpers.busy() {
                return;
            }
        }
        if missing_ytdlp || (missing_other && !self.settings.welcomed) {
            theme::card(ui, p, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    let (rect, _) = ui.allocate_exact_size(Vec2::splat(44.0), Sense::hover());
                    ui.painter().rect_filled(rect, CornerRadius::same(12), p.tint(p.accent));
                    ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, icon::SPARKLE, egui::FontId::proportional(theme::scale::ICON_LG), p.accent);
                    ui.vertical(|ui| {
                        let title = if missing_ytdlp { tr("One step to download from video sites") } else { tr("Finish setting up video sites") };
                        ui.label(theme::semibold(title, theme::scale::LEAD));
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

    /// What goes beside the title of the list: type, search, and the
    /// buttons for all of them.
    fn list_controls(&mut self, ui: &mut Ui, items: &[Item], narrow: bool) {
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
                .desired_width(if narrow { 150.0 } else { 200.0 }),
        );
        // Only one kind of file: video, music, pictures, ….
        let label = match self.kind_filter {
            Some(k) => i18n::icon_text(list::kind_icon(k), list::kind_name(k)),
            None => icon_label(icon::FUNNEL, "All types"),
        };
        egui::containers::menu::MenuButton::new(label).ui(ui, |ui| {
            ui.selectable_value(&mut self.kind_filter, None, tr("All types"));
            for k in [
                FileKind::Video,
                FileKind::Audio,
                FileKind::Image,
                FileKind::Archive,
                FileKind::Program,
                FileKind::Document,
                FileKind::Other,
            ] {
                let n = items.iter().filter(|i| i.kind() == k).count();
                if n > 0 || self.kind_filter == Some(k) {
                    let text = format!("{}  {}  ({n})", list::kind_icon(k), list::kind_name(k));
                    ui.selectable_value(&mut self.kind_filter, Some(k), text);
                }
            }
        });
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
            .filter(|i| self.kind_filter.is_none_or(|k| i.kind() == k))
            .filter(|i| {
                let q = self.search.trim().to_lowercase();
                q.is_empty() || i.display_name().to_lowercase().contains(&q) || i.url.to_lowercase().contains(&q)
            })
            .collect();
        let narrow = ui.available_width() < 800.0;
        if narrow {
            // Two lines: the title, then the controls (which wrap).
            ui.horizontal(|ui| {
                ui.label(theme::semibold(self.filter.title(), theme::scale::LEAD));
                ui.label(RichText::new(shown.len().to_string()).color(p.weak));
            });
            ui.horizontal_wrapped(|ui| self.list_controls(ui, items, true));
        } else {
            ui.horizontal(|ui| {
                ui.label(theme::semibold(self.filter.title(), theme::scale::LEAD));
                ui.label(RichText::new(shown.len().to_string()).color(p.weak));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| self.list_controls(ui, items, false));
            });
        }
        ui.add_space(8.0);

        if shown.is_empty() {
            empty_state(ui, p, items.is_empty(), &self.search);
            return;
        }
        let mut actions = Vec::new();
        let spacing = 8.0;
        egui::ScrollArea::vertical().auto_shrink([false, false]).show_rows(
            ui,
            // The height of a row without the space between rows (egui adds that).
            list::ROW_OUTER,
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
                    ui.label(theme::semibold("SwiftDM", theme::scale::DISPLAY));
                    ui.label(RichText::new(i18n::trf("Version {version}", &[("version", &update::current())])).color(p.weak));
                    ui.label(RichText::new("Powered by Bashar Salmo").color(p.accent_text));
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
                // Only the Linux packages put it on the PATH: show where it is.
                let cli = std::env::current_exe().ok().and_then(|exe| {
                    Some(exe.parent()?.join(if cfg!(windows) { "swiftdm.exe" } else { "swiftdm" }))
                });
                if let Some(cli) = cli.filter(|c| c.is_file()) {
                    let shown = cli.display().to_string();
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(tr("It is here:")).color(p.weak));
                        ui.label(RichText::new(&shown).monospace().size(theme::scale::CAPTION));
                        if ui.small_button(icon::COPY).on_hover_text(tr("Copy")).clicked() {
                            ui.ctx().copy_text(shown.clone());
                        }
                    });
                }
                ui.add_space(4.0);
                for line in [
                    "swiftdm https://youtu.be/…              # best video",
                    "swiftdm -a -k 320 https://…             # MP3, 320 kbps",
                    "swiftdm -q 720 -o ~/Videos https://…    # 720p, to a folder",
                    "swiftdm -c 16 https://example.com/x.iso # 16 connections",
                    "swiftdm info https://…                  # what is behind a link",
                ] {
                    ui.label(RichText::new(line).monospace().size(theme::scale::CAPTION));
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
                        .fill(p.surface())
                        .stroke(Stroke::new(1.0, t.color.gamma_multiply(0.6)))
                        .corner_radius(CornerRadius::same(12))
                        .shadow(ui.visuals().popup_shadow)
                        .inner_margin(Margin::same(12))
                        .show(ui, |ui| {
                            ui.set_width(336.0);
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(t.glyph).size(theme::scale::ICON_LG).color(t.color));
                                ui.vertical(|ui| {
                                    ui.spacing_mut().item_spacing.y = 2.0;
                                    ui.horizontal(|ui| {
                                        ui.label(theme::semibold(&t.text, theme::scale::BODY));
                                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                            if ui.small_button(icon::X).clicked() {
                                                close = Some(i);
                                            }
                                        });
                                    });
                                    for line in t.detail.lines().take(3) {
                                        ui.add(
                                            egui::Label::new(
                                                text::rich(line).color(p.weak).size(theme::scale::CAPTION),
                                            )
                                            .truncate(),
                                        );
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
                ui.label(RichText::new(icon::TRASH).size(theme::scale::ICON_LG).color(p.danger));
                ui.label(theme::semibold(tr("Delete this file?"), theme::scale::LEAD));
            });
            ui.add_space(6.0);
            ui.add(egui::Label::new(text::rich(item.display_name())).wrap());
            ui.label(RichText::new(tr("The file is deleted from the disk, not moved to the trash.")).color(p.weak));
            ui.add_space(12.0);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui
                    .add(
                        egui::Button::new(theme::semibold(tr("Delete"), theme::scale::BODY).color(p.on_danger))
                            .fill(p.danger_fill)
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
    /// Runs every frame, also while the window is minimized or hidden.
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Hand-overs from another start or the browser: taking them here
        // confirms to the sender that this SwiftDM works (see instance.rs).
        let in_front = ctx.input(|i| i.focused);
        while let Some(links) = self.inbox.as_ref().and_then(|i| i.poll(in_front)) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            if !links.is_empty() {
                self.add_requests(ctx, links);
                // Where the system keeps the window from coming to the front
                // (Windows may), the taskbar button flashes: the choices wait.
                ctx.send_viewport_cmd(egui::ViewportCommand::RequestUserAttention(
                    egui::UserAttentionType::Informational,
                ));
            }
        }
        // Also while minimized: notifications, the schedule, when done.
        let p = Palette::new(
            ctx.global_style().visuals.dark_mode,
            self.settings.theme == ThemeChoice::Night,
            self.settings.accent,
        );
        let items = self.manager.items();
        self.watch_items(ctx, &items, &p);
        self.watch_clipboard(ctx, &p);
        let paused = items.iter().any(|i| i.status == Status::Paused);
        let due = self.power.tick(
            ctx,
            self.manager.busy(),
            paused,
            self.settings.keep_awake,
            self.settings.start_at,
            self.settings.pause_at,
        );
        self.on_due(ctx, due);
        // What goes on in the background, shown or not.
        self.advance_adds(ctx);
        self.helpers.poll(ctx);
        if self.updater.poll(ctx) {
            // The installer takes over and starts the new version.
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        self.poll_hashing(ctx, &p);
        self.take_dropped_files(ctx);
        if self.manager.busy() || self.helpers.busy() || self.adds.iter().any(|d| d.looking()) {
            ctx.request_repaint_after(Duration::from_millis(250));
        }
        if self.settings.watch_clipboard {
            ctx.request_repaint_after(Duration::from_secs(1));
        }
    }

    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        // The graphics work once several pictures were really shown for a
        // moment: a crash in the first present (a bad driver) must still leave
        // the marker that makes the next start use OpenGL.
        {
            use std::sync::atomic::{AtomicU32, Ordering};
            static FRAMES: AtomicU32 = AtomicU32::new(0);
            static SINCE: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
            static DRAWN: std::sync::Once = std::sync::Once::new();
            let frames = FRAMES.fetch_add(1, Ordering::Relaxed) + 1;
            let since = *SINCE.get_or_init(Instant::now);
            if !DRAWN.is_completed() {
                if frames >= 6 && since.elapsed() > Duration::from_millis(1500) {
                    DRAWN.call_once(crate::drawn);
                } else {
                    ctx.request_repaint_after(Duration::from_millis(300));
                }
            }
        }
        self.fit_to_screen(&ctx);
        self.apply_settings(&ctx);
        self.thumbs.poll(&ctx);
        let p = Palette::new(ui.visuals().dark_mode, self.settings.theme == ThemeChoice::Night, self.settings.accent);
        // Under everything: the soft background that frosted surfaces lie on.
        theme::paint_backdrop(ui.painter(), ui.max_rect(), &p, p.accent);
        let items = self.manager.items();

        // Ctrl/Cmd+V outside a text field adds the link.
        let nothing_focused = ctx.memory(|m| m.focused().is_none());
        if nothing_focused && self.adds.iter().all(|d| d.quick) {
            let pasted = ctx.input(|i| {
                i.events.iter().find_map(|e| if let egui::Event::Paste(s) = e { Some(s.clone()) } else { None })
            });
            if let Some(text) = pasted
                && text.split_whitespace().any(is_link)
            {
                self.clip_last = text.trim().to_string();
                self.add_links(&ctx, &text);
            }
        }

        for c in self.menu.commands(&ctx, self.language) {
            self.run(&ctx, c);
        }
        if self.browser_checked.elapsed() > Duration::from_secs(5) {
            self.browser_checked = Instant::now();
            self.browser_seen = crate::browser::last_seen();
        }

        // Windows and Linux have no menu bar of their own: one in the window.
        // Development aid: SWIFTDM_WINDOW_MENU shows it on macOS too.
        let window_menu =
            !cfg!(target_os = "macos") || (cfg!(debug_assertions) && std::env::var_os("SWIFTDM_WINDOW_MENU").is_some());
        if window_menu {
            let (after_done, theme) = (self.power.after_done, self.settings.theme);
            let chosen = egui::Panel::top("menubar")
                .resizable(false)
                .show_separator_line(false)
                .frame(Frame::new().fill(p.bar()).inner_margin(Margin::symmetric(8, 2)))
                .show(ui, |ui| crate::menu::window_menu(ui, after_done, theme))
                .inner;
            if let Some(c) = chosen {
                self.run(&ctx, c);
            }
        }
        egui::Panel::top("header")
            .exact_size(54.0)
            .resizable(false)
            .show_separator_line(false)
            .frame(Frame::new().fill(p.header_bar()).stroke(Stroke::new(1.0, p.border)).inner_margin(Margin {
                left: 16,
                right: 14,
                top: 0,
                bottom: 0,
            }))
            .show(ui, |ui| self.header(ui, &p));
        // Right to left: the sidebar is on the right.
        let sidebar = if i18n::is_rtl() { egui::Panel::right("sidebar") } else { egui::Panel::left("sidebar") };
        sidebar
            .exact_size(232.0)
            .resizable(false)
            .frame(Frame::new().fill(p.bar()).stroke(Stroke::new(1.0, p.border)).inner_margin(Margin::symmetric(12, 0)))
            .show(ui, |ui| self.sidebar(ui, &p, &items));
        egui::CentralPanel::default()
            .frame(Frame::new().fill(p.page()).inner_margin(Margin { left: 26, right: 26, top: 22, bottom: 18 }))
            .show(ui, |ui| match self.page {
                Page::Downloads => self.downloads_page(ui, &p, &items),
                Page::Settings => {
                    let cx = settings::Context {
                        helpers: &mut self.helpers,
                        updater: &mut self.updater,
                        after_done: &mut self.power.after_done,
                        browser_seen: self.browser_seen,
                    };
                    settings::page(ui, &p, &mut self.settings, &mut self.settings_page, cx);
                }
                Page::Help => {
                    let action = egui::ScrollArea::vertical()
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            // About 70 characters to a line.
                            ui.set_max_width(ui.available_width().min(780.0));
                            crate::help::page(ui, &p)
                        })
                        .inner;
                    match action {
                        HelpAction::AddToBrowser => self.run(&ctx, Command::AddToBrowser),
                        HelpAction::Settings => self.run(&ctx, Command::Settings),
                        HelpAction::None => {}
                    }
                }
                Page::About => self.about_page(ui, &p),
            });
        self.poll_adds(&ctx, &p);
        self.delete_dialog(&ctx, &p);
        self.prompt_dialog(&ctx, &p);
        self.updater.dialog(&ctx, &p, self.manager.busy());
        let failed = items.iter().filter(|i| i.status == Status::Failed).count();
        let due = self.power.dialog(&ctx, &p, failed);
        self.on_due(&ctx, due);
        self.toasts_ui(&ctx, &p);
        self.drop_hint(&ctx, &p);
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        // The screenshot language is not the user's.
        #[cfg(debug_assertions)]
        if std::env::var_os("SWIFTDM_LANG").is_some() {
            return;
        }
        eframe::set_value(storage, SETTINGS_KEY, &self.settings);
        // A window state saved by SwiftDM 1.0 (maximized, or on a screen
        // that is gone) must not come back.
        storage.set_string("window", String::new());
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        // Running downloads continue at the next start.
        self.manager.shutdown();
        // An update chosen to be installed when the app closes.
        self.updater.install_on_close();
    }
}

/// A menu button like egui's, whose menu is lined up with the button's
/// right edge (and so stays inside the window, for the buttons at the right
/// end of the top bar).
struct EndMenu<'a> {
    button: egui::Button<'a>,
    config: Option<egui::containers::menu::MenuConfig>,
}

impl<'a> EndMenu<'a> {
    fn from_button(button: egui::Button<'a>) -> Self {
        EndMenu { button, config: None }
    }

    fn config(mut self, config: egui::containers::menu::MenuConfig) -> Self {
        self.config = Some(config);
        self
    }

    fn ui<R>(
        self,
        ui: &mut Ui,
        content: impl FnOnce(&mut Ui) -> R,
    ) -> (egui::Response, Option<egui::InnerResponse<R>>) {
        use egui::containers::menu::MenuConfig;
        let response = ui.add(self.button);
        let config = self.config.unwrap_or_default();
        let inner = egui::Popup::menu(&response)
            .align(egui::RectAlign::BOTTOM_END)
            .close_behavior(config.close_behavior)
            .style(config.style.clone())
            .info(egui::UiStackInfo::new(egui::UiKind::Menu).with_tag_value(MenuConfig::MENU_CONFIG_TAG, config))
            .show(content);
        (response, inner)
    }
}

fn group_label(ui: &mut Ui, p: &Palette, text: &str) {
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.add_space(14.0);
        ui.label(RichText::new(text).size(theme::scale::CAPTION).color(p.weak).strong());
    });
}

fn nav_item(ui: &mut Ui, p: &Palette, glyph: &str, label: &str, active: bool, badge: Option<&str>) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 34.0), Sense::click());
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    // Right to left: the symbol and the name on the right, the number left.
    let rtl = i18n::is_rtl();
    let at = |from_start: f32| if rtl { rect.right() - from_start } else { rect.left() + from_start };
    let (start, end) = if rtl {
        (egui::Align2::RIGHT_CENTER, egui::Align2::LEFT_CENTER)
    } else {
        (egui::Align2::LEFT_CENTER, egui::Align2::RIGHT_CENTER)
    };
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        if active {
            painter.rect_filled(rect, CornerRadius::same(10), p.tint(p.accent));
            let bar_x = if rtl { rect.right() - 3.0 } else { rect.left() };
            painter.rect_filled(
                egui::Rect::from_min_size(egui::pos2(bar_x, rect.top() + 8.0), Vec2::new(3.0, rect.height() - 16.0)),
                CornerRadius::same(2),
                p.accent,
            );
        } else if resp.hovered() {
            painter.rect_filled(rect, CornerRadius::same(10), p.card_alt);
        }
        let color = if active { p.text } else { p.weak };
        let y = rect.center().y;
        painter.text(
            egui::pos2(at(14.0), y),
            start,
            glyph,
            egui::FontId::proportional(theme::scale::ICON),
            if active { p.accent } else { color },
        );
        painter.text(
            egui::pos2(at(42.0), y),
            start,
            label,
            egui::FontId::new(theme::scale::BODY, egui::FontFamily::Name(theme::SEMIBOLD.into())),
            color,
        );
        if let Some(b) = badge {
            let c = if b == "●" { p.accent } else { p.weak };
            painter.text(
                egui::pos2(at(rect.width() - 12.0), y),
                end,
                b,
                egui::FontId::proportional(theme::scale::CAPTION),
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
            ui.label(RichText::new(tr("Nothing matches the search.")).color(p.weak).size(theme::scale::BODY));
            return;
        }
        if !nothing_yet {
            ui.label(RichText::new(icon::TRAY).size(40.0).color(p.weak));
            ui.label(RichText::new(tr("Nothing here.")).color(p.weak).size(theme::scale::BODY));
            return;
        }
        ui.label(RichText::new(icon::DOWNLOAD_SIMPLE).size(54.0).color(p.accent));
        ui.add_space(6.0);
        ui.label(theme::semibold(tr("Paste a link to start"), theme::scale::TITLE));
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

/// The SHA-256 of `path`, as hex.
fn sha256(
    path: &std::path::Path,
    progress: &crate::jobs::Progress,
    stop: &std::sync::atomic::AtomicBool,
) -> anyhow::Result<String> {
    use sha2::{Digest, Sha256};
    use std::io::Read;
    let mut f = std::fs::File::open(path)?;
    let total = f.metadata().map(|m| m.len()).ok();
    let mut hash = Sha256::new();
    let mut buf = vec![0; 1024 * 1024];
    let mut done = 0u64;
    loop {
        if stop.load(std::sync::atomic::Ordering::Relaxed) {
            anyhow::bail!("cancelled");
        }
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hash.update(&buf[..n]);
        done += n as u64;
        progress.set(done, total);
    }
    Ok(hash.finalize().iter().map(|b| format!("{b:02x}")).collect())
}
