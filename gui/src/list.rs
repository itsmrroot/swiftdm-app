//! The download list: one row per download, with its progress and actions.

use eframe::egui::{self, Align, Color32, CornerRadius, Frame, Layout, Margin, RichText, Sense, Stroke, Ui, Vec2};
use egui_phosphor::regular as icon;
use swiftdm::manager::{Item, Status};
use swiftdm::util::{FileKind, format_eta, format_size, format_speed, host};

use crate::text;
use crate::theme::{self, Palette};
use crate::thumbs::Thumbs;

pub const ROW_HEIGHT: f32 = 78.0;

pub fn kind_icon(kind: FileKind) -> &'static str {
    match kind {
        FileKind::Video => icon::FILM_STRIP,
        FileKind::Audio => icon::MUSIC_NOTES,
        FileKind::Image => icon::IMAGE,
        FileKind::Archive => icon::FILE_ZIP,
        FileKind::Program => icon::APP_WINDOW,
        FileKind::Document => icon::FILE_TEXT,
        FileKind::Other => icon::FILE,
    }
}

pub fn kind_color(p: &Palette, kind: FileKind) -> Color32 {
    match kind {
        FileKind::Video => p.accent,
        FileKind::Audio => p.purple,
        FileKind::Image => Color32::from_rgb(14, 165, 233),
        FileKind::Archive => p.warning,
        FileKind::Program => p.success,
        FileKind::Document => Color32::from_rgb(100, 116, 139),
        FileKind::Other => p.weak,
    }
}

/// What the user did on a row.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RowAction {
    Pause(u64),
    Resume(u64),
    Restart(u64),
    Open(u64),
    Reveal(u64),
    CopyLink(u64),
    OpenLink(u64),
    Remove(u64),
    Delete(u64),
}

fn date(secs: i64) -> String {
    let ago = swiftdm::util::now() - secs;
    match ago {
        i64::MIN..=59 => "just now".into(),
        60..=3599 => format!("{} min ago", ago / 60),
        3600..=86_399 => format!("{} h ago", ago / 3600),
        86_400..=172_799 => "yesterday".into(),
        _ => format!("{} days ago", ago / 86_400),
    }
}

/// One row.
pub fn row(ui: &mut Ui, p: &Palette, item: &Item, thumbs: &mut Thumbs) -> Option<RowAction> {
    let mut action = None;
    let kind = item.kind();
    let width = ui.available_width();
    let resp = Frame::new()
        .fill(p.card)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(CornerRadius::same(12))
        .inner_margin(Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.set_width(width - 26.0);
            ui.set_height(ROW_HEIGHT - 28.0);
            ui.horizontal_centered(|ui| {
                // Thumbnail or type icon.
                let size = Vec2::new(84.0, 48.0);
                let tex = item.thumbnail.as_deref().and_then(|u| thumbs.get(ui.ctx(), u));
                match tex {
                    Some(t) => {
                        ui.add(egui::Image::new(&t).fit_to_exact_size(size).corner_radius(CornerRadius::same(6)));
                    }
                    None => {
                        let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
                        let c = kind_color(p, kind);
                        ui.painter().rect_filled(rect, CornerRadius::same(8), p.tint(c));
                        ui.painter().text(
                            rect.center(),
                            egui::Align2::CENTER_CENTER,
                            kind_icon(kind),
                            egui::FontId::proportional(24.0),
                            c,
                        );
                    }
                }
                ui.add_space(4.0);
                // Actions on the right, the text in between.
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    action = actions(ui, p, item).or(action);
                    ui.add_space(8.0);
                    ui.with_layout(Layout::top_down(Align::Min), |ui| {
                        ui.spacing_mut().item_spacing.y = 4.0;
                        ui.add(egui::Label::new(text::rich(item.display_name()).size(14.5).strong()).truncate());
                        status_line(ui, p, item);
                        if item.status != Status::Done {
                            let color = match item.status {
                                Status::Failed => p.danger,
                                Status::Paused | Status::Queued => p.weak,
                                _ => p.accent,
                            };
                            let fraction = if item.status == Status::Active && item.stage.is_some() {
                                None
                            } else {
                                item.fraction()
                            };
                            let fraction =
                                if item.status == Status::Active { fraction } else { fraction.or(Some(0.0)) };
                            theme::bar(ui, p, fraction, color, ui.available_width());
                        }
                    });
                });
            });
        })
        .response;
    let resp = ui.interact(resp.rect, egui::Id::new(("row", item.id)), Sense::click());
    if resp.double_clicked() && item.status == Status::Done {
        action = Some(RowAction::Open(item.id));
    }
    resp.context_menu(|ui| {
        if let Some(a) = menu(ui, item) {
            action = Some(a);
            ui.close();
        }
    });
    action
}

fn status_line(ui: &mut Ui, p: &Palette, item: &Item) {
    let size = |done: u64| match item.total {
        Some(t) => format!("{} of {}", format_size(done), format_size(t)),
        None => format_size(done),
    };
    let (text, color) = match item.status {
        Status::Active => match &item.stage {
            Some(stage) => (format!("{stage}…"), p.accent),
            None if item.done == 0 => ("Starting…".into(), p.weak),
            None => {
                let mut parts = vec![size(item.done)];
                if item.speed > 0.0 {
                    parts.push(format_speed(item.speed));
                }
                if let Some(e) = item.eta {
                    parts.push(format!("{} left", format_eta(e)));
                }
                (parts.join("  ·  "), p.weak)
            }
        },
        Status::Queued => ("Waiting for its turn".into(), p.weak),
        Status::Paused => (format!("Paused  ·  {}", size(item.done)), p.warning),
        Status::Failed => (item.error.clone().unwrap_or_else(|| "Failed".into()), p.danger),
        Status::Done => {
            let mut parts = Vec::new();
            if let Some(t) = item.total {
                parts.push(format_size(t));
            }
            parts.push(host(&item.url).to_string());
            if let Some(f) = item.finished {
                parts.push(date(f));
            }
            (parts.join("  ·  "), p.weak)
        }
    };
    ui.add(egui::Label::new(RichText::new(text).size(12.5).color(color)).truncate());
}

fn actions(ui: &mut Ui, p: &Palette, item: &Item) -> Option<RowAction> {
    let id = item.id;
    let mut action = None;
    ui.menu_button(RichText::new(icon::DOTS_THREE_VERTICAL).size(18.0).color(p.weak), |ui| {
        action = menu(ui, item);
        if action.is_some() {
            ui.close();
        }
    });
    match item.status {
        Status::Active | Status::Queued => {
            if theme::icon_button(ui, p, icon::PAUSE, "Pause", None).clicked() {
                action = Some(RowAction::Pause(id));
            }
        }
        Status::Paused => {
            if theme::icon_button(ui, p, icon::PLAY, "Continue", Some(p.accent)).clicked() {
                action = Some(RowAction::Resume(id));
            }
        }
        Status::Failed => {
            if theme::icon_button(ui, p, icon::ARROW_CLOCKWISE, "Try again", Some(p.accent)).clicked() {
                action = Some(RowAction::Resume(id));
            }
        }
        Status::Done => {
            if theme::icon_button(ui, p, icon::FOLDER_OPEN, "Show in folder", None).clicked() {
                action = Some(RowAction::Reveal(id));
            }
            if theme::icon_button(ui, p, icon::PLAY_CIRCLE, "Open", Some(p.accent)).clicked() {
                action = Some(RowAction::Open(id));
            }
        }
    }
    if item.status == Status::Active {
        ui.add_space(6.0);
        let pct = item.fraction().map(|f| format!("{:.0}%", f * 100.0)).unwrap_or_default();
        ui.label(theme::semibold(pct, 14.0).color(p.text));
    }
    action
}

fn menu(ui: &mut Ui, item: &Item) -> Option<RowAction> {
    let id = item.id;
    let mut a = None;
    let mut entry = |ui: &mut Ui, glyph: &str, label: &str, act: RowAction| {
        if ui.button(format!("{glyph}  {label}")).clicked() {
            a = Some(act);
        }
    };
    if item.status == Status::Done {
        entry(ui, icon::PLAY_CIRCLE, "Open", RowAction::Open(id));
        entry(ui, icon::FOLDER_OPEN, "Show in folder", RowAction::Reveal(id));
        ui.separator();
    }
    entry(ui, icon::COPY, "Copy link", RowAction::CopyLink(id));
    entry(ui, icon::GLOBE, "Open link in browser", RowAction::OpenLink(id));
    entry(ui, icon::ARROW_COUNTER_CLOCKWISE, "Download again", RowAction::Restart(id));
    ui.separator();
    entry(ui, icon::X, "Remove from list", RowAction::Remove(id));
    if item.status == Status::Done {
        entry(ui, icon::TRASH, "Delete file…", RowAction::Delete(id));
    }
    a
}

/// The four numbers above the list.
pub fn stats(ui: &mut Ui, p: &Palette, items: &[Item], speed: f64) {
    let active = items.iter().filter(|i| i.status == Status::Active).count();
    let done: Vec<&Item> = items.iter().filter(|i| i.status == Status::Done).collect();
    let bytes: u64 = done.iter().filter_map(|i| i.total).sum();
    let cards = [
        (icon::DOWNLOAD_SIMPLE, "Downloading", active.to_string(), p.accent),
        (icon::LIGHTNING, "Speed", format_speed(speed), p.warning),
        (icon::CHECK_CIRCLE, "Completed", done.len().to_string(), p.success),
        (icon::DATABASE, "Downloaded", format_size(bytes), p.purple),
    ];
    let gap = 10.0;
    let w = ((ui.available_width() - gap * 3.0) / 4.0).max(120.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = gap;
        for (glyph, label, value, color) in cards {
            Frame::new()
                .fill(p.card)
                .stroke(Stroke::new(1.0, p.border))
                .corner_radius(CornerRadius::same(12))
                .inner_margin(Margin::symmetric(12, 10))
                .show(ui, |ui| {
                    ui.set_width(w - 26.0);
                    ui.horizontal(|ui| {
                        let (rect, _) = ui.allocate_exact_size(Vec2::splat(34.0), Sense::hover());
                        ui.painter().rect_filled(rect, CornerRadius::same(9), p.tint(color));
                        ui.painter().text(
                            rect.center(),
                            egui::Align2::CENTER_CENTER,
                            glyph,
                            egui::FontId::proportional(18.0),
                            color,
                        );
                        ui.vertical(|ui| {
                            ui.spacing_mut().item_spacing.y = 0.0;
                            ui.label(theme::semibold(value, 16.5));
                            ui.label(RichText::new(label).size(12.0).color(p.weak));
                        });
                    });
                });
        }
    });
}
