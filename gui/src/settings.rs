//! What the user can choose, kept between runs, and the Settings page.

use std::path::PathBuf;

use eframe::egui::{self, RichText, Ui};
use egui_phosphor::regular as icon;
use serde::{Deserialize, Serialize};
use swiftdm::media::{AudioFormat, Container, Mode};
use swiftdm::util::format_size;

use crate::browser::Browser;
use crate::helpers::Helpers;
use crate::i18n::{Language, icon_label, tr, trf, trl};
use crate::power::{self, AfterDone};
use crate::theme::{self, Accent, Palette, ThemeChoice};
use crate::update::Updater;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    // Appearance
    pub language: Language,
    pub theme: ThemeChoice,
    pub accent: Accent,
    /// Translucent surfaces on a soft colored background.
    pub glass: bool,
    pub ui_scale: f32,
    // Downloads
    pub download_dir: PathBuf,
    pub max_active: usize,
    pub connections: usize,
    /// KB per second; 0 is unlimited.
    pub speed_limit_kb: u64,
    /// Show the choices (quality, folder) before each download.
    pub ask_first: bool,
    /// Offer to download links copied anywhere.
    pub watch_clipboard: bool,
    // Video and music
    pub mode: Mode,
    pub max_height: Option<u32>,
    pub audio_kbps: u32,
    pub audio_format: AudioFormat,
    pub container: Container,
    /// Subtitle languages for videos ("en,ar"); empty: none.
    pub subtitles: String,
    pub sponsorblock: bool,
    // Network
    /// `http://host:port` or `socks5://host:port`; empty: none.
    pub proxy: String,
    /// Files go into Video, Music, Documents, … inside the download folder.
    pub sort_by_kind: bool,
    // When downloads finish, and the schedule
    /// A system notification for each finished download.
    pub notify: bool,
    /// The computer does not go to sleep while downloading.
    pub keep_awake: bool,
    /// Start waiting downloads at this time every day (hour, minute).
    pub start_at: Option<(u8, u8)>,
    /// Pause running downloads at this time every day.
    pub pause_at: Option<(u8, u8)>,
    // Logins
    pub cookies_from: Option<String>,
    pub auto_cookies: bool,
    // Helpers and updates
    pub keep_ytdlp_updated: bool,
    pub check_updates: bool,
    /// The welcome card was closed.
    pub welcomed: bool,
    // More
    /// The browser name to give servers (empty: SwiftDM's own).
    pub user_agent: String,
    /// Downloads caught from the browser start at once, without the window
    /// of choices.
    pub browser_quiet: bool,
    /// A program to run for each finished file: `{file}` is its path.
    pub after_command: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            language: Language::System,
            theme: ThemeChoice::System,
            accent: Accent::Indigo,
            glass: true,
            ui_scale: 1.0,
            download_dir: swiftdm::paths::default_download_dir(),
            max_active: 3,
            connections: 8,
            speed_limit_kb: 0,
            audio_format: AudioFormat::Mp3,
            container: Container::Mp4,
            subtitles: String::new(),
            sponsorblock: false,
            proxy: String::new(),
            sort_by_kind: false,
            notify: true,
            keep_awake: true,
            start_at: None,
            pause_at: None,
            ask_first: true,
            watch_clipboard: true,
            mode: Mode::Video,
            max_height: None,
            audio_kbps: 0,
            cookies_from: None,
            auto_cookies: true,
            keep_ytdlp_updated: true,
            check_updates: true,
            welcomed: false,
            user_agent: String::new(),
            browser_quiet: false,
            after_command: String::new(),
        }
    }
}

impl Settings {
    /// The part the download list runs by.
    pub fn manager(&self) -> swiftdm::manager::Settings {
        swiftdm::manager::Settings {
            max_active: self.max_active.clamp(1, 10),
            connections: self.connections.clamp(1, 32),
            speed_limit: self.speed_limit_kb * 1024,
            cookies_from: self.cookies_from.clone(),
            auto_cookies: self.auto_cookies,
            proxy: Some(self.proxy.trim().to_string()).filter(|p| !p.is_empty()),
            sort_by_kind: self.sort_by_kind,
            user_agent: Some(self.user_agent.trim().to_string()).filter(|u| !u.is_empty()),
        }
    }

    /// Video and music choices for a new download.
    pub fn media_options(&self) -> swiftdm::media::Options {
        swiftdm::media::Options {
            mode: self.mode,
            max_height: self.max_height,
            audio_kbps: self.audio_kbps,
            cookies_from: None,
            audio_format: self.audio_format,
            container: self.container,
            subtitles: self.subtitles.trim().to_string(),
            sponsorblock: self.sponsorblock,
            section: None,
        }
    }
}

/// Subtitle languages to start with: the interface's language and English.
pub fn default_subtitles() -> String {
    match crate::i18n::current() {
        crate::i18n::Lang::Ar => "ar,en".into(),
        crate::i18n::Lang::De => "de,en".into(),
        crate::i18n::Lang::Tr => "tr,en".into(),
        crate::i18n::Lang::En => "en".into(),
    }
}

/// Heights offered as video quality.
pub const HEIGHTS: [u32; 7] = [2160, 1440, 1080, 720, 480, 360, 240];
/// MP3 bit rates offered (0: best).
pub const KBPS: [u32; 4] = [0, 320, 192, 128];

pub fn height_label(h: Option<u32>) -> String {
    match h {
        None => tr("Best").into(),
        Some(2160) => "4K".into(),
        Some(1440) => "2K".into(),
        Some(h) => format!("{h}p"),
    }
}

pub fn kbps_label(k: u32) -> String {
    if k == 0 { tr("Best").into() } else { trf("{n} kbps", &[("n", &k)]) }
}

/// Browsers whose login yt-dlp can read, as (yt-dlp name, shown name).
fn browser_choices() -> Vec<(&'static str, &'static str)> {
    let mut v = vec![("chrome", "Chrome"), ("firefox", "Firefox"), ("edge", "Edge"), ("brave", "Brave")];
    if cfg!(target_os = "macos") {
        v.insert(1, ("safari", "Safari"));
    }
    if cfg!(target_os = "linux") {
        v.push(("chromium", "Chromium"));
    }
    v.extend([("opera", "Opera"), ("vivaldi", "Vivaldi")]);
    v
}

fn row(ui: &mut Ui, p: &Palette, label: &str, hint: &str, add: impl FnOnce(&mut Ui)) {
    // Label and controls start on the same line; every row has the same
    // space below it. Right to left: the label on the right.
    let rtl = crate::i18n::is_rtl();
    let (line, column, controls) = if rtl {
        (
            egui::Layout::right_to_left(egui::Align::Min),
            egui::Layout::top_down(egui::Align::Max),
            egui::Layout::right_to_left(egui::Align::Min).with_main_wrap(true),
        )
    } else {
        (
            egui::Layout::left_to_right(egui::Align::Min),
            egui::Layout::top_down(egui::Align::Min),
            egui::Layout::left_to_right(egui::Align::Min).with_main_wrap(true),
        )
    };
    // The label column gives way when the window is narrow.
    let label_width = (ui.available_width() * 0.4).clamp(130.0, 260.0);
    ui.with_layout(line, |ui| {
        ui.allocate_ui_with_layout(egui::vec2(label_width, 0.0), column, |ui| {
            ui.set_width(label_width);
            ui.spacing_mut().item_spacing.y = 1.0;
            ui.add_space(5.0);
            ui.label(RichText::new(label).size(theme::scale::BODY));
            if !hint.is_empty() {
                theme::paragraph(ui, hint, theme::scale::CAPTION, p.weak);
            }
        });
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 6.0;
            // What does not fit goes to the next line, text included.
            ui.set_max_width(ui.available_width());
            ui.with_layout(controls, |ui| {
                ui.spacing_mut().item_spacing.y = 6.0;
                add(ui);
            });
        });
    });
    ui.add_space(12.0);
}

/// The tabs of the Settings page.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tab {
    #[default]
    General,
    Downloads,
    Videos,
    Browser,
    Schedule,
    Logins,
    Helpers,
}

impl Tab {
    const ALL: [Tab; 7] =
        [Tab::General, Tab::Downloads, Tab::Videos, Tab::Browser, Tab::Schedule, Tab::Logins, Tab::Helpers];

    fn label(self) -> (&'static str, &'static str) {
        match self {
            Tab::General => (icon::SLIDERS_HORIZONTAL, tr("General")),
            Tab::Downloads => (icon::DOWNLOAD_SIMPLE, tr("Downloads")),
            Tab::Videos => (icon::FILM_STRIP, tr("Videos and music")),
            Tab::Browser => (icon::BROWSER, tr("Browser")),
            Tab::Schedule => (icon::CLOCK, tr("Schedule")),
            Tab::Logins => (icon::KEY, tr("Logins")),
            Tab::Helpers => (icon::PUZZLE_PIECE, tr("Helpers")),
        }
    }
}

/// State of the page between frames.
#[derive(Default)]
pub struct Page {
    pub tab: Tab,
    /// The browser whose steps are shown on the Browser tab.
    browser: Option<Browser>,
    /// Typed times of the schedule (`None`: not edited yet).
    start_text: Option<String>,
    pause_text: Option<String>,
    /// The browsers found (looked for once).
    installed: Option<Vec<Browser>>,
    /// Result of the last action on the Browser tab.
    message: Option<(bool, String)>,
    /// Whether SwiftDM starts at sign-in (looked up once), and what went wrong.
    autostart: Option<bool>,
    autostart_error: Option<String>,
}

/// Everything the Settings page changes besides the settings themselves.
pub struct Context<'a> {
    pub helpers: &'a mut Helpers,
    pub updater: &'a mut Updater,
    pub after_done: &'a mut AfterDone,
    /// When the extension last talked to SwiftDM (seconds since 1970).
    pub browser_seen: Option<i64>,
}

pub fn page(ui: &mut Ui, p: &Palette, s: &mut Settings, page: &mut Page, cx: Context) {
    theme::page_title(ui, p, tr("Settings"), tr("Changes apply right away."));
    // The tabs.
    let tabs = if crate::i18n::is_rtl() {
        egui::Layout::right_to_left(egui::Align::Min).with_main_wrap(true)
    } else {
        egui::Layout::left_to_right(egui::Align::Min).with_main_wrap(true)
    };
    ui.with_layout(tabs, |ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        for t in Tab::ALL {
            let (glyph, label) = t.label();
            let on = page.tab == t;
            let text = if on {
                theme::semibold(crate::i18n::icon_text(glyph, label), theme::scale::BODY).color(p.on_accent)
            } else {
                RichText::new(crate::i18n::icon_text(glyph, label)).size(theme::scale::BODY)
            };
            let b = egui::Button::new(text)
                .fill(if on { p.accent_fill } else { p.card })
                .stroke(egui::Stroke::new(1.0, if on { p.accent_fill } else { p.border }))
                .corner_radius(egui::CornerRadius::same(9))
                .min_size(egui::vec2(0.0, 32.0));
            if ui.add(b).clicked() {
                page.tab = t;
            }
        }
    });
    ui.add_space(12.0);
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        // At most 860 wide, and never wider than the window: set_max_width
        // alone would also make it wider than the room there is.
        ui.set_max_width(ui.available_width().min(860.0));
        theme::card(ui, p, |ui| {
            ui.set_width(ui.available_width());
            match page.tab {
                Tab::General => general(ui, p, s, page, cx.updater),
                Tab::Downloads => downloads(ui, p, s),
                Tab::Videos => videos(ui, p, s),
                Tab::Browser => browser(ui, p, s, page, cx.browser_seen),
                Tab::Schedule => schedule(ui, p, s, page, cx.after_done),
                Tab::Logins => logins(ui, p, s),
                Tab::Helpers => {
                    theme::paragraph(
                        ui,
                        trl("Free programs SwiftDM uses for video sites. They are downloaded from their official sources and kept in SwiftDM's own folder."),
                        14.5,
                        p.weak,
                    );
                    ui.add_space(8.0);
                    cx.helpers.table(ui, p);
                    ui.add_space(4.0);
                    ui.checkbox(&mut s.keep_ytdlp_updated, tr("Keep yt-dlp up to date (sites change often)"));
                }
            }
        });
    });
}

fn general(ui: &mut Ui, p: &Palette, s: &mut Settings, page: &mut Page, updater: &mut Updater) {
    row(ui, p, tr("Language"), "", |ui| {
        let current = s.language.label();
        egui::ComboBox::from_id_salt("language").selected_text(current.as_ref()).show_ui(ui, |ui| {
            for l in Language::ALL {
                ui.selectable_value(&mut s.language, l, l.label().as_ref());
            }
        });
    });
    row(ui, p, tr("Theme"), "", |ui| {
        let opts: Vec<(ThemeChoice, &str)> = ThemeChoice::ALL.iter().map(|t| (*t, t.name())).collect();
        theme::segmented(ui, p, &mut s.theme, &opts);
    });
    row(ui, p, tr("Color"), "", |ui| {
        for a in Accent::ALL {
            let selected = s.accent == a;
            let (rect, resp) = ui.allocate_exact_size(egui::vec2(28.0, 28.0), egui::Sense::click());
            ui.painter().circle_filled(rect.center(), 11.0, a.color());
            if selected {
                ui.painter().circle_stroke(rect.center(), 14.0, egui::Stroke::new(2.0, p.text));
            }
            if resp.on_hover_text(a.name()).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                s.accent = a;
            }
        }
    });
    row(ui, p, tr("Glass look"), "", |ui| {
        ui.checkbox(&mut s.glass, tr("Frosted surfaces on a soft background"));
    });
    row(ui, p, tr("Interface size"), trl("Also Ctrl/⌘ with + and −."), |ui| {
        ui.add(
            egui::Slider::new(&mut s.ui_scale, 0.8..=1.5)
                .step_by(0.1)
                .custom_formatter(|v, _| format!("{:.0}%", v * 100.0)),
        );
    });
    row(ui, p, tr("Before downloading"), "", |ui| {
        ui.vertical(|ui| {
            ui.checkbox(&mut s.ask_first, tr("Show the choices (quality, file name, folder)"));
            ui.checkbox(&mut s.watch_clipboard, tr("Offer to download links I copy"));
        });
    });
    row(ui, p, tr("When a download finishes"), "", |ui| {
        ui.checkbox(&mut s.notify, tr("Show a notification of the system"));
    });
    row(ui, p, tr("Startup"), "", |ui| {
        ui.vertical(|ui| {
            let on = *page.autostart.get_or_insert_with(crate::autostart::is_on);
            let mut want = on;
            if ui.checkbox(&mut want, tr("Start SwiftDM when I sign in")).changed() {
                match crate::autostart::set(want) {
                    Ok(()) => {
                        page.autostart = Some(want);
                        page.autostart_error = None;
                    }
                    Err(e) => page.autostart_error = Some(crate::i18n::engine(&format!("{e:#}"))),
                }
            }
            if let Some(e) = &page.autostart_error {
                ui.label(crate::text::rich(e).color(p.danger).size(theme::scale::CAPTION));
            }
        });
    });
    row(ui, p, tr("Updates"), "", |ui| {
        ui.vertical(|ui| {
            ui.checkbox(&mut s.check_updates, tr("Check for a new version of SwiftDM when it starts"));
            updater.status(ui, p);
        });
    });
}

fn downloads(ui: &mut Ui, p: &Palette, s: &mut Settings) {
    row(ui, p, tr("Save to"), "", |ui| {
        ui.vertical(|ui| {
            ui.label(RichText::new(s.download_dir.display().to_string()).color(p.weak));
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
                let default = swiftdm::paths::default_download_dir();
                if s.download_dir != default && ui.button(tr("Reset")).clicked() {
                    s.download_dir = default;
                }
            });
            ui.checkbox(&mut s.sort_by_kind, tr("Sort into folders: Video, Music, Documents, Programs, …"));
        });
    });
    row(ui, p, tr("Downloads at the same time"), trl("The others wait their turn."), |ui| {
        ui.add(egui::Slider::new(&mut s.max_active, 1..=10));
    });
    row(ui, p, tr("Connections per file"), trl("More connections are faster on most servers."), |ui| {
        ui.add(egui::Slider::new(&mut s.connections, 1..=32));
    });
    row(ui, p, tr("Speed limit"), trl("Leaves room for others on the network."), |ui| {
        let mut limited = s.speed_limit_kb > 0;
        if ui.checkbox(&mut limited, tr("Limit to")).changed() {
            s.speed_limit_kb = if limited { 2048 } else { 0 };
        }
        if limited {
            ui.add(egui::DragValue::new(&mut s.speed_limit_kb).range(16..=10_000_000).speed(16).suffix(" KB/s"));
            ui.label(RichText::new(format!("= {}/s", format_size(s.speed_limit_kb * 1024))).color(p.weak));
        }
    });
    row(ui, p, tr("Proxy"), trl("For networks that need one, or for privacy. Empty: none."), |ui| {
        ui.add(egui::TextEdit::singleline(&mut s.proxy).hint_text("socks5://127.0.0.1:9050").desired_width(260.0));
    });
    row(
        ui,
        p,
        tr("After each download"),
        trl("A program to run for each finished file, for example a virus scan. {file} is the file's path."),
        |ui| {
            ui.add(
                egui::TextEdit::singleline(&mut s.after_command)
                    .hint_text(r#""C:\Tools\scan.exe" {file}"#)
                    .desired_width(300.0),
            );
        },
    );
    row(
        ui,
        p,
        tr("Browser name"),
        trl("What servers are told SwiftDM is (the user agent). Empty: SwiftDM's own."),
        |ui| {
            ui.add(egui::TextEdit::singleline(&mut s.user_agent).hint_text("Mozilla/5.0 …").desired_width(300.0));
        },
    );
}

fn videos(ui: &mut Ui, p: &Palette, s: &mut Settings) {
    row(ui, p, tr("Download"), trl("What new downloads from video sites keep."), |ui| {
        theme::segmented(ui, p, &mut s.mode, &[(Mode::Video, tr("Video")), (Mode::Audio, tr("Sound only"))]);
    });
    row(ui, p, tr("Video quality"), trl("The best up to this; less is smaller and faster."), |ui| {
        egui::ComboBox::from_id_salt("quality").selected_text(height_label(s.max_height)).show_ui(ui, |ui| {
            ui.selectable_value(&mut s.max_height, None, tr("Best"));
            for h in HEIGHTS {
                ui.selectable_value(&mut s.max_height, Some(h), height_label(Some(h)));
            }
        });
    });
    row(ui, p, tr("Video format"), trl("MP4 plays everywhere; MKV keeps the very best quality."), |ui| {
        for c in Container::ALL {
            ui.selectable_value(&mut s.container, c, c.name().to_uppercase());
        }
    });
    row(ui, p, tr("Sound format"), "", |ui| {
        for f in AudioFormat::ALL {
            ui.selectable_value(&mut s.audio_format, f, f.name().to_uppercase());
        }
    });
    if s.audio_format.lossy() {
        row(ui, p, tr("Sound quality"), "", |ui| {
            egui::ComboBox::from_id_salt("kbps").selected_text(kbps_label(s.audio_kbps)).show_ui(ui, |ui| {
                for k in KBPS {
                    ui.selectable_value(&mut s.audio_kbps, k, kbps_label(k));
                }
            });
        });
    }
    row(ui, p, tr("Subtitles"), trl("Languages, e.g. en,ar — or all. Empty: none."), |ui| {
        ui.add(egui::TextEdit::singleline(&mut s.subtitles).hint_text(default_subtitles()).desired_width(160.0));
    });
    row(ui, p, tr("Sponsors"), "", |ui| {
        ui.checkbox(&mut s.sponsorblock, tr("Cut out sponsor segments (SponsorBlock)"));
    });
}

fn browser(ui: &mut Ui, p: &Palette, s: &mut Settings, page: &mut Page, seen: Option<i64>) {
    theme::paragraph(
        ui,
        trl(
            "With the extension, you download from the browser itself: right-click links and videos, a button on every video, and the browser's own downloads go to SwiftDM.",
        ),
        14.5,
        p.text,
    );
    ui.add_space(6.0);
    row(ui, p, tr("Downloads from the browser"), "", |ui| {
        ui.checkbox(&mut s.browser_quiet, tr("Start them at once, without the window of choices"));
    });
    match seen {
        Some(t) => {
            let when = crate::list::date(t);
            theme::notice(
                ui,
                p,
                p.success,
                icon::CHECK_CIRCLE,
                &crate::i18n::trlf("The extension is connected (last used {when}).", &[("when", &when)]),
            );
        }
        None => theme::notice(
            ui,
            p,
            p.accent,
            icon::INFO,
            trl("Not connected yet: add the extension to your browser below."),
        ),
    }
    ui.add_space(10.0);
    let installed =
        page.installed.get_or_insert_with(|| Browser::ALL.into_iter().filter(|b| b.installed()).collect()).clone();
    if installed.is_empty() {
        theme::paragraph(ui, trl("No supported browser was found on this computer."), 14.5, p.weak);
        return;
    }
    ui.horizontal_wrapped(|ui| {
        for b in &installed {
            if ui.selectable_label(page.browser == Some(*b), b.name()).clicked() {
                page.browser = Some(*b);
                page.message = None;
            }
        }
    });
    let Some(b) = page.browser.filter(|b| installed.contains(b)).or(installed.first().copied()) else { return };
    page.browser = Some(b);
    ui.add_space(8.0);
    let num = |ui: &mut Ui, n: usize, text: &str| {
        ui.horizontal_top(|ui| {
            ui.label(theme::semibold(format!("{n}."), theme::scale::BODY).color(p.accent_text));
            theme::paragraph(ui, text, 14.5, p.text);
        });
    };
    if b.is_firefox() {
        num(ui, 1, trl("Click Show the extension: its folder opens."));
        num(
            ui,
            2,
            trl(
                "In Firefox, open the add-ons debugging page (Open the page below), click Load Temporary Add-on and choose manifest.json in that folder.",
            ),
        );
        theme::paragraph(
            ui,
            trl(
                "Firefox removes add-ons loaded this way when it closes, until SwiftDM is in the Firefox Add-ons store.",
            ),
            13.0,
            p.weak,
        );
    } else {
        num(ui, 1, trl("Click Show the extension: its folder opens."));
        num(ui, 2, trl("Click Open the extensions page, and turn on Developer mode there (top right)."));
        num(
            ui,
            3,
            trl(
                "Click Load unpacked and choose the folder from step 1. The SwiftDM button appears in the toolbar (pin it with the puzzle icon).",
            ),
        );
    }
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        if ui.button(icon_label(icon::FOLDER_OPEN, "Show the extension")).clicked() {
            page.message = Some(match crate::browser::write_extension(b.is_firefox()) {
                Ok(dir) => {
                    let _ = swiftdm::util::open_path(&dir);
                    (true, dir.display().to_string())
                }
                Err(e) => (false, crate::i18n::engine(&format!("{e:#}"))),
            });
        }
        // To paste into the browser's folder picker.
        if ui.button(icon_label(icon::COPY, "Copy the path")).clicked() {
            page.message = Some(match crate::browser::write_extension(b.is_firefox()) {
                Ok(dir) => {
                    ui.ctx().copy_text(dir.display().to_string());
                    (true, crate::i18n::trlf("Copied: {path}", &[("path", &dir.display())]))
                }
                Err(e) => (false, crate::i18n::engine(&format!("{e:#}"))),
            });
        }
        let page_label = if b.is_firefox() {
            icon_label(icon::ARROW_SQUARE_OUT, "Open the page")
        } else {
            icon_label(icon::ARROW_SQUARE_OUT, "Open the extensions page")
        };
        if ui.button(page_label).clicked()
            && let Err(e) = b.open(b.extensions_page())
        {
            page.message = Some((false, format!("{e:#}")));
        }
    });
    if let Some((ok, msg)) = &page.message {
        ui.add_space(4.0);
        ui.label(crate::text::rich(msg).color(if *ok { p.weak } else { p.danger }).size(theme::scale::CAPTION));
    }
}

fn schedule(ui: &mut Ui, p: &Palette, s: &mut Settings, page: &mut Page, after_done: &mut AfterDone) {
    let time_row = |ui: &mut Ui, label: &str, hint: &str, value: &mut Option<(u8, u8)>, text: &mut Option<String>| {
        row(ui, p, label, hint, |ui| {
            let mut on = value.is_some();
            if ui.checkbox(&mut on, tr("Every day at")).changed() {
                let typed = text.as_deref().and_then(power::parse_hhmm);
                *value = if on { Some(typed.unwrap_or((1, 0))) } else { None };
                *text = None;
            }
            if let Some(v) = *value {
                // What is typed stays while it is being typed (also nothing).
                let text = text.get_or_insert_with(|| power::hhmm(v));
                let resp = ui.add(egui::TextEdit::singleline(text).desired_width(60.0).hint_text("01:00"));
                if resp.changed()
                    && let Some(v) = power::parse_hhmm(text)
                {
                    *value = Some(v);
                }
                if power::parse_hhmm(text).is_none() {
                    ui.label(RichText::new(icon::WARNING).color(p.warning)).on_hover_text("HH:MM");
                }
            }
        });
    };
    time_row(
        ui,
        tr("Start downloads"),
        trl("Waiting and paused downloads start, e.g. at night."),
        &mut s.start_at,
        &mut page.start_text,
    );
    time_row(
        ui,
        tr("Pause downloads"),
        trl("Running downloads pause, e.g. in the morning."),
        &mut s.pause_at,
        &mut page.pause_text,
    );
    row(ui, p, tr("When downloads are done"), trl("Only this time; afterwards it is Do nothing again."), |ui| {
        egui::ComboBox::from_id_salt("after").selected_text(after_done.name()).show_ui(ui, |ui| {
            for a in AfterDone::ALL {
                ui.selectable_value(after_done, a, a.name());
            }
        });
    });
    row(ui, p, tr("Sleep"), "", |ui| {
        ui.checkbox(&mut s.keep_awake, tr("Keep the computer awake while downloading"));
    });
}

fn logins(ui: &mut Ui, p: &Palette, s: &mut Settings) {
    theme::paragraph(
        ui,
        trl(
            "Some videos are only shown to logged-in users (private Instagram posts, age-restricted YouTube videos). SwiftDM can use the login of a browser you are logged in with. It is read on this computer only and never sent anywhere else.",
        ),
        14.5,
        p.weak,
    );
    ui.add_space(8.0);
    row(ui, p, tr("Use the login of"), "", |ui| {
        let choices = browser_choices();
        let shown = s
            .cookies_from
            .as_deref()
            .and_then(|c| choices.iter().find(|(k, _)| *k == c).map(|(_, n)| *n))
            .unwrap_or(tr("No browser"));
        egui::ComboBox::from_id_salt("cookies").selected_text(shown).show_ui(ui, |ui| {
            ui.selectable_value(&mut s.cookies_from, None, tr("No browser"));
            for (key, name) in choices {
                ui.selectable_value(&mut s.cookies_from, Some(key.to_string()), name);
            }
        });
    });
    row(ui, p, "", "", |ui| {
        ui.checkbox(&mut s.auto_cookies, tr("When a site refuses, try the logins of my browsers"));
    });
    if cfg!(target_os = "macos") {
        theme::paragraph(
            ui,
            trl("macOS asks once for permission to read Chrome's login; Safari's needs Full Disk Access."),
            12.5,
            p.weak,
        );
    }
}
