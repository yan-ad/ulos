use std::fmt;

const MAX_BYTES: usize = 2 * 1024 * 1024;
const MAX_ENTRIES: usize = 500;
const MAX_LINE: usize = 16 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MigrationFormat {
    IdmEf2,
    Text,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MigrationEntry {
    pub source: String,
    pub ignored_fields: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MigrationBatch {
    pub entries: Vec<MigrationEntry>,
    pub ignored_fields: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MigrationError {
    TooLarge,
    TooManyEntries,
    InvalidUtf8,
    InvalidLine,
    UnsupportedField,
    InvalidUrl,
    Empty,
}

impl fmt::Display for MigrationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::TooLarge => "migration file is too large",
            Self::TooManyEntries => "migration file contains too many entries",
            Self::InvalidUtf8 => "migration file is not UTF-8",
            Self::InvalidLine => "migration file contains an invalid entry",
            Self::UnsupportedField => "migration file contains unsupported fields",
            Self::InvalidUrl => "migration file contains an unsupported URL",
            Self::Empty => "migration file contains no supported URLs",
        })
    }
}

impl std::error::Error for MigrationError {}

fn validate_url(value: &str) -> Result<(), MigrationError> {
    let url = url::Url::parse(value).map_err(|_| MigrationError::InvalidUrl)?;
    if !matches!(url.scheme(), "http" | "https") || url.username() != "" || url.password().is_some()
    {
        return Err(MigrationError::InvalidUrl);
    }
    Ok(())
}

fn entry_from_text(line: &str) -> Result<MigrationEntry, MigrationError> {
    if line.is_empty() || line.len() > MAX_LINE {
        return Err(MigrationError::InvalidLine);
    }
    let mut parts = line.split_whitespace();
    let source = parts.next().ok_or(MigrationError::InvalidLine)?.to_owned();
    validate_url(&source)?;
    let mut ignored_fields = 0;
    while let Some(part) = parts.next() {
        if part.starts_with("referer:") || part.starts_with("User-Agent:") {
            ignored_fields += 1;
            let _ = parts.next();
        } else {
            return Err(MigrationError::UnsupportedField);
        }
    }
    Ok(MigrationEntry {
        source,
        ignored_fields,
    })
}

pub fn parse(bytes: &[u8], format: MigrationFormat) -> Result<MigrationBatch, MigrationError> {
    if bytes.len() > MAX_BYTES {
        return Err(MigrationError::TooLarge);
    }
    let text = std::str::from_utf8(bytes).map_err(|_| MigrationError::InvalidUtf8)?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut entries = Vec::new();
    let mut ignored_fields = 0;
    match format {
        MigrationFormat::Text => {
            for line in text.lines().map(str::trim).filter(|line| !line.is_empty()) {
                if entries.len() == MAX_ENTRIES {
                    return Err(MigrationError::TooManyEntries);
                }
                let entry = entry_from_text(line)?;
                ignored_fields += entry.ignored_fields;
                entries.push(entry);
            }
        }
        MigrationFormat::IdmEf2 => {
            for raw in text.lines().map(str::trim).filter(|line| !line.is_empty()) {
                if entries.len() == MAX_ENTRIES {
                    return Err(MigrationError::TooManyEntries);
                }
                if !raw.starts_with('<') || !raw.ends_with('>') {
                    return Err(MigrationError::InvalidLine);
                }
                let entry = entry_from_text(raw[1..raw.len() - 1].trim())?;
                ignored_fields += entry.ignored_fields;
                entries.push(entry);
            }
        }
    }
    if entries.is_empty() {
        return Err(MigrationError::Empty);
    }
    Ok(MigrationBatch {
        entries,
        ignored_fields,
    })
}

pub fn encode(entries: &[String], format: MigrationFormat) -> Result<Vec<u8>, MigrationError> {
    if entries.is_empty() {
        return Err(MigrationError::Empty);
    }
    if entries.len() > MAX_ENTRIES {
        return Err(MigrationError::TooManyEntries);
    }
    let mut output = String::new();
    for source in entries {
        validate_url(source)?;
        match format {
            MigrationFormat::Text => {
                output.push_str(source);
                output.push_str("\r\n");
            }
            MigrationFormat::IdmEf2 => {
                output.push_str("< ");
                output.push_str(source);
                output.push_str(" >\r\n");
            }
        }
    }
    if output.len() > MAX_BYTES {
        return Err(MigrationError::TooLarge);
    }
    Ok(output.into_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    const URL: &str = "https://example.org/file.zip";

    #[test]
    fn round_trips_text_and_ef2_url_entries() {
        for format in [MigrationFormat::Text, MigrationFormat::IdmEf2] {
            let bytes = encode(&[URL.to_owned()], format).unwrap();
            let batch = parse(&bytes, format).unwrap();
            assert_eq!(batch.entries[0].source, URL);
        }
    }

    #[test]
    fn accepts_known_idm_headers_but_never_preserves_them() {
        let batch = parse(
            b"< https://example.org/a referer: https://example.org User-Agent: browser >\r\n",
            MigrationFormat::IdmEf2,
        )
        .unwrap();
        assert_eq!(batch.entries[0].ignored_fields, 2);
    }

    #[test]
    fn rejects_credentials_unknown_fields_and_magnets() {
        for input in [
            "https://user:pass@example.org/a",
            "magnet:?xt=urn:btih:x",
            "https://example.org/a cookies: secret",
        ] {
            assert!(parse(input.as_bytes(), MigrationFormat::Text).is_err());
        }
    }
}
