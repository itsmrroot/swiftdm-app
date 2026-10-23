//! The Help page: a short guide to everything SwiftDM does.

use eframe::egui::{self, CornerRadius, Frame, Margin, RichText, Stroke, Ui};
use egui_phosphor::regular as icon;

use crate::i18n::{icon_label, tr, trl};
use crate::theme::{self, Palette};

/// Something the guide's buttons ask the app for.
pub enum HelpAction {
    None,
    AddToBrowser,
    Settings,
}

fn section(ui: &mut Ui, p: &Palette, glyph: &str, title: &str, open: bool, body: impl FnOnce(&mut Ui)) {
    Frame::new()
        .fill(p.surface())
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(CornerRadius::same(12))
        .inner_margin(Margin::symmetric(16, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            egui::CollapsingHeader::new(
                RichText::new(crate::i18n::icon_text(glyph, title)).size(theme::scale::LEAD).strong(),
            )
            .id_salt(title)
            .default_open(open)
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 6.0;
                body(ui)
            });
        });
    ui.add_space(8.0);
}

fn text(ui: &mut Ui, p: &Palette, t: &str) {
    theme::paragraph(ui, t, 14.5, p.text);
}

fn steps(ui: &mut Ui, p: &Palette, items: &[&str]) {
    for (n, t) in items.iter().enumerate() {
        ui.horizontal_top(|ui| {
            ui.label(theme::semibold(format!("{}.", n + 1), theme::scale::BODY).color(p.accent_text));
            theme::paragraph(ui, t, 14.5, p.text);
        });
    }
}

fn keys(ui: &mut Ui, p: &Palette, rows: &[(&str, &str)]) {
    egui::Grid::new(ui.next_auto_id()).num_columns(2).spacing([24.0, 6.0]).show(ui, |ui| {
        for (k, what) in rows {
            ui.label(RichText::new(*k).monospace().color(p.accent_text));
            ui.label(*what);
            ui.end_row();
        }
    });
}

pub fn page(ui: &mut Ui, p: &Palette) -> HelpAction {
    let mut action = HelpAction::None;
    theme::page_title(ui, p, tr("Help"), tr("How to get the most out of SwiftDM."));
    let m = if cfg!(target_os = "macos") { "⌘" } else { "Ctrl+" };

    section(ui, p, icon::ROCKET_LAUNCH, tr("Getting started"), true, |ui| {
        steps(
            ui,
            p,
            &[
                trl("Copy a link — from YouTube, TikTok, Instagram, any site, or a link to a file."),
                trl(
                    "Paste it into the bar at the top and press Download (or just copy it: SwiftDM offers to download it).",
                ),
                trl("Choose what to keep — the video in the quality you want, or only the sound — and press Download."),
            ],
        );
        text(
            ui,
            p,
            trl(
                "The first time, SwiftDM asks to install its free helpers for video sites. Files from plain links work without them.",
            ),
        );
    });

    section(ui, p, icon::FILM_STRIP, tr("Videos and music"), false, |ui| {
        text(
            ui,
            p,
            trl(
                "Quality: the sizes next to each choice tell how big the file will be. Best is the highest the site offers.",
            ),
        );
        text(
            ui,
            p,
            trl("Sound only saves the music or speech as MP3 (or M4A, Opus, FLAC, WAV) with its cover picture."),
        );
        text(
            ui,
            p,
            trl(
                "Under More choices: subtitles in your languages, cutting out sponsor segments, and downloading only a part of a long video.",
            ),
        );
        text(
            ui,
            p,
            trl(
                "Format: MP4 plays everywhere; MKV keeps the very best quality; WebM is what many sites use themselves.",
            ),
        );
    });

    section(ui, p, icon::PLAYLIST, tr("Playlists and channels"), false, |ui| {
        text(
            ui,
            p,
            trl(
                "Paste the link of a playlist, an album or a channel: SwiftDM lists its items, and you choose which to download. They go into a folder with the playlist's name.",
            ),
        );
    });

    section(ui, p, icon::FILE_ARROW_DOWN, tr("Files from any link"), false, |ui| {
        text(
            ui,
            p,
            trl(
                "Programs, disk images, documents, archives: SwiftDM downloads them over several connections at once, which is much faster on most servers.",
            ),
        );
        text(
            ui,
            p,
            trl(
                "You can pause a download, or close SwiftDM, and continue later from where it stopped. A lost connection is taken up again by itself.",
            ),
        );
        text(ui, p, trl("Several links at once: paste them all, or use Import links from a file in the menu."));
    });

    section(ui, p, icon::BROWSER, tr("In your browser"), false, |ui| {
        text(
            ui,
            p,
            trl(
                "With the SwiftDM extension you download straight from Chrome, Edge, Brave, Opera, Vivaldi or Firefox:",
            ),
        );
        steps(
            ui,
            p,
            &[
                trl("Right-click a link, a video or an image and choose Download with SwiftDM."),
                trl("Point at a video: a Download button appears on it (video or sound)."),
                trl("Downloads the browser starts go to SwiftDM by themselves (you can turn this off)."),
                trl("The extension's button lists the video and audio files the page has loaded."),
            ],
        );
        if theme::primary_button(
            ui,
            p,
            &format!("  {}  ", icon_label(icon::PUZZLE_PIECE, "Add SwiftDM to your browser")),
            true,
        )
        .clicked()
        {
            action = HelpAction::AddToBrowser;
        }
    });

    section(ui, p, icon::KEY, tr("Sites that need a login"), false, |ui| {
        text(
            ui,
            p,
            trl(
                "Private or age-restricted videos need you to be logged in. In Settings → Logins, choose the browser you are logged in with; SwiftDM uses that login. When a site refuses, SwiftDM tries your browsers by itself.",
            ),
        );
        text(
            ui,
            p,
            trl(
                "From the extension, files that need a login (on cloud drives and the like) are downloaded with the browser's login.",
            ),
        );
    });

    section(ui, p, icon::GAUGE, tr("Speed, schedule and power"), false, |ui| {
        text(
            ui,
            p,
            trl(
                "The bar at the top changes the most used settings in one click: the speed limit, how many downloads run at once, and what happens when everything is done (sleep, shut down or quit).",
            ),
        );
        text(
            ui,
            p,
            trl(
                "In Settings → Schedule, downloads can start and pause at set times every day — for example at night. While downloading, the computer does not go to sleep.",
            ),
        );
        if ui.button(icon_label(icon::GEAR_SIX, "Open Settings")).clicked() {
            action = HelpAction::Settings;
        }
    });

    section(ui, p, icon::KEYBOARD, tr("Keyboard shortcuts"), false, |ui| {
        let rows = [
            (format!("{m}N"), tr("New download")),
            (format!("{m}V"), tr("Paste a link anywhere in the window")),
            (format!("{m}O"), tr("Import links from a file…")),
            (format!("{m}1 … {m}5"), tr("All, downloading, completed, paused, failed")),
            (format!("{m}⇧P  /  {m}⇧R"), tr("Pause all / continue all")),
            (format!("{m},"), tr("Settings")),
            (format!("{m}+  /  {m}−"), tr("Interface size")),
            ("F1".to_string(), tr("Help")),
        ];
        let refs: Vec<(&str, &str)> = rows.iter().map(|(k, w)| (k.as_str(), *w)).collect();
        keys(ui, p, &refs);
    });

    section(ui, p, icon::TERMINAL_WINDOW, tr("Command line"), false, |ui| {
        text(ui, p, trl("The installation also brings the swiftdm command, for scripts and the keyboard:"));
        for line in [
            "swiftdm https://youtu.be/…                 # best video",
            "swiftdm -a -k 320 https://…                # MP3, 320 kbps",
            "swiftdm -a -f flac https://…               # FLAC",
            "swiftdm -q 720 -o ~/Videos https://…       # 720p, to a folder",
            "swiftdm --subs en,ar --sponsorblock https://…",
            "swiftdm --section 1:30-2:45 https://…      # only a part",
            "swiftdm -c 16 https://example.com/x.iso    # 16 connections",
            "swiftdm info https://…                     # what is behind a link",
        ] {
            ui.label(RichText::new(line).monospace().size(theme::scale::CAPTION));
        }
    });

    section(ui, p, icon::LIFEBUOY, tr("Problems and solutions"), false, |ui| {
        let faq = [
            (
                tr("A video site stopped working"),
                trl(
                    "Sites change often. Settings → Helpers → Update next to yt-dlp; SwiftDM also does this by itself when it starts.",
                ),
            ),
            (
                tr("YouTube says: sign in to confirm you're not a bot"),
                trl("Choose your browser in Settings → Logins (you must be logged in to YouTube there)."),
            ),
            (tr("MP3 does not work, or only 360p is offered"), trl("FFmpeg is missing: Settings → Helpers → Install.")),
            (
                tr("The extension says SwiftDM is not reachable"),
                trl(
                    "Open SwiftDM once after installing or moving it: it connects itself to your browsers. Then reload the page.",
                ),
            ),
            (
                tr("Downloads are slow"),
                trl(
                    "Check the speed limit in the bar at the top, and try more connections per file in Settings → Downloads.",
                ),
            ),
        ];
        for (q, a) in faq {
            ui.label(theme::semibold(q, theme::scale::BODY));
            theme::paragraph(ui, a, 14.0, p.weak);
            ui.add_space(4.0);
        }
    });
    action
}
