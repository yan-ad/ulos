use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
};

#[derive(Default)]
pub struct ClipboardDetector {
    fingerprint: Option<u64>,
}

impl ClipboardDetector {
    pub fn candidate(&mut self, text: Option<&str>) -> Option<String> {
        let text = text.filter(|text| text.len() <= 16_384)?;
        let mut hash = DefaultHasher::new();
        text.hash(&mut hash);
        let fingerprint = hash.finish();
        if self.fingerprint == Some(fingerprint) {
            return None;
        }
        self.fingerprint = Some(fingerprint);
        clipboard_url(text)
    }
}

pub fn clipboard_url(text: &str) -> Option<String> {
    if text.len() > 16_384 {
        return None;
    }
    let source = text.trim();
    if source.chars().any(char::is_whitespace) {
        return None;
    }
    ulos_core::naming::suggest_filename(source).ok()?;
    Some(source.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn accepts_only_one_bounded_http_url() {
        assert_eq!(
            clipboard_url(" https://example.org/test?token=value \n"),
            Some("https://example.org/test?token=value".into())
        );
        for source in [
            "password",
            "file:///etc/passwd",
            "magnet:?xt=x",
            "https://u:p@example.org/f",
            "https://example.org/1\nhttps://example.org/2",
        ] {
            assert!(clipboard_url(source).is_none());
        }
        assert!(clipboard_url(&format!("https://example.org/{}", "x".repeat(16384))).is_none());
    }
    #[test]
    fn deduplicates_without_retaining_clipboard_content() {
        let mut detector = ClipboardDetector::default();
        assert!(detector.candidate(Some("https://example.org/f")).is_some());
        assert!(detector.candidate(Some("https://example.org/f")).is_none());
        assert!(detector.candidate(Some("not a URL")).is_none());
        assert!(detector.candidate(Some("https://example.org/f")).is_some());
    }
}
