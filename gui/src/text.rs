//! Titles in any script. egui shapes Arabic (joined letters) but has no
//! bidirectional layout: it cuts a line into runs by font, places the runs
//! left to right, and the shaper reverses every run that contains Arabic.
//! [`visual`] reorders a line with the Unicode bidi algorithm and then
//! pre-reverses the Arabic runs, so that egui draws the right order —
//! including numbers and Latin words inside Arabic titles. Such text is
//! drawn with the [`RTL`] font family, which puts the Arabic font first.

use std::borrow::Cow;
use std::sync::LazyLock;

use eframe::egui::{FontFamily, RichText};
use skrifa::MetadataProvider;
use unicode_bidi::{BidiInfo, Level};

pub const ARABIC_FONT: &[u8] = include_bytes!("../assets/fonts/NotoSansArabic-Regular.ttf");

/// Font family for text with Arabic in it.
pub const RTL: &str = "rtl";

/// `text` ready to draw, in the right font and order.
pub fn rich(text: &str) -> RichText {
    if needs_reordering(text) {
        RichText::new(visual(text)).family(FontFamily::Name(RTL.into()))
    } else {
        RichText::new(text)
    }
}

/// Whether `s` has Chinese, Japanese or Korean characters, which need the
/// system's font.
pub fn has_cjk(s: &str) -> bool {
    s.chars().any(|c| matches!(c, '\u{3040}'..='\u{30FF}' | '\u{3400}'..='\u{9FFF}' | '\u{AC00}'..='\u{D7AF}' | '\u{F900}'..='\u{FAFF}'))
}

fn is_arabic(c: char) -> bool {
    matches!(c, '\u{0600}'..='\u{06FF}' | '\u{0750}'..='\u{077F}' | '\u{08A0}'..='\u{08FF}' | '\u{FB50}'..='\u{FDFF}' | '\u{FE70}'..='\u{FEFF}')
}

fn needs_reordering(s: &str) -> bool {
    s.chars().any(is_arabic)
}

/// Whether egui draws `c` with the Arabic font (it comes first in the
/// [`RTL`] family, so: whether that font has the character).
fn in_arabic_font(c: char) -> bool {
    static FONT: LazyLock<Option<skrifa::FontRef<'static>>> = LazyLock::new(|| skrifa::FontRef::new(ARABIC_FONT).ok());
    FONT.as_ref().is_some_and(|f| f.charmap().map(c).is_some())
}

/// Converts text to the order in which egui must receive it to draw it
/// correctly as right-to-left text. Text without Arabic is returned as is.
pub fn visual(s: &str) -> Cow<'_, str> {
    if !needs_reordering(s) {
        return Cow::Borrowed(s);
    }
    let mut out = String::with_capacity(s.len() + 8);
    for (i, line) in s.split('\n').enumerate() {
        if i > 0 {
            out.push('\n');
        }
        visual_line(line, &mut out);
    }
    Cow::Owned(out)
}

fn visual_line(line: &str, out: &mut String) {
    let info = BidiInfo::new(line, Some(Level::rtl()));
    let Some(para) = info.paragraphs.first() else {
        out.push_str(line);
        return;
    };
    // The order in which the characters must appear on screen, left to right,
    // and whether each one is part of right-to-left text.
    let (_, runs) = info.visual_runs(para, para.range.clone());
    let mut screen: Vec<(char, bool)> = Vec::with_capacity(line.len());
    for run in runs {
        let rtl = info.levels[run.start].is_rtl();
        let chars = line[run.clone()].chars().map(|c| (c, rtl));
        if rtl { screen.extend(chars.rev()) } else { screen.extend(chars) }
    }
    // egui shapes each stretch of characters that share a font separately and
    // the shaper reverses (and mirrors) those containing Arabic: reverse them
    // beforehand. Brackets in right-to-left text that the shaper does not see
    // as Arabic are mirrored here instead.
    let mut start = 0;
    while start < screen.len() {
        let arabic_font = in_arabic_font(screen[start].0);
        let end = screen[start..]
            .iter()
            .position(|&(c, _)| in_arabic_font(c) != arabic_font)
            .map_or(screen.len(), |n| start + n);
        let piece = &screen[start..end];
        if arabic_font && piece.iter().any(|&(c, _)| is_arabic(c)) {
            out.extend(piece.iter().rev().map(|&(c, _)| c));
        } else {
            out.extend(piece.iter().map(|&(c, rtl)| if rtl { mirror(c) } else { c }));
        }
        start = end;
    }
}

/// The mirror image of a bracket, as drawn in right-to-left text.
fn mirror(c: char) -> char {
    match c {
        '(' => ')',
        ')' => '(',
        '[' => ']',
        ']' => '[',
        '{' => '}',
        '}' => '{',
        '<' => '>',
        '>' => '<',
        '«' => '»',
        '»' => '«',
        c => c,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn latin_text_is_unchanged() {
        assert!(matches!(visual("Me at the zoo (2005)"), Cow::Borrowed(_)));
        assert!(has_cjk("東京") && !has_cjk("Tokyo"));
    }

    #[test]
    fn arabic_is_reordered() {
        let shown = visual("مرحبا 2024");
        assert_ne!(shown, "مرحبا 2024");
        assert_eq!(shown.chars().count(), "مرحبا 2024".chars().count());
    }
}
