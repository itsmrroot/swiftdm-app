//! Text that comes from outside the app (titles, file names, messages of
//! the engine), drawn in the right font and order whatever its script.

use eframe::egui::{FontFamily, RichText};

use crate::i18n::{needs_reordering, visual};

/// Font family for text with Arabic in it: the Arabic font first (see
/// `i18n` for why the order matters).
pub const RTL: &str = "rtl";

/// `text` ready to draw, in the right font and order.
pub fn rich(text: &str) -> RichText {
    if needs_reordering(text) {
        RichText::new(visual(text)).family(FontFamily::Name(RTL.into()))
    } else {
        // Direction marks around numbers and names (see `i18n::ltr`) are no
        // glyphs of any font: removed also when there is no Arabic to order.
        RichText::new(crate::i18n::strip_marks(text))
    }
}

/// Whether `s` has Chinese, Japanese or Korean characters, which need the
/// system's font.
pub fn has_cjk(s: &str) -> bool {
    s.chars().any(|c| {
        matches!(c, '\u{3040}'..='\u{30FF}' | '\u{3400}'..='\u{9FFF}' | '\u{AC00}'..='\u{D7AF}' | '\u{F900}'..='\u{FAFF}')
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scripts() {
        assert!(has_cjk("東京") && !has_cjk("Tokyo"));
    }
}
