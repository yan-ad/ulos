//! Pure, URL-derived filename suggestions and portable single-component sanitization.
//!
//! These helpers perform no network or filesystem operations. Suggestions do not
//! inspect response headers (including Content-Disposition); callers should also
//! sanitize manually entered names before joining them to a destination directory.

use percent_encoding::percent_decode_str;
use url::Url;

use crate::DownloadError;

/// Maximum UTF-8 byte length of a sanitized filename.
pub const MAX_FILENAME_BYTES: usize = 180;
const MAX_EXTENSION_BYTES: usize = 20;
const DEFAULT_FILENAME: &str = "download.bin";

/// Suggest a safe filename from the final URL path segment, ignoring query and fragment.
///
/// Directory URLs use `download.bin`. Invalid UTF-8 in percent-encoded segments
/// is replaced lossily, and decoded separators never become directory components.
/// URL validation mirrors [`crate::DownloadRequest`] without inspecting a destination.
pub fn suggest_filename(source: &str) -> Result<String, DownloadError> {
    let url = Url::parse(source).map_err(|_| DownloadError::InvalidUrl)?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err(DownloadError::InvalidUrl);
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(DownloadError::EmbeddedCredentials);
    }

    let segment = url.path().rsplit('/').next().unwrap_or_default();
    Ok(sanitize_filename(
        &percent_decode_str(segment).decode_utf8_lossy(),
    ))
}

/// Sanitize any name into a non-hidden, portable, bounded single filename component.
///
/// Case and Unicode spelling are preserved; Unicode normalization, collision
/// avoidance, and filesystem-specific equivalence are not guaranteed. This does
/// not create files or permit overwriting them.
pub fn sanitize_filename(name: &str) -> String {
    let replaced: String = name
        .chars()
        .map(|ch| {
            if ch.is_control()
                || matches!(ch, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' | '\u{061c}' | '\u{200e}' | '\u{200f}')
                || matches!(ch, '/' | '\\' | '<' | '>' | ':' | '"' | '|' | '?' | '*')
            {
                '_'
            } else {
                ch
            }
        })
        .collect();
    let trimmed = replaced.trim_matches([' ', '.']);
    if trimmed.is_empty() {
        return DEFAULT_FILENAME.to_owned();
    }

    let mut safe = String::with_capacity(trimmed.len());
    if is_reserved(trimmed) {
        safe.push('_');
    }
    safe.push_str(trimmed);

    if safe.len() > MAX_FILENAME_BYTES {
        let extension = safe.rfind('.').and_then(|index| {
            let suffix = &safe[index..];
            (suffix.len() > 1 && suffix.len() <= MAX_EXTENSION_BYTES).then_some(index)
        });
        if let Some(index) = extension {
            let suffix = safe[index..].to_owned();
            let end = char_boundary(&safe, MAX_FILENAME_BYTES - suffix.len());
            safe.truncate(end);
            // Truncation may expose a space or dot at the end of the stem.
            safe = safe.trim_end_matches([' ', '.']).to_owned();
            safe.push_str(&suffix);
        } else {
            let end = char_boundary(&safe, MAX_FILENAME_BYTES);
            safe.truncate(end);
        }
    }
    let safe = safe.trim_end_matches([' ', '.']);
    if safe.is_empty() {
        DEFAULT_FILENAME.to_owned()
    } else if is_reserved(safe) {
        // Truncation and trailing-space trimming can expose a device base.
        format!("_{safe}")
    } else {
        safe.to_owned()
    }
}

fn char_boundary(value: &str, mut end: usize) -> usize {
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    end
}

fn is_reserved(name: &str) -> bool {
    let base = name.split('.').next().unwrap_or_default().trim_matches(' ');
    if ["CON", "PRN", "AUX", "NUL"]
        .iter()
        .any(|reserved| base.eq_ignore_ascii_case(reserved))
    {
        return true;
    }
    let mut chars = base.chars();
    let prefix: String = chars.by_ref().take(3).collect();
    (prefix.eq_ignore_ascii_case("COM") || prefix.eq_ignore_ascii_case("LPT"))
        && matches!(chars.next(), Some('1'..='9' | '¹' | '²' | '³'))
        && chars.next().is_none()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suggestions_decode_unicode_and_preserve_case() {
        assert_eq!(
            suggest_filename("https://example.test/files/Caf%C3%A9.ZIP").unwrap(),
            "Café.ZIP"
        );
    }

    #[test]
    fn suggestions_ignore_signed_query_and_fragment() {
        assert_eq!(
            suggest_filename("https://example.test/file.zip?token=SECRET/name#PRIVATE").unwrap(),
            "file.zip"
        );
    }

    #[test]
    fn directory_urls_use_default() {
        for url in [
            "https://example.test",
            "http://example.test/",
            "https://example.test/files/?token=secret",
        ] {
            assert_eq!(suggest_filename(url).unwrap(), DEFAULT_FILENAME);
        }
    }

    #[test]
    fn decoded_separators_and_dotdot_cannot_traverse() {
        assert_eq!(
            suggest_filename("https://example.test/%2F..%2Fsecret%5Cfile").unwrap(),
            "_.._secret_file"
        );
        assert_eq!(
            suggest_filename("https://example.test/%2e%2e").unwrap(),
            DEFAULT_FILENAME
        );
        assert_eq!(
            suggest_filename("https://example.test/%2e%2e%2f").unwrap(),
            "_"
        );
    }

    #[test]
    fn invalid_encoded_utf8_is_lossy_without_panicking() {
        assert_eq!(
            suggest_filename("https://example.test/%FF.bin").unwrap(),
            "�.bin"
        );
    }

    #[test]
    fn rejects_invalid_or_unsupported_urls() {
        for source in [
            "",
            "not a URL",
            "https://",
            "https://[invalid]/a",
            "file:///a",
            "ftp://example.test/a",
            "data:text/plain,a",
        ] {
            assert!(matches!(
                suggest_filename(source),
                Err(DownloadError::InvalidUrl)
            ));
        }
    }

    #[test]
    fn rejects_embedded_credentials_without_echoing_secrets() {
        for source in [
            "https://user:secret@example.test/file",
            "https://user@example.test/file",
            "https://:secret@example.test/file",
        ] {
            let error = suggest_filename(source).unwrap_err();
            assert!(matches!(error, DownloadError::EmbeddedCredentials));
            assert!(!error.to_string().contains("secret"));
        }
    }

    #[test]
    fn strips_hidden_and_empty_names() {
        for name in ["", " ", ".", "..", " ... "] {
            assert_eq!(sanitize_filename(name), DEFAULT_FILENAME);
        }
        assert_eq!(sanitize_filename(" ..hidden.txt. "), "hidden.txt");
    }

    #[test]
    fn replaces_path_and_platform_punctuation() {
        assert_eq!(sanitize_filename("/a\\b<>:\"|?*"), "_a_b_______");
        assert_eq!(sanitize_filename("C:\\windows\\file"), "C__windows_file");
    }

    #[test]
    fn replaces_controls_and_bidi_formatting() {
        assert_eq!(
            sanitize_filename(
                "a\0\n\u{7f}\u{85}\u{202a}\u{202b}\u{202c}\u{202d}\u{202e}\u{2066}\u{2067}\u{2068}\u{2069}z"
            ),
            format!("a{}z", "_".repeat(13))
        );
    }

    #[test]
    fn reserves_windows_device_bases_even_with_extensions_and_spaces() {
        for base in [
            "con", "PrN", "AUX", "nul", "COM1", "com9", "LPT1", "lpt9", "COM¹", "COM²", "COM³",
            "LPT¹", "LPT²", "LPT³",
        ] {
            for name in [
                base.to_owned(),
                format!("{base}.txt"),
                format!(" {base} .txt "),
            ] {
                assert!(sanitize_filename(&name).starts_with('_'), "{name}");
            }
        }
        for name in [
            "COM0.txt",
            "LPT0",
            "COM10",
            "LPT10.txt",
            "COM⁴",
            "company.txt",
            "console.txt",
        ] {
            assert_eq!(sanitize_filename(name), name);
        }
    }

    #[test]
    fn bounds_multibyte_names_and_preserves_short_extension() {
        let safe = sanitize_filename(&format!("{}.zip", "界".repeat(100)));
        assert!(safe.len() <= MAX_FILENAME_BYTES);
        assert!(safe.ends_with(".zip"));
        assert_eq!(safe, format!("{}.zip", "界".repeat(58)));
        assert_eq!(sanitize_filename(&"a".repeat(180)).len(), 180);
        assert_eq!(sanitize_filename(&"a".repeat(181)).len(), 180);
    }

    #[test]
    fn bounds_long_extensions_and_exposed_trailing_spaces() {
        assert_eq!(
            sanitize_filename(&format!("COM1{}z", " ".repeat(180))),
            "_COM1"
        );
        assert_eq!(
            sanitize_filename(&format!("{}.{}", "a".repeat(180), "b".repeat(21))),
            "a".repeat(180)
        );
        let safe = sanitize_filename(&format!("{} {}", "a".repeat(179), "z".repeat(40)));
        assert_eq!(safe, "a".repeat(179));
        let safe = sanitize_filename(&format!("{}.{}.txt", "a".repeat(175), "z".repeat(40)));
        assert!(safe.ends_with(".txt"));
        assert!(safe.len() <= MAX_FILENAME_BYTES);
    }

    #[test]
    fn sanitization_is_idempotent_and_single_component() {
        for name in [
            "../a",
            "..\\a",
            "/root",
            "CON.txt",
            "...",
            "C:\\a",
            "résumé.txt",
            "\u{202e}txt.exe",
        ] {
            let safe = sanitize_filename(name);
            assert_eq!(sanitize_filename(&safe), safe);
            assert!(!safe.starts_with('.'));
            assert!(!safe.contains(['/', '\\']));
            assert!(safe.len() <= MAX_FILENAME_BYTES);
        }
    }
}
