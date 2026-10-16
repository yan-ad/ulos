//! Pure, bounded local `.torrent` intake. No filesystem, network, or persistence work.
use std::{ffi::OsString, path::PathBuf};

use url::Url;

pub const MAX_INTAKE_FILES: usize = 16;
const MAX_PATH_BYTES: usize = 16 * 1024;
const INVALID_PATH: &str = "Choose a local .torrent file.";
const TOO_MANY: &str = "Choose at most 16 torrent files at a time.";
const TOO_LONG: &str = "A torrent path exceeds the 16 KiB limit.";

/// Merge validated OS proposals atomically; repeats do not consume inbox capacity.
pub fn merge_pending(
    pending: &mut Vec<PathBuf>,
    proposed: Vec<PathBuf>,
) -> Result<(), &'static str> {
    let proposed = torrent_paths(proposed)?;
    let mut merged = pending.clone();
    for path in proposed {
        if !merged.contains(&path) {
            merged.push(path);
        }
    }
    if merged.len() > MAX_INTAKE_FILES {
        return Err(TOO_MANY);
    }
    *pending = merged;
    Ok(())
}

#[cfg(test)]
mod inbox_tests {
    use super::*;
    #[test]
    fn repeats_do_not_consume_inbox_capacity_and_overflow_is_atomic() {
        let mut pending: Vec<_> = (0..16)
            .map(|index| PathBuf::from(format!("{index}.torrent")))
            .collect();
        let full = pending.clone();
        merge_pending(&mut pending, vec![PathBuf::from("0.torrent")]).unwrap();
        assert_eq!(pending, full);
        assert!(merge_pending(&mut pending, vec![PathBuf::from("new.torrent")]).is_err());
        assert_eq!(pending, full);
        pending.pop();
        merge_pending(
            &mut pending,
            vec![PathBuf::from("0.torrent"), PathBuf::from("new.torrent")],
        )
        .unwrap();
        assert_eq!(pending.len(), 16);
        assert_eq!(pending.last().unwrap(), &PathBuf::from("new.torrent"));
    }
}

/// Validate lexical paths, preserving native encoding and first-seen order.
/// This cannot establish existence, regular-file status, or trusted file contents.
/// An empty selection is an error; duplicates still count toward the input bound.
pub fn torrent_paths(
    inputs: impl IntoIterator<Item = PathBuf>,
) -> Result<Vec<PathBuf>, &'static str> {
    let mut paths = Vec::new();
    for (index, path) in inputs.into_iter().enumerate() {
        if index >= MAX_INTAKE_FILES {
            return Err(TOO_MANY);
        }
        let bytes = path.as_os_str().as_encoded_bytes();
        if bytes.len() > MAX_PATH_BYTES {
            return Err(TOO_LONG);
        }
        if bytes.is_empty()
            || bytes.contains(&0)
            || bytes.ends_with(b"/")
            || bytes.ends_with(b"/.")
            || (cfg!(windows) && bytes.ends_with(b"\\."))
            || (cfg!(windows) && bytes.ends_with(b"\\"))
            || bytes.starts_with(b"//")
            || bytes.starts_with(b"\\\\")
            || !path
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("torrent"))
        {
            return Err(INVALID_PATH);
        }
        if !paths.contains(&path) {
            paths.push(path);
        }
    }
    if paths.is_empty() {
        return Err(INVALID_PATH);
    }
    Ok(paths)
}

/// Parse arguments *after* the executable name (the caller must skip argv[0]).
/// Empty startup arguments are valid. `--` ends option parsing; unknown options
/// fail closed, and legacy macOS process-serial-number arguments are ignored.
/// At most 18 arguments are examined (16 paths plus `--` and one legacy token).
pub fn argument_paths(
    args: impl IntoIterator<Item = OsString>,
) -> Result<Vec<PathBuf>, &'static str> {
    let mut paths = Vec::new();
    let mut positional = false;
    for (index, arg) in args.into_iter().enumerate() {
        if index >= MAX_INTAKE_FILES + 2 {
            return Err(TOO_MANY);
        }
        if arg.as_encoded_bytes().len() > MAX_PATH_BYTES {
            return Err(TOO_LONG);
        }
        if !positional {
            if arg == "--" {
                positional = true;
                continue;
            }
            if arg.as_encoded_bytes().starts_with(b"-psn_") {
                continue;
            }
            if arg.as_encoded_bytes().starts_with(b"-") {
                return Err("Unsupported opener argument.");
            }
        }
        paths.push(match arg.to_str() {
            Some(text) => local_path(text)?,
            None => PathBuf::from(arg),
        });
        if paths.len() > MAX_INTAKE_FILES {
            return Err(TOO_MANY);
        }
    }
    if paths.is_empty() {
        Ok(paths)
    } else {
        torrent_paths(paths)
    }
}

/// Parse an OS open-file callback. URLs are consumed, never saved or logged.
pub fn opened_urls(urls: Vec<String>) -> Result<Vec<PathBuf>, &'static str> {
    if urls.len() > MAX_INTAKE_FILES {
        return Err(TOO_MANY);
    }
    let paths = urls
        .iter()
        .map(|text| local_path(text))
        .collect::<Result<Vec<_>, _>>()?;
    torrent_paths(paths)
}

fn local_path(text: &str) -> Result<PathBuf, &'static str> {
    if text.len() > MAX_PATH_BYTES {
        return Err(TOO_LONG);
    }
    if text
        .get(..7)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("file://"))
    {
        // URL parsing tolerates malformed percent escapes; intake does not.
        let bytes = text.as_bytes();
        for (index, byte) in bytes.iter().enumerate() {
            if *byte == b'%'
                && !bytes
                    .get(index + 1..index + 3)
                    .is_some_and(|hex| hex.iter().all(u8::is_ascii_hexdigit))
            {
                return Err(INVALID_PATH);
            }
        }
        let url = Url::parse(text).map_err(|_| INVALID_PATH)?;
        if url.scheme() != "file"
            || url.query().is_some()
            || url.fragment().is_some()
            || url.host_str().is_some_and(|host| host != "localhost")
            || !url.username().is_empty()
            || url.password().is_some()
            || url.port().is_some()
        {
            return Err(INVALID_PATH);
        }
        return url.to_file_path().map_err(|_| INVALID_PATH);
    }
    // A colon before the first path separator is a scheme, not a local path.
    // Windows drive paths are native paths, not single-letter URL schemes.
    let drive = cfg!(windows)
        && text.as_bytes().first().is_some_and(u8::is_ascii_alphabetic)
        && text.as_bytes().get(1) == Some(&b':')
        && text
            .as_bytes()
            .get(2)
            .is_some_and(|b| matches!(b, b'/' | b'\\'));
    if !drive
        && text
            .split(['/', '\\'])
            .next()
            .is_some_and(|part| part.contains(':'))
    {
        return Err(INVALID_PATH);
    }
    Ok(PathBuf::from(text))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn windows_drive_file_urls_roundtrip_but_remote_and_driveless_urls_do_not() {
        let path = PathBuf::from("C:\\Users\\Example\\Downloads\\日 a.torrent");
        assert_eq!(
            opened_urls(vec![
                "file:///C:/Users/Example/Downloads/%E6%97%A5%20a.torrent".into()
            ]),
            Ok(vec![path.clone()])
        );
        assert_eq!(
            argument_paths([path.as_os_str().to_owned()]),
            Ok(vec![path])
        );
        for value in [
            "file:///tmp/a.torrent",
            "file://server/share/a.torrent",
            "\\\\server\\share\\a.torrent",
            "\\\\?\\C:\\tmp\\a.torrent",
        ] {
            assert!(
                opened_urls(vec![value.into()]).is_err(),
                "unsupported path was accepted"
            );
        }
    }

    #[test]
    fn preserves_unicode_spaces_case_and_first_seen_order() {
        let first = PathBuf::from("/tmp/日本 語.TORRENT");
        let second = PathBuf::from("relative.torrent");
        assert_eq!(
            torrent_paths([first.clone(), second.clone(), first.clone()]),
            Ok(vec![first, second])
        );
    }

    #[test]
    fn no_filesystem_access_is_needed() {
        assert!(torrent_paths([PathBuf::from("/does-not-exist/file.torrent")]).is_ok());
        for path in [
            "",
            "a.txt",
            "a.torrent/",
            "a.torrent/..",
            "a.torrent/.",
            "//server/a.torrent",
            "a\0.torrent",
        ] {
            assert!(torrent_paths([PathBuf::from(path)]).is_err());
        }
        assert!(torrent_paths([]).is_err());
    }

    #[test]
    fn decodes_only_file_urls_and_rejects_remote_or_malformed_input() {
        let path = if cfg!(windows) {
            PathBuf::from("C:\\tmp\\日 a.torrent")
        } else {
            PathBuf::from("/tmp/日 a.torrent")
        };
        let url = Url::from_file_path(&path).unwrap();
        assert_eq!(
            opened_urls(vec![url.to_string(), path.to_str().unwrap().into()]),
            Ok(vec![path])
        );
        let localhost = if cfg!(windows) {
            "file://localhost/C:/tmp/a.torrent"
        } else {
            "file://localhost/tmp/a.torrent"
        };
        assert!(opened_urls(vec![localhost.into()]).is_ok());
        for value in [
            "https://host/a.torrent",
            "magnet:?xt=abc",
            "file://server/tmp/a.torrent",
            "file:/tmp/a.torrent",
            "file:///tmp/a%Q0.torrent",
            "file:///tmp/a%.torrent",
            "file:///tmp/a%00.torrent",
            "file:///tmp/a.torrent?q=x",
            "file:///tmp/a.torrent#x",
            "file://user@localhost/a.torrent",
            "foo:a.torrent",
        ] {
            assert!(opened_urls(vec![value.into()]).is_err(), "{value}");
        }
        assert!(opened_urls(vec![]).is_err());
    }

    #[test]
    fn bounds_raw_inputs_even_when_all_are_duplicates() {
        assert_eq!(
            torrent_paths(std::iter::repeat_n(PathBuf::from("a.torrent"), 16)),
            Ok(vec![PathBuf::from("a.torrent")])
        );
        assert_eq!(
            torrent_paths(std::iter::repeat_n(PathBuf::from("a.torrent"), 17)),
            Err(TOO_MANY)
        );
        assert_eq!(opened_urls(vec!["a.torrent".into(); 17]), Err(TOO_MANY));
        assert_eq!(
            opened_urls(vec![format!("{}.torrent", "a".repeat(MAX_PATH_BYTES))]),
            Err(TOO_LONG)
        );
    }

    #[test]
    fn argument_contract_and_options() {
        let path = if cfg!(windows) {
            PathBuf::from("C:\\tmp\\a b.torrent")
        } else {
            PathBuf::from("/tmp/a b.torrent")
        };
        let url = Url::from_file_path(&path).unwrap().to_string();
        let args = ["-psn_0_123", "--", "-name.torrent", &url].map(OsString::from);
        assert_eq!(
            argument_paths(args),
            Ok(vec![PathBuf::from("-name.torrent"), path])
        );
        assert_eq!(argument_paths([]), Ok(vec![]));
        assert!(argument_paths([OsString::from("--help")]).is_err());
        assert!(argument_paths([OsString::from("-name.torrent")]).is_err());
        assert_eq!(
            argument_paths([OsString::from("--"), OsString::from("-name.torrent")]),
            Ok(vec![PathBuf::from("-name.torrent")])
        );
        assert!(argument_paths(std::iter::repeat_n(OsString::from("-psn_0_1"), 19)).is_err());
        assert!(argument_paths(std::iter::repeat_n(OsString::from("a.torrent"), 17)).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn preserves_non_utf8_local_paths() {
        use std::os::unix::ffi::OsStringExt;
        let native = OsString::from_vec(b"/tmp/\xff.torrent".to_vec());
        assert_eq!(
            argument_paths([native.clone()]),
            Ok(vec![PathBuf::from(native)])
        );
        assert_eq!(
            opened_urls(vec!["file:///tmp/%FF.torrent".into()]),
            Ok(vec![PathBuf::from(OsString::from_vec(
                b"/tmp/\xff.torrent".to_vec()
            ))])
        );
    }
}
