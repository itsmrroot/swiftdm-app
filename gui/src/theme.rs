//! Look and feel: palettes, fonts, egui style, and small reusable widgets.

use eframe::egui::{
    self, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Frame, InnerResponse, Margin, Response,
    RichText, Shadow, Stroke, TextStyle, Theme, Ui, Vec2, Visuals,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use swiftdm::http::Resume;

use crate::{i18n, text};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ThemeChoice {
    System,
    Light,
    Dark,
    /// Near-black, as in the first SwiftDM.
    Night,
}

impl ThemeChoice {
    pub const ALL: [ThemeChoice; 4] = [ThemeChoice::System, ThemeChoice::Light, ThemeChoice::Dark, ThemeChoice::Night];

    pub fn preference(self) -> egui::ThemePreference {
        match self {
            ThemeChoice::System => egui::ThemePreference::System,
            ThemeChoice::Light => egui::ThemePreference::Light,
            ThemeChoice::Dark | ThemeChoice::Night => egui::ThemePreference::Dark,
        }
    }

    /// The name in logical order, for menus the system draws (macOS).
    #[cfg(target_os = "macos")]
    pub fn name_logical(self) -> &'static str {
        match self {
            ThemeChoice::System => i18n::trl("Automatic"),
            ThemeChoice::Light => i18n::trl("Light"),
            ThemeChoice::Dark => i18n::trl("Dark"),
            ThemeChoice::Night => i18n::trl("Night"),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            ThemeChoice::System => i18n::tr("Automatic"),
            ThemeChoice::Light => i18n::tr("Light"),
            ThemeChoice::Dark => i18n::tr("Dark"),
            ThemeChoice::Night => i18n::tr("Night"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Accent {
    Indigo,
    Blue,
    Violet,
    Emerald,
    Orange,
    Rose,
}

impl Accent {
    pub const ALL: [Accent; 6] =
        [Accent::Indigo, Accent::Blue, Accent::Violet, Accent::Emerald, Accent::Orange, Accent::Rose];

    pub fn color(self) -> Color32 {
        match self {
            Accent::Indigo => Color32::from_rgb(99, 102, 241),
            Accent::Blue => Color32::from_rgb(59, 130, 246),
            Accent::Violet => Color32::from_rgb(139, 92, 246),
            Accent::Emerald => Color32::from_rgb(16, 185, 129),
            Accent::Orange => Color32::from_rgb(249, 115, 22),
            Accent::Rose => Color32::from_rgb(244, 63, 94),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Accent::Indigo => i18n::tr("Indigo"),
            Accent::Blue => i18n::tr("Blue"),
            Accent::Violet => i18n::tr("Violet"),
            Accent::Emerald => i18n::tr("Emerald"),
            Accent::Orange => i18n::tr("Orange"),
            Accent::Rose => i18n::tr("Rose"),
        }
    }
}

/// Whether surfaces are frosted (translucent over a soft backdrop): the
/// setting, read when a [`Palette`] is made.
static GLASS: AtomicBool = AtomicBool::new(true);

pub fn set_glass(on: bool) {
    GLASS.store(on, Ordering::Relaxed);
}

/// Colours for the current theme.
#[derive(Clone, Copy)]
pub struct Palette {
    pub dark: bool,
    /// Frosted surfaces (see [`Palette::surface`]).
    pub glass: bool,
    pub bg: Color32,
    pub sidebar: Color32,
    pub card: Color32,
    pub card_alt: Color32,
    pub border: Color32,
    /// The edge of buttons and fields: a little stronger than the cards'.
    pub control_border: Color32,
    pub text: Color32,
    pub weak: Color32,
    pub accent: Color32,
    /// The accent as the fill of a button, and the text on it: always at
    /// least 4.5:1 (see [`solid`]).
    pub accent_fill: Color32,
    pub on_accent: Color32,
    /// The accent as text on cards: lightened (dark themes) or darkened
    /// (light theme) to be at least 4.5:1.
    pub accent_text: Color32,
    /// Danger as the fill of a button, and the text on it.
    pub danger_fill: Color32,
    pub on_danger: Color32,
    pub success: Color32,
    pub warning: Color32,
    pub danger: Color32,
    pub purple: Color32,
}

impl Palette {
    /// `night` only applies to the dark palette.
    pub fn new(dark: bool, night: bool, accent: Accent) -> Self {
        let mut p = Self::base(dark, night, accent);
        p.glass = GLASS.load(Ordering::Relaxed);
        let (accent_fill, on_accent) = solid(p.accent);
        let (danger_fill, on_danger) = solid(p.danger);
        p.accent_fill = accent_fill;
        p.on_accent = on_accent;
        p.danger_fill = danger_fill;
        p.on_danger = on_danger;
        // Against the least contrasting surface (cards, the page, raised parts).
        p.accent_text = readable_on(p.accent, p.card_alt, dark);
        p
    }

    fn base(dark: bool, night: bool, accent: Accent) -> Self {
        let accent = accent.color();
        let unset = Color32::TRANSPARENT;
        if dark && night {
            Self {
                dark,
                glass: false,
                bg: Color32::from_rgb(2, 2, 8),
                sidebar: Color32::from_rgb(5, 5, 16),
                card: Color32::from_rgb(8, 8, 26),
                card_alt: Color32::from_rgb(13, 13, 34),
                border: Color32::from_rgb(24, 24, 62),
                control_border: Color32::from_rgb(46, 46, 96),
                text: Color32::from_rgb(214, 214, 255),
                weak: Color32::from_rgb(126, 126, 188),
                accent,
                accent_fill: unset,
                on_accent: unset,
                accent_text: unset,
                danger_fill: unset,
                on_danger: unset,
                success: Color32::from_rgb(16, 185, 129),
                warning: Color32::from_rgb(245, 158, 11),
                danger: Color32::from_rgb(248, 113, 113),
                purple: Color32::from_rgb(168, 85, 247),
            }
        } else if dark {
            Self {
                dark,
                glass: false,
                bg: Color32::from_rgb(7, 7, 14),
                sidebar: Color32::from_rgb(12, 12, 24),
                card: Color32::from_rgb(16, 16, 34),
                card_alt: Color32::from_rgb(22, 22, 48),
                border: Color32::from_rgb(34, 34, 60),
                control_border: Color32::from_rgb(56, 56, 94),
                text: Color32::from_rgb(234, 234, 248),
                weak: Color32::from_rgb(136, 136, 176),
                accent,
                accent_fill: unset,
                on_accent: unset,
                accent_text: unset,
                danger_fill: unset,
                on_danger: unset,
                success: Color32::from_rgb(34, 197, 94),
                warning: Color32::from_rgb(245, 158, 11),
                danger: Color32::from_rgb(248, 113, 113),
                purple: Color32::from_rgb(168, 85, 247),
            }
        } else {
            Self {
                dark,
                glass: false,
                bg: Color32::from_rgb(244, 244, 248),
                sidebar: Color32::from_rgb(255, 255, 255),
                card: Color32::from_rgb(255, 255, 255),
                card_alt: Color32::from_rgb(238, 238, 245),
                border: Color32::from_rgb(203, 203, 219),
                control_border: Color32::from_rgb(168, 168, 192),
                text: Color32::from_rgb(24, 24, 42),
                weak: Color32::from_rgb(96, 96, 140),
                accent,
                accent_fill: unset,
                on_accent: unset,
                accent_text: unset,
                danger_fill: unset,
                on_danger: unset,
                success: Color32::from_rgb(20, 112, 52),
                warning: Color32::from_rgb(165, 75, 8),
                danger: Color32::from_rgb(190, 30, 30),
                purple: Color32::from_rgb(147, 51, 234),
            }
        }
    }

    /// `c` as a frosted surface: mostly opaque, so that text on it keeps
    /// its contrast, with the backdrop showing through a little.
    pub fn frost(&self, c: Color32, alpha: f32) -> Color32 {
        if self.glass { Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), (alpha * 255.0).round() as u8) } else { c }
    }

    /// Cards.
    pub fn surface(&self) -> Color32 {
        self.frost(self.card, 0.80)
    }

    /// The bars (header, sidebar).
    pub fn bar(&self) -> Color32 {
        self.frost(self.sidebar, 0.78)
    }

    /// The page behind the cards: see through to the backdrop when frosted.
    pub fn page(&self) -> Color32 {
        if self.glass { Color32::TRANSPARENT } else { self.bg }
    }

    /// A soft background tint of `c` that works on cards.
    pub fn tint(&self, c: Color32) -> Color32 {
        c.gamma_multiply(if self.dark { 0.16 } else { 0.10 })
    }
}

/// How strong the backdrop's glow is at its corners (of 255): the accent
/// at the top left, the second color at the top right.
fn glow_alphas(dark: bool) -> (u8, u8) {
    if dark { (72, 34) } else { (52, 26) }
}

/// The soft background that frosted surfaces lie on: the accent and a
/// second color fading across the window.
pub fn paint_backdrop(painter: &egui::Painter, rect: egui::Rect, p: &Palette, accent: Color32) {
    if !p.glass {
        return;
    }
    // The page color first (the window itself may be any color), then the glow.
    painter.rect_filled(rect, 0.0, p.bg);
    let (strong, faint) = glow_alphas(p.dark);
    let with = |c: Color32, a: u8| Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), a);
    let second = p.purple;
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(rect.left_top(), with(accent, strong));
    mesh.colored_vertex(rect.right_top(), with(second, faint));
    mesh.colored_vertex(rect.right_bottom(), with(accent, 0));
    mesh.colored_vertex(rect.left_bottom(), with(second, faint / 2));
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    painter.add(egui::Shape::mesh(mesh));
}

/// The WCAG contrast ratio of two opaque colors.
pub fn contrast(a: Color32, b: Color32) -> f32 {
    let l = |c: Color32| {
        let f = |v: u8| {
            let v = v as f32 / 255.0;
            if v <= 0.03928 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
        };
        0.2126 * f(c.r()) + 0.7152 * f(c.g()) + 0.0722 * f(c.b())
    };
    let (la, lb) = (l(a), l(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

/// Text on a solid color: the fill and the text color for a button of
/// `color`, at least 4.5:1. White text on a slightly darker fill where that
/// is enough (blue, rose, indigo); dark text on bright colors (emerald,
/// orange), which would have to be made much darker for white.
pub fn solid(color: Color32) -> (Color32, Color32) {
    const DARK_TEXT: Color32 = Color32::from_rgb(16, 16, 28);
    let mut fill = color;
    for _ in 0..6 {
        if contrast(Color32::WHITE, fill) >= 4.5 {
            return (fill, Color32::WHITE);
        }
        fill = blend(fill, Color32::BLACK, 0.04);
    }
    if contrast(DARK_TEXT, color) >= 4.5 {
        return (color, DARK_TEXT);
    }
    while contrast(Color32::WHITE, fill) < 4.5 {
        fill = blend(fill, Color32::BLACK, 0.04);
    }
    (fill, Color32::WHITE)
}

/// `color` moved towards white (`dark`) or black until it is at least 4.5:1
/// on `bg`: for text in the accent color.
pub fn readable_on(color: Color32, bg: Color32, dark: bool) -> Color32 {
    let to = if dark { Color32::WHITE } else { Color32::BLACK };
    let mut c = color;
    while contrast(c, bg) < 4.5 && c != to {
        c = blend(c, to, 0.06);
    }
    c
}

/// Inter (SIL Open Font License, see LICENSE-Inter.txt).
const INTER_REGULAR: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");
const INTER_SEMIBOLD: &[u8] = include_bytes!("../assets/fonts/Inter-SemiBold.ttf");

pub const SEMIBOLD: &str = "semibold";

fn semibold_family() -> FontFamily {
    FontFamily::Name(SEMIBOLD.into())
}

/// The system's own CJK fonts (tens of MB), loaded only once a title needs
/// them: one with Chinese and Japanese, and one with Korean, which the
/// Chinese fonts of Windows and macOS lack.
fn system_cjk_fonts() -> Vec<FontData> {
    let groups: &[&[&str]] = if cfg!(windows) {
        &[
            &[r"C:\Windows\Fonts\msyh.ttc", r"C:\Windows\Fonts\YuGothM.ttc", r"C:\Windows\Fonts\simsun.ttc"],
            &[r"C:\Windows\Fonts\malgun.ttf"],
        ]
    } else if cfg!(target_os = "macos") {
        &[
            &["/System/Library/Fonts/Hiragino Sans GB.ttc", "/System/Library/Fonts/STHeiti Medium.ttc"],
            &["/System/Library/Fonts/AppleSDGothicNeo.ttc"],
        ]
    } else {
        // Noto Sans CJK has all three.
        &[&[
            "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/google-noto-cjk/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/truetype/wqy/wqy-microhei.ttc",
        ]]
    };
    groups.iter().filter_map(|g| g.iter().find_map(|p| std::fs::read(p).ok())).map(FontData::from_owned).collect()
}

/// System UI font (Segoe UI on Windows, SF on macOS), Phosphor icons, the
/// bundled Arabic font and, with `cjk`, the system's CJK font. While the
/// interface is Arabic, the Arabic font comes first so that spaces and digits
/// in Arabic sentences use it too (see `i18n::visual`); otherwise it is a
/// fallback for Arabic titles.
pub fn install_fonts(ctx: &egui::Context, lang: i18n::Lang, cjk: bool) {
    let mut fonts = FontDefinitions::default();
    let candidates: &[(&str, &str)] = if cfg!(windows) {
        &[("system", r"C:\Windows\Fonts\segoeui.ttf"), ("system-semibold", r"C:\Windows\Fonts\seguisb.ttf")]
    } else if cfg!(target_os = "macos") {
        &[("system", "/System/Library/Fonts/SFNS.ttf")]
    } else {
        &[]
    };
    // Where there is no system font file to use (Linux has none that is
    // the same everywhere), the bundled Inter: the same on every computer.
    let bundled: &[(&str, &'static [u8])] = &[("system", INTER_REGULAR), ("system-semibold", INTER_SEMIBOLD)];
    fonts.font_data.insert("arabic".into(), Arc::new(FontData::from_static(i18n::ARABIC_FONT)));
    fonts.font_data.insert("arabic-semibold".into(), Arc::new(FontData::from_static(i18n::ARABIC_FONT_SEMIBOLD)));
    let mut loaded = Vec::new();
    // Development aid: see the bundled font on any system.
    let candidates: &[(&str, &str)] =
        if cfg!(debug_assertions) && std::env::var_os("SWIFTDM_FONT").is_some() { &[] } else { candidates };
    for (name, path) in candidates {
        if let Ok(bytes) = std::fs::read(path) {
            fonts.font_data.insert((*name).to_string(), Arc::new(FontData::from_owned(bytes)));
            loaded.push(*name);
        }
    }
    for (name, bytes) in bundled {
        if !loaded.contains(name) {
            fonts.font_data.insert((*name).to_string(), Arc::new(FontData::from_static(bytes)));
            loaded.push(*name);
        }
    }
    let proportional = fonts.families.entry(FontFamily::Proportional).or_default();
    if loaded.contains(&"system") {
        proportional.insert(0, "system".into());
    }
    let mut semibold = proportional.clone();
    if loaded.contains(&"system-semibold") {
        semibold.insert(0, "system-semibold".into());
    }
    // Icons go right after the text font of each family.
    egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
    semibold.insert(semibold.len().min(1), "phosphor".into());
    let proportional = fonts.families.entry(FontFamily::Proportional).or_default();
    if lang == i18n::Lang::Ar {
        proportional.insert(0, "arabic".into());
        semibold.insert(0, "arabic-semibold".into());
    } else {
        proportional.push("arabic".into());
        semibold.push("arabic-semibold".into());
    }
    // Arabic titles in any interface language: the Arabic font first.
    let mut rtl = vec!["arabic".to_string()];
    rtl.extend(proportional.iter().filter(|f| *f != "arabic").cloned());
    let monospace = fonts.families.entry(FontFamily::Monospace).or_default();
    monospace.push("arabic".into());
    if cjk {
        for (n, data) in system_cjk_fonts().into_iter().enumerate() {
            let name = format!("cjk-{n}");
            fonts.font_data.insert(name.clone(), Arc::new(data));
            for family in [FontFamily::Proportional, FontFamily::Monospace] {
                fonts.families.entry(family).or_default().push(name.clone());
            }
            semibold.push(name.clone());
            rtl.push(name);
        }
    }
    fonts.families.insert(semibold_family(), semibold);
    fonts.families.insert(FontFamily::Name(text::RTL.into()), rtl);
    ctx.set_fonts(fonts);
}

/// The text sizes of the app: a small scale used by the shared widgets.
pub mod scale {
    /// Details: sizes, times, hints.
    pub const CAPTION: f32 = 13.0;
    /// Text and buttons.
    pub const BODY: f32 = 14.5;
    /// Names of sections and dialogs.
    pub const LEAD: f32 = 17.0;
    /// Headings.
    pub const TITLE: f32 = 22.0;
    /// Page titles.
    pub const DISPLAY: f32 = 28.0;
    /// Icons: in rows and buttons, beside a title, large.
    pub const ICON: f32 = 18.0;
    pub const ICON_LG: f32 = 22.0;
    pub const ICON_XL: f32 = 28.0;
}

pub fn apply_style(ctx: &egui::Context, accent: Accent, night: bool) {
    for theme in [Theme::Dark, Theme::Light] {
        let pal = Palette::new(theme == Theme::Dark, night, accent);
        ctx.style_mut_of(theme, |s| {
            s.visuals = visuals(&pal);
            s.spacing.item_spacing = Vec2::new(8.0, 8.0);
            s.spacing.button_padding = Vec2::new(12.0, 6.0);
            s.spacing.interact_size.y = 30.0;
            s.spacing.window_margin = Margin::same(16);
            s.text_styles = [
                (TextStyle::Heading, FontId::new(scale::TITLE, semibold_family())),
                (TextStyle::Body, FontId::new(scale::BODY, FontFamily::Proportional)),
                (TextStyle::Button, FontId::new(scale::BODY, FontFamily::Proportional)),
                (TextStyle::Small, FontId::new(scale::CAPTION, FontFamily::Proportional)),
                (TextStyle::Monospace, FontId::new(scale::CAPTION, FontFamily::Monospace)),
            ]
            .into();
        });
    }
}

fn visuals(p: &Palette) -> Visuals {
    let mut v = if p.dark { Visuals::dark() } else { Visuals::light() };
    let r = CornerRadius::same(8);
    v.panel_fill = p.bg;
    // Dialogs and menus are solid: text behind them must not show through.
    v.window_fill = p.card;
    v.window_stroke = Stroke::new(1.0, p.border);
    v.window_corner_radius = CornerRadius::same(14);
    v.window_shadow =
        Shadow { offset: [0, 8], blur: 28, spread: 0, color: Color32::from_black_alpha(if p.dark { 110 } else { 40 }) };
    v.popup_shadow =
        Shadow { offset: [0, 4], blur: 16, spread: 0, color: Color32::from_black_alpha(if p.dark { 90 } else { 30 }) };
    v.menu_corner_radius = r;
    v.extreme_bg_color = if p.dark { blend(p.bg, Color32::BLACK, 0.3) } else { Color32::WHITE };
    v.faint_bg_color = p.card_alt;
    v.hyperlink_color = p.accent_text;
    v.selection.bg_fill = blend(p.card, p.accent, if p.dark { 0.38 } else { 0.22 });
    v.selection.stroke = Stroke::new(1.0, p.text);
    v.override_text_color = Some(p.text);
    v.weak_text_color = Some(p.weak);
    v.warn_fg_color = p.warning;
    v.error_fg_color = p.danger;

    let w = &mut v.widgets;
    w.noninteractive.bg_fill = p.card;
    w.noninteractive.weak_bg_fill = p.card;
    w.noninteractive.bg_stroke = Stroke::new(1.0, p.border);
    w.noninteractive.fg_stroke = Stroke::new(1.0, p.text);
    w.inactive.bg_fill = p.card_alt;
    w.inactive.weak_bg_fill = p.card_alt;
    w.inactive.bg_stroke = Stroke::new(1.0, p.control_border);
    w.inactive.fg_stroke = Stroke::new(1.0, p.text);
    w.hovered.bg_fill = p.card_alt;
    w.hovered.weak_bg_fill = blend(p.card_alt, p.accent, if p.dark { 0.12 } else { 0.06 });
    w.hovered.bg_stroke = Stroke::new(1.0, p.accent.gamma_multiply(0.7));
    w.hovered.fg_stroke = Stroke::new(1.5, p.text);
    w.active.bg_fill = p.accent.gamma_multiply(0.35);
    w.active.weak_bg_fill = p.accent.gamma_multiply(0.35);
    w.active.bg_stroke = Stroke::new(1.0, p.accent);
    w.active.fg_stroke = Stroke::new(1.5, p.text);
    w.open = w.active;
    for s in [&mut w.noninteractive, &mut w.inactive, &mut w.hovered, &mut w.active, &mut w.open] {
        s.corner_radius = r;
        s.expansion = 0.0;
    }
    v
}

/// Opaque mix of `a` towards `b` by `t`.
pub fn blend(a: Color32, b: Color32, t: f32) -> Color32 {
    let m = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(m(a.r(), b.r()), m(a.g(), b.g()), m(a.b(), b.b()))
}

// ---------------------------------------------------------------------------
// Widgets

pub fn semibold(text: impl Into<String>, size: f32) -> RichText {
    RichText::new(text).family(semibold_family()).size(size)
}

pub fn card<R>(ui: &mut Ui, p: &Palette, add: impl FnOnce(&mut Ui) -> R) -> InnerResponse<R> {
    Frame::new()
        .fill(p.surface())
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(CornerRadius::same(12))
        .inner_margin(Margin::same(18))
        .show(ui, add)
}

pub fn page_title(ui: &mut Ui, p: &Palette, title: &str, subtitle: &str) {
    ui.label(semibold(title, scale::DISPLAY).color(p.text));
    if !subtitle.is_empty() {
        ui.label(RichText::new(subtitle).color(p.weak).size(scale::BODY));
    }
    ui.add_space(14.0);
}

pub fn section_title(ui: &mut Ui, p: &Palette, icon: &str, title: &str) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(icon).size(scale::ICON).color(p.accent));
        ui.label(semibold(title, scale::LEAD).color(p.text));
    });
    ui.add_space(4.0);
}

pub fn primary_button(ui: &mut Ui, p: &Palette, text: &str, enabled: bool) -> Response {
    // Not faded when nothing can be done yet: a clear "not yet" (a calm
    // fill, readable text), not a button that looks broken.
    let (fill, color, stroke) = if enabled {
        (p.accent_fill, p.on_accent, Stroke::NONE)
    } else {
        (p.card_alt, p.weak, Stroke::new(1.0, p.border))
    };
    ui.add(
        egui::Button::new(semibold(text, scale::BODY).color(color))
            .fill(fill)
            .stroke(stroke)
            .corner_radius(CornerRadius::same(10))
            .min_size(Vec2::new(0.0, 40.0))
            .sense(if enabled { egui::Sense::click() } else { egui::Sense::hover() }),
    )
}

pub fn secondary_button(ui: &mut Ui, text: &str) -> Response {
    ui.add(
        egui::Button::new(RichText::new(text).size(scale::BODY))
            .corner_radius(CornerRadius::same(10))
            .min_size(Vec2::new(0.0, 36.0)),
    )
}

/// A small square button with just an icon, and a tooltip.
pub fn icon_button(ui: &mut Ui, p: &Palette, icon: &str, tip: &str, color: Option<Color32>) -> Response {
    ui.add(
        egui::Button::new(RichText::new(icon).size(scale::ICON).color(color.unwrap_or(p.weak)))
            .fill(Color32::TRANSPARENT)
            .stroke(Stroke::NONE)
            .corner_radius(CornerRadius::same(8))
            .min_size(Vec2::splat(32.0)),
    )
    .on_hover_text(tip)
    .on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Small rounded status label.
pub fn pill(ui: &mut Ui, p: &Palette, text: &str, color: Color32) -> Response {
    Frame::new()
        .fill(p.tint(color))
        .corner_radius(CornerRadius::same(255))
        .inner_margin(Margin::symmetric(9, 2))
        .show(ui, |ui| ui.label(RichText::new(text).size(scale::CAPTION).color(color).strong()))
        .response
}

/// A choice among a few options, drawn as joined buttons.
pub fn segmented<T: PartialEq + Copy>(ui: &mut Ui, p: &Palette, value: &mut T, options: &[(T, &str)]) -> bool {
    let mut changed = false;
    Frame::new().fill(p.card_alt).corner_radius(CornerRadius::same(10)).inner_margin(Margin::same(3)).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            for (v, label) in options {
                let on = *value == *v;
                let text = if on {
                    semibold(*label, scale::BODY).color(p.on_accent)
                } else {
                    RichText::new(*label).size(scale::BODY)
                };
                let b = egui::Button::new(text)
                    .fill(if on { p.accent_fill } else { Color32::TRANSPARENT })
                    .stroke(Stroke::NONE)
                    .corner_radius(CornerRadius::same(8))
                    .min_size(Vec2::new(0.0, 30.0));
                if ui.add(b).clicked() && !on {
                    *value = *v;
                    changed = true;
                }
            }
        });
    });
    changed
}

/// A notice box with an icon, e.g. a warning.
pub fn notice(ui: &mut Ui, p: &Palette, color: Color32, icon: &str, text: &str) {
    Frame::new()
        .fill(p.tint(color))
        .stroke(Stroke::new(1.0, color.gamma_multiply(0.45)))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(Margin::symmetric(14, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(RichText::new(icon).size(scale::ICON).color(color));
                paragraph(ui, text, 14.0, p.text);
            });
        });
}

/// Text wrapped to the available width. `text` is in logical order
/// (`i18n::trl`, `i18n::engine`); right-to-left text is broken into lines
/// here, because egui would put the lines of reordered text in the wrong
/// order. The lines line up with the rest of the (left-to-right) layout.
pub fn paragraph(ui: &mut Ui, text: &str, size: f32, color: Color32) {
    if !i18n::needs_reordering(text) {
        ui.add(egui::Label::new(RichText::new(i18n::strip_marks(text)).size(size).color(color)).wrap());
        return;
    }
    let family = if i18n::is_rtl() { FontFamily::Proportional } else { FontFamily::Name(text::RTL.into()) };
    let width = ui.available_width();
    let font = FontId::new(size, family.clone());
    let measure = |ui: &Ui, s: &str| {
        let shown = i18n::visual(s).into_owned();
        ui.ctx().fonts_mut(|f| f.layout_no_wrap(shown, font.clone(), color).size().x)
    };
    let mut lines: Vec<String> = Vec::new();
    for para in text.split('\n') {
        let mut line = String::new();
        for word in para.split(' ') {
            let candidate = if line.is_empty() { word.to_string() } else { format!("{line} {word}") };
            if !line.is_empty() && measure(ui, &candidate) > width {
                lines.push(std::mem::replace(&mut line, word.to_string()));
            } else {
                line = candidate;
            }
        }
        lines.push(line);
    }
    let centered = ui.layout().is_vertical() && ui.layout().horizontal_align() == egui::Align::Center;
    let align = if centered { egui::Align::Center } else { egui::Align::Min };
    ui.with_layout(egui::Layout::top_down(align), |ui| {
        ui.spacing_mut().item_spacing.y = 2.0;
        for line in lines {
            ui.label(RichText::new(i18n::visual(&line)).size(size).color(color).family(family.clone()));
        }
    });
}

/// What the segments of a download look like on a bar of `width`: for each
/// connection the part it has finished and where it works now (`None` once
/// it is done). Positions are 0..=width.
pub fn segment_spans(resume: &Resume, width: f32) -> Vec<(f32, f32, Option<f32>)> {
    if resume.size == 0 {
        return Vec::new();
    }
    let x = |byte: u64| (byte.min(resume.size) as f64 / resume.size as f64 * width as f64) as f32;
    resume
        .segments
        .iter()
        .map(|&[start, pos, end]| {
            let stop = end.saturating_add(1);
            let filled_to = pos.clamp(start, stop);
            (x(start), x(filled_to), (pos <= end).then(|| x(pos)))
        })
        .collect()
}

/// A progress bar of a download over several connections: each connection
/// fills its own part of the file, and a brighter mark shows where it
/// works now.
pub fn segmented_bar(ui: &mut Ui, p: &Palette, resume: &Resume, color: Color32, width: f32, live: bool) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 6.0), egui::Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(3), p.card_alt);
    let head = blend(color, Color32::WHITE, 0.55);
    let spans = segment_spans(resume, rect.width());
    for (from, to, pos) in &spans {
        if to - from >= 0.5 {
            let r = egui::Rect::from_min_max(
                egui::pos2(rect.left() + from, rect.top()),
                egui::pos2(rect.left() + to, rect.bottom()),
            );
            painter.rect_filled(r, CornerRadius::same(2), color);
        }
        // Where this connection works now.
        if live && let Some(at) = pos {
            let h = egui::Rect::from_min_max(
                egui::pos2((rect.left() + at - 1.0).max(rect.left()), rect.top()),
                egui::pos2((rect.left() + at + 1.5).min(rect.right()), rect.bottom()),
            );
            painter.rect_filled(h, CornerRadius::same(1), head);
        }
    }
}

/// A thin progress bar in `color`.
pub fn bar(ui: &mut Ui, p: &Palette, fraction: Option<f32>, color: Color32, width: f32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 6.0), egui::Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(3), p.card_alt);
    match fraction {
        Some(f) => {
            let mut r = rect;
            r.set_width((rect.width() * f.clamp(0.0, 1.0)).max(if f > 0.0 { 6.0 } else { 0.0 }));
            painter.rect_filled(r, CornerRadius::same(3), color);
        }
        None => {
            // Unknown size: a moving stripe.
            let t = ui.input(|i| i.time) as f32;
            let w = rect.width() * 0.25;
            let x = rect.left() + ((t * 0.6).fract() * (rect.width() + w)) - w;
            let r = egui::Rect::from_min_max(
                egui::pos2(x.max(rect.left()), rect.top()),
                egui::pos2((x + w).min(rect.right()), rect.bottom()),
            );
            if r.width() > 0.0 {
                painter.rect_filled(r, CornerRadius::same(3), color);
            }
            ui.ctx().request_repaint();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every accent in every theme: text on buttons, accent text, and the
    /// status colors as text on cards and on their own tint are readable
    /// (WCAG AA, 4.5:1 for text up to 17 pt).
    #[test]
    fn colors_are_readable() {
        for (dark, night) in [(true, true), (true, false), (false, false)] {
            for accent in Accent::ALL {
                let p = Palette::new(dark, night, accent);
                let who = format!("{accent:?} dark={dark} night={night}");
                assert!(contrast(p.on_accent, p.accent_fill) >= 4.5, "button text, {who}");
                assert!(contrast(p.on_danger, p.danger_fill) >= 4.5, "delete button, {who}");
                assert!(contrast(p.accent_text, p.card) >= 4.5, "accent text on card, {who}");
                assert!(contrast(p.accent_text, p.bg) >= 4.5, "accent text on background, {who}");
                assert!(contrast(p.accent_text, p.card_alt) >= 4.5, "accent text on raised parts, {who}");
                // A button color stays recognizably the accent.
                let (a, b) =
                    (p.accent.r() as i32 - p.accent_fill.r() as i32, p.accent.b() as i32 - p.accent_fill.b() as i32);
                assert!(a.abs() < 80 && b.abs() < 80, "button color too far from the accent, {who}");
            }
            let p = Palette::new(dark, night, Accent::Indigo);
            for (name, c) in [("success", p.success), ("warning", p.warning), ("danger", p.danger)] {
                assert!(contrast(c, p.card) >= 4.5, "{name} on card, dark={dark} night={night}");
                assert!(
                    contrast(c, blend(p.card, c, if dark { 0.16 } else { 0.10 })) >= 4.5,
                    "{name} on its tint, dark={dark} night={night}"
                );
            }
            for bg in [p.bg, p.card, p.card_alt, p.sidebar] {
                assert!(contrast(p.text, bg) >= 7.0, "text, dark={dark} night={night}");
                assert!(contrast(p.weak, bg) >= 4.5, "weak text, dark={dark} night={night}");
            }
        }
    }

    #[test]
    fn segments_become_spans_of_the_bar() {
        // Three parts of a 300-byte file: half done, not started, done.
        let r = Resume { size: 300, segments: vec![[0, 50, 99], [100, 100, 199], [200, 300, 299]], validator: None };
        let spans = segment_spans(&r, 300.0);
        assert_eq!(spans[0], (0.0, 50.0, Some(50.0)));
        assert_eq!(spans[1], (100.0, 100.0, Some(100.0)));
        assert_eq!(spans[2], (200.0, 300.0, None));
        assert!(segment_spans(&Resume { size: 0, segments: vec![], validator: None }, 100.0).is_empty());
    }

    /// The bundled text font must not have glyphs of its own in the private
    /// use area: that is where the icons are, and the first font that has a
    /// code point draws it (as a stray letter, if it is Inter's).
    #[test]
    fn the_bundled_font_leaves_the_icons_alone() {
        use skrifa::MetadataProvider;
        for (name, bytes) in [("regular", INTER_REGULAR), ("semibold", INTER_SEMIBOLD)] {
            let font = skrifa::FontRef::new(bytes).expect(name);
            let clashes = font.charmap().mappings().filter(|(c, _)| (0xE000..=0xF8FF).contains(c)).count();
            assert_eq!(clashes, 0, "{name} has {clashes} glyphs in the private use area");
            // Text is still all there.
            assert!(font.charmap().map('A').is_some() && font.charmap().map('ö').is_some(), "{name}");
        }
    }

    /// Frosted surfaces let the backdrop through a little: with its glow at
    /// the strongest, text on them is still readable.
    #[test]
    fn text_is_readable_on_frosted_surfaces() {
        let over = |top: Color32, alpha: f32, below: Color32| blend(below, top, alpha);
        for (dark, night) in [(true, true), (true, false), (false, false)] {
            for accent in Accent::ALL {
                set_glass(true);
                let p = Palette::new(dark, night, accent);
                // The page with the backdrop at its strongest (top left corner).
                let glow = glow_alphas(dark).0 as f32 / 255.0;
                let page = over(p.accent, glow, p.bg);
                let who = format!("{accent:?} dark={dark} night={night}");
                for (name, c, a) in [("card", p.card, 0.80), ("bar", p.sidebar, 0.78)] {
                    let surface = over(c, a, page);
                    assert!(contrast(p.text, surface) >= 7.0, "text on {name}, {who}");
                    assert!(contrast(p.weak, surface) >= 4.5, "weak text on {name}, {who}");
                    assert!(contrast(p.accent_text, surface) >= 4.5, "accent text on {name}, {who}");
                    for (what, color) in [("success", p.success), ("warning", p.warning), ("danger", p.danger)] {
                        assert!(contrast(color, surface) >= 4.5, "{what} on {name}, {who}");
                    }
                }
            }
        }
        set_glass(true);
    }
}
