use ulos_core::migration::{self, MigrationError, MigrationFormat::*};

#[test]
fn actual_ef2_blocks_ignore_only_recognized_headers() {
    let batch = migration::parse(b"<\r\nhttps://example.org/a?signature=private\r\nreferer: https://example.org/\r\nUser-Agent: a browser with spaces\r\n>\r\n", IdmEf2).unwrap();
    assert_eq!(batch.entries.len(), 1);
    assert_eq!(batch.ignored_fields, 2);
    let output = migration::encode(&[batch.entries[0].source.clone()], IdmEf2).unwrap();
    assert_eq!(
        output,
        b"<\r\nhttps://example.org/a?signature=private\r\n>\r\n"
    );
    for field in [
        "cookie: secret",
        "Authorization: secret",
        "postdata: secret",
        "unknown: secret",
    ] {
        let input = format!("<\r\nhttps://example.org/a\r\n{field}\r\n>\r\n");
        assert!(matches!(
            migration::parse(input.as_bytes(), IdmEf2),
            Err(MigrationError::UnsupportedField)
        ));
    }
}

#[test]
fn rejects_raw_whitespace_credentials_controls_and_bounds() {
    for input in [
        " https://example.org/a",
        "https://example.org/a b",
        "https://example.org/a\t",
        "https://user:secret@example.org/a",
        "file:///a",
        "magnet:?a",
        "https://example.org/\0a",
    ] {
        assert!(migration::parse(input.as_bytes(), Text).is_err());
        assert!(migration::encode(&[input.into()], Text).is_err());
    }
    assert!(matches!(
        migration::parse(&vec![b'a'; migration::MAX_BYTES + 1], Text),
        Err(MigrationError::TooLarge)
    ));
    assert!(matches!(
        migration::parse("https://example.org/a\n".repeat(501).as_bytes(), Text),
        Err(MigrationError::TooManyEntries)
    ));
    assert!(
        migration::parse(
            format!("https://example.org/{}", "a".repeat(migration::MAX_LINE)).as_bytes(),
            Text
        )
        .is_err()
    );
    assert!(migration::parse(b"<\nhttps://example.org/a\n", IdmEf2).is_err());
}

#[test]
fn bounded_file_io_and_no_clobber_publication() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("export.ef2");
    let sources = vec!["https://example.org/a".into()];
    migration::export_file(&path, &sources, IdmEf2).unwrap();
    assert_eq!(
        migration::read_file(&path, IdmEf2).unwrap().entries[0].source,
        sources[0]
    );
    let original = std::fs::read(&path).unwrap();
    assert!(migration::export_file(&path, &["https://example.org/b".into()], Text).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), original);
    assert!(migration::read_file(dir.path(), Text).is_err());
    let large = dir.path().join("large");
    std::fs::File::create(&large)
        .unwrap()
        .set_len(migration::MAX_BYTES as u64 + 1)
        .unwrap();
    assert!(matches!(
        migration::read_file(&large, Text),
        Err(MigrationError::TooLarge)
    ));
    #[cfg(unix)]
    {
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(&path, &link).unwrap();
        assert!(migration::read_file(&link, Text).is_err());
        assert!(migration::export_file(&link, &sources, Text).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), original);
    }
    assert_eq!(
        std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| e.file_name().to_string_lossy().starts_with(".ulos-export-"))
            .count(),
        0
    );
}

#[test]
fn simultaneous_exports_have_exactly_one_winner() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("race.txt");
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let workers: Vec<_> = ["https://example.org/a", "https://example.org/b"]
        .into_iter()
        .map(|source| {
            let path = path.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                migration::export_file(&path, &[source.into()], Text).is_ok()
            })
        })
        .collect();
    assert_eq!(
        workers
            .into_iter()
            .filter_map(|worker| worker.join().ok())
            .filter(|ok| *ok)
            .count(),
        1
    );
    let content = std::fs::read_to_string(path).unwrap();
    assert!(content == "https://example.org/a\r\n" || content == "https://example.org/b\r\n");
}
