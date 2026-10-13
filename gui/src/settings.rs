//! What the user can choose, kept between runs, and the Settings page.

use std::path::PathBuf;

use eframe::egui::{self, RichText, Ui};
use egui_phosphor::regular as icon;
use serde::{Deserialize, Serialize};
use swiftdm::media::Mode;
use swiftdm::util::format_size;

use crate::helpers::Helpers;
use crate::theme::{self, Accent, Palette, ThemeChoice};
use crate::update::Updater;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    // Appearance
    pub theme: ThemeChoice,
    pub accent: Accent,
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
    // Logins
    pub cookies_from: Option<String>,
    pub auto_cookies: bool,
    // Helpers and updates
    pub keep_ytdlp_updated: bool,
    pub check_updates: bool,
    /// The welcome card was closed.
    pub welcomed: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: ThemeChoice::System,
            accent: Accent::Indigo,
            ui_scale: 1.0,
            download_dir: swiftdm::paths::default_download_dir(),
            max_active: 3,
            connections: 8,
            speed_limit_kb: 0,
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
        }
    }
}

/// Heights offered as video quality.
pub const HEIGHTS: [u32; 7] = [2160, 1440, 1080, 720, 480, 360, 240];
/// MP3 bit rates offered (0: best).
pub const KBPS: [u32; 4] = [0, 320, 192, 128];

pub fn height_label(h: Option<u32>) -> String {
    match h {
        None => "Best".into(),
        Some(2160) => "4K".into(),
        Some(1440) => "2K".into(),
        Some(h) => format!("{h}p"),
    }
}

pub fn kbps_label(k: u32) -> String {
    if k == 0 { "Best".into() } else { format!("{k} kbps") }
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
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.set_width(260.0);
            ui.spacing_mut().item_spacing.y = 1.0;
            ui.label(RichText::new(label).size(14.5));
            if !hint.is_empty() {
                ui.add(egui::Label::new(RichText::new(hint).color(p.weak).size(12.5)).wrap());
            }
        });
        add(ui);
    });
    ui.add_space(8.0);
}

pub fn page(ui: &mut Ui, p: &Palette, s: &mut Settings, helpers: &mut Helpers, updater: &mut Updater) {
    theme::page_title(ui, p, "Settings", "Changes apply right away.");

    theme::card(ui, p, |ui| {
        ui.set_width(ui.available_width());
        theme::section_title(ui, p, icon::DOWNLOAD_SIMPLE, "Downloads");
        row(ui, p, "Save to", "", |ui| {
            ui.vertical(|ui| {
                ui.label(RichText::new(s.download_dir.display().to_string()).color(p.weak));
                ui.horizontal(|ui| {
                    if ui.button(format!("{}  Change…", icon::FOLDER_OPEN)).clicked()
                        && let Some(d) = rfd::FileDialog::new().set_directory(&s.download_dir).pick_folder()
                    {
                        s.download_dir = d;
                    }
                    if ui.button("Open").clicked() {
                        let _ = std::fs::create_dir_all(&s.download_dir);
                        let _ = swiftdm::util::open_path(&s.download_dir);
                    }
                    let default = swiftdm::paths::default_download_dir();
                    if s.download_dir != default && ui.button("Reset").clicked() {
                        s.download_dir = default;
                    }
                });
            });
        });
        row(ui, p, "Downloads at the same time", "The others wait their turn.", |ui| {
            ui.add(egui::Slider::new(&mut s.max_active, 1..=10));
        });
        row(ui, p, "Connections per file", "More connections are faster on most servers.", |ui| {
            ui.add(egui::Slider::new(&mut s.connections, 1..=32));
        });
        row(ui, p, "Speed limit", "Leaves room for others on the network.", |ui| {
            let mut limited = s.speed_limit_kb > 0;
            if ui.checkbox(&mut limited, "Limit to").changed() {
                s.speed_limit_kb = if limited { 2048 } else { 0 };
            }
            if limited {
                ui.add(egui::DragValue::new(&mut s.speed_limit_kb).range(16..=10_000_000).speed(16).suffix(" KB/s"));
                ui.label(RichText::new(format!("= {}/s", format_size(s.speed_limit_kb * 1024))).color(p.weak));
            }
        });
        row(ui, p, "Before downloading", "", |ui| {
            ui.vertical(|ui| {
                ui.checkbox(&mut s.ask_first, "Show the choices (quality, file name, folder)");
                ui.checkbox(&mut s.watch_clipboard, "Offer to download links I copy");
            });
        });
    });
    ui.add_space(12.0);

    theme::card(ui, p, |ui| {
        ui.set_width(ui.available_width());
        theme::section_title(ui, p, icon::FILM_STRIP, "Videos and music");
        row(ui, p, "Download", "What new downloads from video sites keep.", |ui| {
            theme::segmented(ui, p, &mut s.mode, &[(Mode::Video, "Video"), (Mode::Audio, "Sound only (MP3)")]);
        });
        row(ui, p, "Video quality", "The best up to this; less is smaller and faster.", |ui| {
            egui::ComboBox::from_id_salt("quality").selected_text(height_label(s.max_height)).show_ui(ui, |ui| {
                ui.selectable_value(&mut s.max_height, None, "Best");
                for h in HEIGHTS {
                    ui.selectable_value(&mut s.max_height, Some(h), height_label(Some(h)));
                }
            });
        });
        row(ui, p, "MP3 quality", "", |ui| {
            egui::ComboBox::from_id_salt("kbps").selected_text(kbps_label(s.audio_kbps)).show_ui(ui, |ui| {
                for k in KBPS {
                    ui.selectable_value(&mut s.audio_kbps, k, kbps_label(k));
                }
            });
        });
    });
    ui.add_space(12.0);

    theme::card(ui, p, |ui| {
        ui.set_width(ui.available_width());
        theme::section_title(ui, p, icon::KEY, "Logins");
        ui.add(
            egui::Label::new(
                RichText::new(
                    "Some videos are only shown to logged-in users (private Instagram posts, age-restricted YouTube \
                     videos). SwiftDM can use the login of a browser you are logged in with. It is read on this \
                     computer only and never sent anywhere else.",
                )
                .color(p.weak),
            )
            .wrap(),
        );
        ui.add_space(8.0);
        row(ui, p, "Use the login of", "", |ui| {
            let choices = browser_choices();
            let shown = s
                .cookies_from
                .as_deref()
                .and_then(|c| choices.iter().find(|(k, _)| *k == c).map(|(_, n)| *n))
                .unwrap_or("No browser");
            egui::ComboBox::from_id_salt("cookies").selected_text(shown).show_ui(ui, |ui| {
                ui.selectable_value(&mut s.cookies_from, None, "No browser");
                for (key, name) in choices {
                    ui.selectable_value(&mut s.cookies_from, Some(key.to_string()), name);
                }
            });
        });
        row(ui, p, "", "", |ui| {
            ui.checkbox(&mut s.auto_cookies, "When a site refuses, try the logins of my browsers");
        });
        if cfg!(target_os = "macos") {
            ui.label(
                RichText::new(
                    "macOS asks once for permission to read Chrome's login; Safari's needs Full Disk Access.",
                )
                .color(p.weak)
                .size(12.5),
            );
        }
    });
    ui.add_space(12.0);

    theme::card(ui, p, |ui| {
        ui.set_width(ui.available_width());
        theme::section_title(ui, p, icon::PUZZLE_PIECE, "Helpers");
        ui.add(
            egui::Label::new(
                RichText::new(
                    "Free programs SwiftDM uses for video sites. They are downloaded from their official sources and \
                     kept in SwiftDM's own folder.",
                )
                .color(p.weak),
            )
            .wrap(),
        );
        ui.add_space(8.0);
        helpers.table(ui, p);
        ui.add_space(4.0);
        ui.checkbox(&mut s.keep_ytdlp_updated, "Keep yt-dlp up to date (sites change often)");
    });
    ui.add_space(12.0);

    theme::card(ui, p, |ui| {
        ui.set_width(ui.available_width());
        theme::section_title(ui, p, icon::PAINT_BRUSH, "Appearance");
        row(ui, p, "Theme", "", |ui| {
            let opts: Vec<(ThemeChoice, &str)> = ThemeChoice::ALL.iter().map(|t| (*t, t.name())).collect();
            theme::segmented(ui, p, &mut s.theme, &opts);
        });
        row(ui, p, "Colour", "", |ui| {
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
        row(ui, p, "Interface size", "Also Ctrl/⌘ with + and −.", |ui| {
            ui.add(
                egui::Slider::new(&mut s.ui_scale, 0.8..=1.5)
                    .step_by(0.1)
                    .custom_formatter(|v, _| format!("{:.0}%", v * 100.0)),
            );
        });
    });
    ui.add_space(12.0);

    theme::card(ui, p, |ui| {
        ui.set_width(ui.available_width());
        theme::section_title(ui, p, icon::ARROW_CIRCLE_UP, "Updates");
        ui.checkbox(&mut s.check_updates, "Check for a new version of SwiftDM when it starts");
        ui.add_space(4.0);
        updater.status(ui, p);
    });
}
