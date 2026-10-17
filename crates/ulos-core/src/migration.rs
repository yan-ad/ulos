//! Bounded URL-only migration. Source-bearing values deliberately do not implement Debug.
use std::{
    fmt,
    fs::File,
    io::{Read, Write},
    path::Path,
};

pub const MAX_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_ENTRIES: usize = 500;
pub const MAX_LINE: usize = 16 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MigrationFormat {
    IdmEf2,
    Text,
}
#[derive(Clone, PartialEq, Eq)]
pub struct MigrationEntry {
    pub source: String,
    pub ignored_fields: usize,
}
#[derive(Clone, PartialEq, Eq)]
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
    Read,
    Write,
}
impl fmt::Display for MigrationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::TooLarge => "migration file is too large",
            Self::TooManyEntries => "migration file contains too many entries",
            Self::InvalidUtf8 => "migration file is not UTF-8",
            Self::InvalidLine => "migration file contains an invalid entry",
            Self::UnsupportedField => "migration file contains unsupported fields (including authentication, cookies or POST data)",
            Self::InvalidUrl => "migration file contains an unsupported URL",
            Self::Empty => "migration file contains no supported URLs",
            Self::Read => "could not read a regular migration file (symlinks are not supported)",
            Self::Write => "could not create export; choose a new file name in a writable folder",
        })
    }
}
impl std::error::Error for MigrationError {}

fn validate_url(value: &str) -> Result<(), MigrationError> {
    if value.is_empty()
        || value.len() > MAX_LINE
        || value.chars().any(|c| c.is_whitespace() || c.is_control())
    {
        return Err(MigrationError::InvalidUrl);
    }
    let url = url::Url::parse(value).map_err(|_| MigrationError::InvalidUrl)?;
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.host_str().is_none()
    {
        return Err(MigrationError::InvalidUrl);
    }
    Ok(())
}
fn push(
    entries: &mut Vec<MigrationEntry>,
    source: &str,
    ignored_fields: usize,
) -> Result<(), MigrationError> {
    validate_url(source)?;
    if entries.len() == MAX_ENTRIES {
        return Err(MigrationError::TooManyEntries);
    }
    entries.push(MigrationEntry {
        source: source.to_owned(),
        ignored_fields,
    });
    Ok(())
}
pub fn parse(bytes: &[u8], format: MigrationFormat) -> Result<MigrationBatch, MigrationError> {
    if bytes.len() > MAX_BYTES {
        return Err(MigrationError::TooLarge);
    }
    let text = std::str::from_utf8(bytes).map_err(|_| MigrationError::InvalidUtf8)?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut entries = Vec::new();
    let mut block: Option<MigrationEntry> = None;
    for line in text.lines() {
        if line.len() > MAX_LINE {
            return Err(MigrationError::InvalidLine);
        }
        if line.is_empty() && block.is_none() {
            continue;
        }
        if format == MigrationFormat::Text {
            push(&mut entries, line, 0)?;
            continue;
        }
        if line == "<" && block.is_none() {
            block = Some(MigrationEntry {
                source: String::new(),
                ignored_fields: 0,
            });
        } else if let Some(entry) = &mut block {
            if entry.source.is_empty() {
                validate_url(line)?;
                entry.source = line.to_owned();
            } else if line == ">" {
                let entry = block.take().ok_or(MigrationError::InvalidLine)?;
                push(&mut entries, &entry.source, entry.ignored_fields)?;
            } else if line.split_once(':').is_some_and(|(name, _)| {
                name.eq_ignore_ascii_case("referer") || name.eq_ignore_ascii_case("user-agent")
            }) && !line.chars().any(char::is_control)
            {
                entry.ignored_fields += 1;
            } else {
                return Err(MigrationError::UnsupportedField);
            }
        } else if let Some(source) = line.strip_prefix('<').and_then(|v| v.strip_suffix('>')) {
            // Older Ulos exports used one URL-only record per line.
            push(&mut entries, source.trim_matches(' '), 0)?;
        } else {
            return Err(MigrationError::InvalidLine);
        }
    }
    if block.is_some() {
        return Err(MigrationError::InvalidLine);
    }
    if entries.is_empty() {
        return Err(MigrationError::Empty);
    }
    let ignored_fields = entries.iter().map(|entry| entry.ignored_fields).sum();
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
        if format == MigrationFormat::IdmEf2 {
            output.push_str("<\r\n");
        }
        output.push_str(source);
        output.push_str("\r\n");
        if format == MigrationFormat::IdmEf2 {
            output.push_str(">\r\n");
        }
        if output.len() > MAX_BYTES {
            return Err(MigrationError::TooLarge);
        }
    }
    Ok(output.into_bytes())
}

/// Blocking I/O: call on a blocking worker, never on a UI executor.
pub fn read_file(path: &Path, format: MigrationFormat) -> Result<MigrationBatch, MigrationError> {
    let metadata = std::fs::symlink_metadata(path).map_err(|_| MigrationError::Read)?;
    if !metadata.is_file() || metadata.len() > MAX_BYTES as u64 {
        return Err(if metadata.len() > MAX_BYTES as u64 {
            MigrationError::TooLarge
        } else {
            MigrationError::Read
        });
    }
    #[cfg(unix)]
    let file = File::from(
        rustix::fs::open(
            path,
            rustix::fs::OFlags::RDONLY
                | rustix::fs::OFlags::NOFOLLOW
                | rustix::fs::OFlags::NONBLOCK,
            rustix::fs::Mode::empty(),
        )
        .map_err(|_| MigrationError::Read)?,
    );
    #[cfg(windows)]
    let file = {
        use std::os::windows::fs::OpenOptionsExt;
        // Open the reparse point itself, not its target, then reject non-regular handles.
        std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(0x0020_0000) // FILE_FLAG_OPEN_REPARSE_POINT
            .open(path)
            .map_err(|_| MigrationError::Read)?
    };
    #[cfg(not(any(unix, windows)))]
    let file = File::open(path).map_err(|_| MigrationError::Read)?;
    if !file.metadata().map_err(|_| MigrationError::Read)?.is_file() {
        return Err(MigrationError::Read);
    }
    let mut bytes = Vec::new();
    file.take(MAX_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| MigrationError::Read)?;
    parse(&bytes, format)
}

/// Unique same-directory staging and atomic no-clobber publication, including races.
/// Never replaces an existing file, symlink or directory. Blocking I/O.
pub fn export_file(
    path: &Path,
    entries: &[String],
    format: MigrationFormat,
) -> Result<(), MigrationError> {
    let bytes = encode(entries, format)?;
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut temporary = tempfile::Builder::new()
        .prefix(".ulos-export-")
        .tempfile_in(parent)
        .map_err(|_| MigrationError::Write)?;
    temporary
        .write_all(&bytes)
        .map_err(|_| MigrationError::Write)?;
    temporary
        .as_file()
        .sync_all()
        .map_err(|_| MigrationError::Write)?;
    temporary
        .persist_noclobber(path)
        .map_err(|_| MigrationError::Write)?;
    Ok(())
}
