use sha1::{Digest, Sha1};
use std::{collections::BTreeMap, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};
use ulos_core::{tasks::TransferState, torrent::*};

#[derive(Clone)]
enum B {
    S(Vec<u8>),
    I(u64),
    L(Vec<B>),
    D(BTreeMap<Vec<u8>, B>),
}
fn s(v: impl AsRef<[u8]>) -> B {
    B::S(v.as_ref().to_vec())
}
fn d(entries: Vec<(&str, B)>) -> B {
    B::D(
        entries
            .into_iter()
            .map(|(k, v)| (k.as_bytes().to_vec(), v))
            .collect(),
    )
}
fn encode(v: &B, out: &mut Vec<u8>) {
    match v {
        B::S(v) => {
            out.extend_from_slice(format!("{}:", v.len()).as_bytes());
            out.extend(v);
        }
        B::I(v) => out.extend_from_slice(format!("i{v}e").as_bytes()),
        B::L(v) => {
            out.push(b'l');
            for v in v {
                encode(v, out);
            }
            out.push(b'e');
        }
        B::D(v) => {
            out.push(b'd');
            for (k, v) in v {
                encode(&s(k), out);
                encode(v, out);
            }
            out.push(b'e');
        }
    }
}
fn bytes(v: &B) -> Vec<u8> {
    let mut out = Vec::new();
    encode(v, &mut out);
    out
}
fn info(name: &str, payload: &[u8], files: Option<Vec<(&str, u64)>>) -> B {
    let pieces: Vec<u8> = payload
        .chunks(16384)
        .flat_map(|c| Sha1::digest(c).to_vec())
        .collect();
    let mut fields = vec![
        ("name", s(name)),
        ("piece length", B::I(16384)),
        ("pieces", s(pieces)),
    ];
    if let Some(files) = files {
        fields.push((
            "files",
            B::L(
                files
                    .into_iter()
                    .map(|(path, len)| {
                        d(vec![
                            ("length", B::I(len)),
                            ("path", B::L(path.split('/').map(s).collect())),
                        ])
                    })
                    .collect(),
            ),
        ));
    } else {
        fields.push(("length", B::I(payload.len() as u64)));
    }
    d(fields)
}
fn metainfo(info: B, tracker: Option<&str>) -> Vec<u8> {
    let mut fields = vec![("info", info)];
    if let Some(t) = tracker {
        fields.push(("announce", s(t)));
    }
    bytes(&d(fields))
}
fn fixture() -> TorrentSource {
    parse(metainfo(
        info("sample.bin", b"legitimate fixture", None),
        None,
    ))
    .unwrap()
}
async fn terminal(task: &TorrentDownloadTask) -> TransferState {
    let mut rx = task.subscribe();
    tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            let state = rx.borrow_and_update().clone();
            if state.is_terminal() {
                return state;
            }
            rx.changed().await.unwrap();
        }
    })
    .await
    .unwrap()
}

#[test]
fn offline_manifest_hash_and_limits() {
    let source = fixture();
    assert_eq!(source.name(), "sample.bin");
    assert_eq!(source.file_count(), 1);
    assert_eq!(source.total_bytes(), 18);
    assert_eq!(source.tracker_count(), 0);
    assert!(!source.is_multi_file());
    let i = info("sample.bin", b"legitimate fixture", None);
    assert_eq!(source.info_hash(), format!("{:x}", Sha1::digest(bytes(&i))));
    assert!(parse(vec![0; MAX_METAINFO_BYTES + 1]).is_err());
    let deep = [vec![b'l'; 100_000], vec![b'e'; 100_000]].concat();
    assert!(parse(deep).is_err());
    assert!(parse(metainfo(info("empty", b"", None), None)).is_err());
    assert!(parse(b"d4:infod4:name1:xeetrailing".to_vec()).is_err());
}
#[test]
fn unsafe_paths_trackers_and_conflicts_are_rejected() {
    for name in [
        "..",
        "/root",
        "C:\\file",
        "NUL.txt",
        "bad.",
        "a\u{202e}b",
        "CON .txt",
        "a:b",
        "x\0y",
    ] {
        assert!(
            parse(metainfo(info(name, b"x", None), None)).is_err(),
            "{name}"
        );
    }
    for paths in [
        vec![("a", 1), ("A", 1)],
        vec![("a", 1), ("a/b", 1)],
        vec![("a/b", 1), ("a", 1)],
        vec![("../escape", 2)],
        vec![("é", 1), ("e\u{301}", 1)],
    ] {
        assert!(parse(metainfo(info("root", b"xy", Some(paths)), None)).is_err());
    }
    for tracker in [
        "file:///secret",
        "https://user:secret@example.test/announce",
        "https://example.test/announce#secret",
    ] {
        let err = parse(metainfo(info("ok", b"x", None), Some(tracker)))
            .err()
            .unwrap();
        assert!(!err.to_string().contains("secret"));
    }
    let mut i = info("ok", b"x", None);
    if let B::D(ref mut map) = i {
        map.insert(b"attr".to_vec(), s("l"));
    }
    assert!(parse(metainfo(i, None)).is_err());
    assert_eq!(
        parse(metainfo(
            info("ok", b"x", None),
            Some("https://example.test/announce?passkey=secret")
        ))
        .unwrap()
        .tracker_count(),
        1
    );
}
#[test]
fn shared_directory_spelling_must_be_consistent_before_engine_io() {
    for paths in [
        vec![("A/one", 1), ("a/two", 1)],
        vec![("a/one", 1), ("A/two", 1)],
        vec![("é/one", 1), ("e\u{301}/two", 1)],
        vec![("e\u{301}/one", 1), ("é/two", 1)],
        vec![("root/A/one", 1), ("root/a/two", 1)],
    ] {
        assert!(matches!(
            parse(metainfo(info("root", b"xy", Some(paths)), None)),
            Err(TorrentError::UnsafePath)
        ));
    }
    // A shared directory with identical spelling remains valid.
    for parent in ["A", "é", "e\u{301}"] {
        let one = format!("{parent}/one");
        let two = format!("{parent}/two");
        assert!(
            parse(metainfo(
                info("root", b"xy", Some(vec![(&one, 1), (&two, 1)])),
                None,
            ))
            .is_ok()
        );
    }
}

#[tokio::test]
async fn load_regular_only() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("test.torrent");
    std::fs::write(&p, metainfo(info("ok", b"x", None), None)).unwrap();
    assert!(load(p.clone()).await.is_ok());
    assert!(load(dir.path().to_path_buf()).await.is_err());
    #[cfg(unix)]
    {
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(p, &link).unwrap();
        assert!(load(link).await.is_err());
    }
}
#[tokio::test(flavor = "multi_thread")]
async fn reject_and_overwrite_fail_before_network() {
    let dir = tempfile::tempdir().unwrap();
    let destination = dir.path().join("existing");
    std::fs::write(&destination, b"original").unwrap();
    for collision in [CollisionPolicy::Reject, CollisionPolicy::Overwrite] {
        let task = TorrentDownloadTask::start(
            &tokio::runtime::Handle::current(),
            fixture(),
            destination.clone(),
            TorrentOptions { collision },
        );
        assert!(matches!(terminal(&task).await, TransferState::Failed(_)));
        assert_eq!(std::fs::read(&destination).unwrap(), b"original");
    }
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
}
#[tokio::test(flavor = "multi_thread")]
async fn cancel_and_drop_wait_for_cleanup_before_terminal() {
    let dir = tempfile::tempdir().unwrap();
    for drop_task in [false, true] {
        let mut task = TorrentDownloadTask::start(
            &tokio::runtime::Handle::current(),
            fixture(),
            dir.path().join("out"),
            TorrentOptions::default(),
        );
        let mut rx = task.subscribe();
        tokio::time::sleep(Duration::from_millis(250)).await;
        if drop_task {
            drop(task);
        } else {
            task.cancel();
            drop(task);
        }
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let state = rx.borrow_and_update().clone();
                if state.is_terminal() {
                    assert_eq!(state, TransferState::Cancelled);
                    break;
                }
                rx.changed().await.unwrap();
            }
        })
        .await
        .unwrap();
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }
}

// A handcrafted local BitTorrent v1 peer, not a second engine (upload feature is
// disabled in this crate). Tracker compact response points only to loopback.
async fn seed(stream: TcpStream, payload: Vec<u8>) {
    seed_with_delay(stream, payload, Duration::ZERO).await;
}
async fn seed_with_delay(mut stream: TcpStream, payload: Vec<u8>, delay: Duration) {
    let mut handshake = [0u8; 68];
    if stream.read_exact(&mut handshake).await.is_err() {
        return;
    }
    handshake[48..].copy_from_slice(b"-FIXTURE-12345678901");
    if stream.write_all(&handshake).await.is_err() {
        return;
    }
    let count = payload.len().div_ceil(16384);
    let mut bitfield = vec![0u8; count.div_ceil(8)];
    for n in 0..count {
        bitfield[n / 8] |= 1 << (7 - n % 8);
    }
    let mut msg = vec![5];
    msg.extend(bitfield);
    send(&mut stream, &msg).await;
    send(&mut stream, &[1]).await;
    while let Ok(len) = stream.read_u32().await {
        if len > 100_000 {
            break;
        }
        if len == 0 {
            continue;
        }
        let mut msg = vec![0; len as usize];
        if stream.read_exact(&mut msg).await.is_err() {
            break;
        }
        if msg[0] == 6 && msg.len() == 13 {
            let index = u32::from_be_bytes(msg[1..5].try_into().unwrap()) as usize;
            let offset = u32::from_be_bytes(msg[5..9].try_into().unwrap()) as usize;
            let len = u32::from_be_bytes(msg[9..13].try_into().unwrap()) as usize;
            let start = index * 16384 + offset;
            if start + len > payload.len() {
                break;
            }
            tokio::time::sleep(delay).await;
            let mut response = vec![7];
            response.extend_from_slice(&msg[1..9]);
            response.extend_from_slice(&payload[start..start + len]);
            send(&mut stream, &response).await;
        }
    }
}
async fn send(stream: &mut TcpStream, msg: &[u8]) {
    let _ = stream.write_u32(msg.len() as u32).await;
    let _ = stream.write_all(msg).await;
}
async fn local_source(
    payload: Vec<u8>,
    multi: bool,
) -> (
    TorrentSource,
    tokio::task::JoinHandle<()>,
    tokio::task::JoinHandle<()>,
) {
    let peer = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = peer.local_addr().unwrap().port();
    let seeder = tokio::spawn(async move {
        while let Ok((stream, _)) = peer.accept().await {
            let payload = payload.clone();
            tokio::spawn(seed(stream, payload));
        }
    });
    let tracker = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/announce", tracker.local_addr().unwrap());
    let tracker_task = tokio::spawn(async move {
        while let Ok((mut stream, _)) = tracker.accept().await {
            let mut request = vec![0; 16384];
            let _ = stream.read(&mut request).await;
            let [hi, lo] = port.to_be_bytes();
            let body = bytes(&d(vec![
                ("interval", B::I(1)),
                ("peers", s([127, 0, 0, 1, hi, lo])),
            ]));
            let headers = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(headers.as_bytes()).await;
            let _ = stream.write_all(&body).await;
        }
    });
    let payload: Vec<u8> = (0..40_000).map(|i| (i % 251) as u8).collect();
    let files = multi.then_some(vec![("nested/one.bin", 20_000), ("two.bin", 20_000)]);
    let source = parse(metainfo(info("sample.bin", &payload, files), Some(&url))).unwrap();
    (source, seeder, tracker_task)
}
#[tokio::test(flavor = "multi_thread")]
async fn verified_local_peer_single_multi_and_rename() {
    for multi in [false, true] {
        let payload: Vec<u8> = (0..40_000).map(|i| (i % 251) as u8).collect();
        let (source, seeder, tracker) = local_source(payload.clone(), multi).await;
        let dir = tempfile::tempdir().unwrap();
        let destination = dir.path().join("final.bin");
        if multi {
            std::fs::create_dir(&destination).unwrap();
            std::fs::write(destination.join("original"), b"original").unwrap();
        } else {
            std::fs::write(&destination, b"original").unwrap();
        }
        let task = TorrentDownloadTask::start(
            &tokio::runtime::Handle::current(),
            source,
            destination.clone(),
            TorrentOptions {
                collision: CollisionPolicy::Rename,
            },
        );
        let state = terminal(&task).await;
        let TransferState::Completed(receipt) = state else {
            panic!("{state:?}");
        };
        assert_eq!(receipt.downloaded_bytes, 40000);
        assert_ne!(receipt.destination, destination);
        if multi {
            assert_eq!(
                std::fs::read(receipt.destination.join("nested/one.bin")).unwrap(),
                payload[..20000]
            );
            assert_eq!(
                std::fs::read(receipt.destination.join("two.bin")).unwrap(),
                payload[20000..]
            );
            assert_eq!(
                std::fs::read(destination.join("original")).unwrap(),
                b"original"
            );
        } else {
            assert_eq!(std::fs::read(&receipt.destination).unwrap(), payload);
            assert_eq!(std::fs::read(destination).unwrap(), b"original");
        }
        assert_eq!(task.telemetry().borrow().uploaded_bytes, 0);
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 2);
        seeder.abort();
        tracker.abort();
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn actual_pause_resume_retains_slot() {
    let dir = tempfile::tempdir().unwrap();
    let mut task = TorrentDownloadTask::start(
        &tokio::runtime::Handle::current(),
        fixture(),
        dir.path().join("out"),
        TorrentOptions::default(),
    );
    let mut telemetry = task.telemetry();
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if telemetry.borrow_and_update().phase == TorrentPhase::Downloading {
                break;
            }
            telemetry.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
    assert!(task.pause());
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if telemetry.borrow_and_update().phase == TorrentPhase::Paused {
                break;
            }
            telemetry.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
    assert!(matches!(
        *task.subscribe().borrow(),
        TransferState::Paused(_)
    ));
    {
        let sample = telemetry.borrow().clone();
        assert_eq!(sample.download_bytes_per_second, 0);
        assert_eq!(sample.upload_bytes_per_second, 0);
        assert_eq!(sample.peers, 0);
        assert_eq!(sample.connecting_peers, 0);
        assert_eq!(sample.queued_peers, 0);
        assert_eq!(sample.stalled_for_secs, 0);
    }
    assert!(task.resume());
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if telemetry.borrow_and_update().phase == TorrentPhase::Downloading {
                break;
            }
            telemetry.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
    task.cancel();
    assert_eq!(terminal(&task).await, TransferState::Cancelled);
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
}

#[test]
fn bounded_manifests_and_unicode_paths() {
    let source = parse(metainfo(
        info("資料", b"ab", Some(vec![("café/一.txt", 1), ("二.txt", 1)])),
        None,
    ))
    .unwrap();
    assert_eq!(
        source.files()[0].path,
        std::path::Path::new("café").join("一.txt")
    );
    let too_many = vec![("same", 0); 4097];
    assert!(parse(metainfo(info("root", b"x", Some(too_many)), None)).is_err());
    assert!(parse(metainfo(info(&"界".repeat(61), b"x", None), None)).is_err());
    let mut i = info("ok", b"x", None);
    if let B::D(ref mut map) = i {
        map.insert(b"piece length".to_vec(), B::I(0));
    }
    assert!(parse(metainfo(i, None)).is_err());
    let mut i = info("ok", b"x", None);
    if let B::D(ref mut map) = i {
        map.remove(b"pieces".as_slice());
        map.insert(b"meta version".to_vec(), B::I(2));
    }
    assert!(parse(metainfo(i, None)).is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn corrupt_peer_payload_is_not_reported_as_verified_or_published() {
    let (source, seeder, tracker) = local_source(vec![0; 40_000], false).await;
    let dir = tempfile::tempdir().unwrap();
    let destination = dir.path().join("out");
    let mut task = TorrentDownloadTask::start(
        &tokio::runtime::Handle::current(),
        source,
        destination.clone(),
        TorrentOptions::default(),
    );
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert_eq!(task.telemetry().borrow().downloaded_bytes, 0);
    assert!(!destination.exists());
    task.cancel();
    assert_eq!(terminal(&task).await, TransferState::Cancelled);
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    seeder.abort();
    tracker.abort();
}

#[test]
fn v1_hybrid_and_symlink_extensions_are_explicit() {
    let mut hybrid = info("hybrid", b"x", None);
    if let B::D(ref mut map) = hybrid {
        map.insert(b"meta version".to_vec(), B::I(2));
        map.insert(b"file tree".to_vec(), d(vec![]));
    }
    assert!(parse(metainfo(hybrid, None)).is_ok());
    let mut symlink = info("ok", b"x", None);
    if let B::D(ref mut map) = symlink {
        map.insert(b"symlink path".to_vec(), B::L(vec![s("elsewhere")]));
    }
    assert!(parse(metainfo(symlink, None)).is_err());
    let metadata = d(vec![
        (
            "announce-list",
            B::L(vec![B::L(vec![s("http://127.0.0.1/announce"); 65])]),
        ),
        ("info", info("ok", b"x", None)),
    ]);
    assert!(parse(bytes(&metadata)).is_err());
}

#[test]
fn telemetry_labels_and_redacted_diagnostics() {
    let mut t = TorrentTelemetry::default();
    assert_eq!(t.activity_label(), "Preparing torrent");
    assert_eq!(t.diagnostic(), None);
    for (phase, label) in [
        (TorrentPhase::Checking, "Checking"),
        (TorrentPhase::Paused, "Paused"),
        (TorrentPhase::Publishing, "Verifying / publishing"),
    ] {
        t.phase = phase;
        assert_eq!(t.activity_label(), label);
        assert_eq!(t.diagnostic(), None);
    }
    t.phase = TorrentPhase::Downloading;
    assert_eq!(t.activity_label(), "No peer discovery");
    assert!(t.diagnostic().unwrap().contains("No trackers"));
    t.tracker_count = 1;
    assert_eq!(t.activity_label(), "Searching for peers");
    assert!(t.diagnostic().unwrap().contains("no peer candidates"));
    t.seen_peers = 2;
    assert!(t.diagnostic().unwrap().contains("peers were seen"));
    t.connecting_peers = 1;
    assert_eq!(t.activity_label(), "Connecting to peers");
    t.connecting_peers = 0;
    t.queued_peers = 1;
    assert_eq!(t.activity_label(), "Connecting to peers");
    t.queued_peers = 0;
    t.peers = 1;
    assert_eq!(t.activity_label(), "Waiting for peers");
    t.download_bytes_per_second = 42;
    assert_eq!(t.activity_label(), "Downloading");
    t.stalled_for_secs = 15;
    assert_eq!(t.activity_label(), "Stalled");
    assert!(t.diagnostic().unwrap().contains("may be choked"));
    t.phase = TorrentPhase::Paused;
    assert_eq!(t.activity_label(), "Paused");
    assert_eq!(t.diagnostic(), None);
}

async fn downloading_sample(task: &TorrentDownloadTask) -> TorrentTelemetry {
    let mut telemetry = task.telemetry();
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let sample = telemetry.borrow_and_update().clone();
            if sample.phase == TorrentPhase::Downloading {
                return sample;
            }
            telemetry.changed().await.unwrap();
        }
    })
    .await
    .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn trackerless_and_empty_or_failed_local_tracker_have_truthful_diagnostics() {
    for status in [None, Some(200), Some(500)] {
        let tracker = TcpListener::bind("127.0.0.1:0").await.unwrap();
        // A synthetic secret must never escape via telemetry diagnostics.
        let url = format!(
            "http://{}/announce?passkey=fixture-secret",
            tracker.local_addr().unwrap()
        );
        let requests = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
        let observed = requests.clone();
        let server = tokio::spawn(async move {
            while let Ok((mut stream, _)) = tracker.accept().await {
                let mut request = vec![0; 16384];
                let _ = stream.read(&mut request).await;
                observed.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let body = bytes(&d(vec![("interval", B::I(1)), ("peers", s([]))]));
                let headers = format!(
                    "HTTP/1.1 {} Fixture\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    status.unwrap_or(200),
                    body.len()
                );
                let _ = stream.write_all(headers.as_bytes()).await;
                let _ = stream.write_all(&body).await;
            }
        });
        let dir = tempfile::tempdir().unwrap();
        let source = parse(metainfo(
            info("fixture.bin", b"local only", None),
            status.map(|_| url.as_str()),
        ))
        .unwrap();
        let mut task = TorrentDownloadTask::start(
            &tokio::runtime::Handle::current(),
            source,
            dir.path().join("out"),
            TorrentOptions::default(),
        );
        let initial = downloading_sample(&task).await;
        assert_eq!(initial.tracker_count, usize::from(status.is_some()));
        if status.is_some() {
            tokio::time::timeout(Duration::from_secs(5), async {
                while requests.load(std::sync::atomic::Ordering::Relaxed) == 0 {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            })
            .await
            .unwrap();
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
        let sample = task.telemetry().borrow().clone();
        assert_eq!(sample.peers, 0);
        assert_eq!(sample.seen_peers, 0);
        assert_eq!(sample.download_bytes_per_second, 0);
        assert_eq!(sample.upload_bytes_per_second, 0);
        assert_eq!(
            sample.activity_label(),
            if status.is_none() {
                "No peer discovery"
            } else {
                "Searching for peers"
            }
        );
        let diagnostic = sample.diagnostic().unwrap();
        assert!(diagnostic.to_lowercase().contains("tracker-only"));
        assert!(!diagnostic.contains("fixture-secret"));
        if status.is_some() {
            assert!(diagnostic.contains("failure is not confirmed"));
        }
        task.cancel();
        assert_eq!(terminal(&task).await, TransferState::Cancelled);
        let stopped = task.telemetry().borrow().clone();
        assert_eq!(stopped.peers, 0);
        assert_eq!(stopped.download_bytes_per_second, 0);
        server.abort();
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn slow_local_payload_reports_real_peers_and_binary_byte_rates() {
    // Long enough for librqbit's one-second estimator snapshots; no public peers.
    let payload: Vec<u8> = (0..128 * 16384).map(|i| (i % 251) as u8).collect();
    let peer = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = peer.local_addr().unwrap().port();
    let seed_payload = payload.clone();
    let seeder = tokio::spawn(async move {
        let (stream, _) = peer.accept().await.unwrap();
        seed_with_delay(stream, seed_payload, Duration::from_millis(70)).await;
    });
    let tracker = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/announce", tracker.local_addr().unwrap());
    let server = tokio::spawn(async move {
        while let Ok((mut stream, _)) = tracker.accept().await {
            let mut request = vec![0; 16384];
            let _ = stream.read(&mut request).await;
            let [hi, lo] = port.to_be_bytes();
            let body = bytes(&d(vec![
                ("interval", B::I(1)),
                ("peers", s([127, 0, 0, 1, hi, lo])),
            ]));
            let headers = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(headers.as_bytes()).await;
            let _ = stream.write_all(&body).await;
        }
    });
    let source = parse(metainfo(info("slow.bin", &payload, None), Some(&url))).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let task = TorrentDownloadTask::start(
        &tokio::runtime::Handle::current(),
        source,
        dir.path().join("out"),
        TorrentOptions::default(),
    );
    let mut telemetry = task.telemetry();
    tokio::time::timeout(Duration::from_secs(8), async {
        loop {
            let sample = telemetry.borrow_and_update().clone();
            assert_eq!(sample.upload_bytes_per_second, 0);
            if sample.download_bytes_per_second > 0 {
                assert_eq!(sample.phase, TorrentPhase::Downloading);
                assert!(sample.peers > 0);
                assert!(sample.seen_peers > 0);
                assert_eq!(sample.tracker_count, 1);
                assert!(sample.downloaded_bytes > 0);
                assert!(sample.downloaded_bytes < sample.total_bytes);
                assert!(sample.stalled_for_secs < 15);
                break;
            }
            telemetry.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
    assert!(matches!(terminal(&task).await, TransferState::Completed(_)));
    assert_eq!(std::fs::read(dir.path().join("out")).unwrap(), payload);
    let stopped = telemetry.borrow().clone();
    assert_eq!(stopped.phase, TorrentPhase::Publishing);
    assert_eq!(stopped.tracker_count, 1);
    assert_eq!(stopped.peers, 0);
    assert_eq!(stopped.connecting_peers, 0);
    assert_eq!(stopped.queued_peers, 0);
    assert_eq!(stopped.download_bytes_per_second, 0);
    assert_eq!(stopped.upload_bytes_per_second, 0);
    seeder.abort();
    server.abort();
}
