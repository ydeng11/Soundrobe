//! Comparison preparation never changes display metadata.
use unicode_normalization::UnicodeNormalization;

pub fn comparison_text(value: &str) -> String {
    html_escape::decode_html_entities(value).nfkc().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn entities_and_unicode_compare_without_losing_recording_versions() {
        assert_eq!(
            comparison_text("B&amp;B &#39;Cafe&#x301;&#39;"),
            "B&B 'Café'"
        );
        assert_eq!(
            comparison_text("Song (Live &amp; Acoustic)"),
            "Song (Live & Acoustic)"
        );
        assert_eq!(
            comparison_text("Unknown &notanentity;"),
            "Unknown &notanentity;"
        );
    }
}
