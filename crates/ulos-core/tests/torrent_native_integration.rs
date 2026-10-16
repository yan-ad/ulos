//! Run explicitly with ULOS_TORRENT_WORKER and ULOS_TORRENT_FIXTURE pointing to
//! the freshly built native binaries: cargo test -p ulos-core --test
//! torrent_native_integration -- --ignored. All discovery/traffic is loopback.
use std::{
    collections::BTreeMap,
    io::{BufRead, BufReader},
    path::Path,
    process::{Child, Command, Stdio},
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};
use ulos_core::{tasks::TransferState, torrent::*};

#[derive(Clone)]
enum B {
    Bytes(Vec<u8>),
    Int(u64),
    List(Vec<B>),
    Dict(BTreeMap<Vec<u8>, B>),
}
fn decode(input: &[u8], pos: &mut usize) -> B {
    match input[*pos] {
        b'i' => {
            *pos += 1;
            let start = *pos;
            while input[*pos] != b'e' {
                *pos += 1;
            }
            let n = std::str::from_utf8(&input[start..*pos])
                .unwrap()
                .parse()
                .unwrap();
            *pos += 1;
            B::Int(n)
        }
        b'l' | b'd' => {
            let dict = input[*pos] == b'd';
            *pos += 1;
            let mut list = Vec::new();
            let mut map = BTreeMap::new();
            while input[*pos] != b'e' {
                let v = decode(input, pos);
                if dict {
                    let B::Bytes(k) = v else { panic!("key") };
                    map.insert(k, decode(input, pos));
                } else {
                    list.push(v);
                }
            }
            *pos += 1;
            if dict { B::Dict(map) } else { B::List(list) }
        }
        _ => {
            let start = *pos;
            while input[*pos] != b':' {
                *pos += 1;
            }
            let n: usize = std::str::from_utf8(&input[start..*pos])
                .unwrap()
                .parse()
                .unwrap();
            *pos += 1;
            let b = input[*pos..*pos + n].to_vec();
            *pos += n;
            B::Bytes(b)
        }
    }
}
fn encode(value: &B, out: &mut Vec<u8>) {
    match value {
        B::Int(n) => out.extend(format!("i{n}e").as_bytes()),
        B::Bytes(b) => {
            out.extend(format!("{}:", b.len()).as_bytes());
            out.extend(b);
        }
        B::List(l) => {
            out.push(b'l');
            for v in l {
                encode(v, out);
            }
            out.push(b'e');
        }
        B::Dict(d) => {
            out.push(b'd');
            for (k, v) in d {
                encode(&B::Bytes(k.clone()), out);
                encode(v, out);
            }
            out.push(b'e');
        }
    }
}
struct Seed(Child);
impl Drop for Seed {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn create(fixture: &Path, data: &Path, meta: &Path, mode: &str) {
    assert!(
        Command::new(fixture)
            .args(["--create-fixture"])
            .arg(data)
            .arg("--torrent-out")
            .arg(meta)
            .arg(mode)
            .status()
            .unwrap()
            .success()
    );
}

fn seed(fixture: &Path, meta: &Path, data_dir: &Path) -> (Seed, u16) {
    let mut seed = Seed(
        Command::new(fixture)
            .arg("--seed-test")
            .arg(meta)
            .arg("--data")
            .arg(data_dir)
            .args(["--listen", "127.0.0.1:0"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let mut line = String::new();
    BufReader::new(seed.0.stdout.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    let ready: serde_json::Value = serde_json::from_str(&line).unwrap();
    assert_eq!(ready["event"], "seed_ready");
    let port = u16::try_from(ready["port"].as_u64().unwrap()).unwrap();
    assert_ne!(port, 0);
    (seed, port)
}

fn local_magnet(fixture: &Path, meta: &Path, port: u16) -> String {
    let output = Command::new(fixture)
        .arg("--magnet-info")
        .arg(meta)
        .output()
        .unwrap();
    assert!(output.status.success());
    let info: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let mut uri = url::Url::parse(info["magnet"].as_str().unwrap()).unwrap();
    // Exercise percent-escaped explicit peers through the public parser and its
    // libtorrent normalization, not a worker-only connect-peer test hook.
    uri.query_pairs_mut()
        .append_pair("x.pe", &format!("127.0.0.1:{port}"));
    let uri = uri.to_string();
    assert!(uri.contains("x.pe=127.0.0.1%3A"));
    validate_magnet(&uri).unwrap();
    uri
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires freshly built native fixture and worker; explicitly run in native CI"]
async fn native_public_magnet_resolves_v1_v2_and_hybrid_without_payload_publication() {
    let fixture = std::env::var_os("ULOS_TORRENT_FIXTURE").expect("ULOS_TORRENT_FIXTURE required");
    assert!(std::env::var_os("ULOS_TORRENT_WORKER").is_some());
    for (mode, protocol) in [
        ("--v1", TorrentProtocol::V1),
        ("--v2", TorrentProtocol::V2),
        ("--hybrid", TorrentProtocol::Hybrid),
    ] {
        for size in [3, 4096] {
            let dir = tempfile::tempdir().unwrap();
            let seed_dir = dir.path().join("seed");
            std::fs::create_dir(&seed_dir).unwrap();
            let data = seed_dir.join("payload.bin");
            let payload: Vec<u8> = b"abc".iter().copied().cycle().take(size).collect();
            std::fs::write(&data, &payload).unwrap();
            let meta = dir.path().join("fixture.torrent");
            create(Path::new(&fixture), &data, &meta, mode);
            let raw = std::fs::read(&meta).unwrap();
            let expected = parse(raw.clone()).unwrap();
            assert_eq!(expected.tracker_count(), 0);
            let (_seed, port) = seed(Path::new(&fixture), &meta, &seed_dir);
            let uri = local_magnet(Path::new(&fixture), &meta, port);
            // No trackers, DHT, LSD or port mapping: the only discovery source
            // is the numeric loopback peer above.
            let source = tokio::time::timeout(Duration::from_secs(15), resolve_magnet(uri, false))
                .await
                .expect("public native magnet resolution timed out")
                .unwrap();
            assert_eq!(source.info_hash(), expected.info_hash(), "{mode}, {size}");
            assert_eq!(source.protocol(), protocol);
            assert_eq!(source.files(), expected.files());
            assert_eq!(source.file_count(), 1);
            assert_eq!(source.total_bytes(), size as u64);
            assert_eq!(source.files()[0].length, size as u64);
            assert!(!source.is_multi_file());
            assert_eq!(source.tracker_count(), 0);
            assert_eq!(std::fs::read(&data).unwrap(), payload);
            assert_eq!(std::fs::read(&meta).unwrap(), raw);
            // Resolver staging is privately owned under the system temp root,
            // not this fixture directory. Do not claim to inspect that staging;
            // prove that discovery did not publish into our supplied data tree.
            assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 2);
            assert_eq!(std::fs::read_dir(&seed_dir).unwrap().count(), 1);
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires freshly built native fixture and worker; explicitly run in native CI"]
async fn native_public_magnet_missing_v2_piece_layers_fails_promptly() {
    let fixture = std::env::var_os("ULOS_TORRENT_FIXTURE").expect("ULOS_TORRENT_FIXTURE required");
    assert!(std::env::var_os("ULOS_TORRENT_WORKER").is_some());
    for mode in ["--v2", "--hybrid"] {
        let dir = tempfile::tempdir().unwrap();
        let seed_dir = dir.path().join("seed");
        std::fs::create_dir(&seed_dir).unwrap();
        let data = seed_dir.join("payload.bin");
        let payload: Vec<u8> = (0..65_537).map(|i| (i % 251) as u8).collect();
        std::fs::write(&data, &payload).unwrap();
        let meta = dir.path().join("fixture.torrent");
        create(Path::new(&fixture), &data, &meta, mode);
        let raw = std::fs::read(&meta).unwrap();
        let expected = parse(raw.clone()).unwrap();
        assert_eq!(expected.tracker_count(), 0);
        let (_seed, port) = seed(Path::new(&fixture), &meta, &seed_dir);
        let uri = local_magnet(Path::new(&fixture), &meta, port);
        // BEP 9 does not supply the multi-piece BEP 52 layers. Fail closed
        // immediately rather than wait for the worker's 120-second timeout.
        let result =
            tokio::time::timeout(Duration::from_secs(10), resolve_magnet(uri, false)).await;
        if result.is_err() {
            // Dropping the resolving future signals owned child cleanup. Leave
            // the runtime alive briefly even on this failing timeout path.
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
        assert!(
            result
                .expect("missing piece layers hung instead of failing")
                .is_err()
        );
        assert_eq!(std::fs::read(&data).unwrap(), payload);
        assert_eq!(std::fs::read(&meta).unwrap(), raw);
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 2);
        assert_eq!(std::fs::read_dir(&seed_dir).unwrap().count(), 1);
    }
}

async fn terminal(task: &TorrentDownloadTask) -> TransferState {
    let mut states = task.subscribe();
    tokio::time::timeout(Duration::from_secs(60), async {
        loop {
            let state = states.borrow_and_update().clone();
            if state.is_terminal() {
                return state;
            }
            states.changed().await.unwrap();
        }
    })
    .await
    .expect("native task did not terminate")
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires freshly built native fixture and worker; explicitly run in native CI"]
async fn native_v2_and_hybrid_actual_task_publish_single_and_multi() {
    let fixture = std::env::var_os("ULOS_TORRENT_FIXTURE").expect("ULOS_TORRENT_FIXTURE required");
    assert!(
        std::env::var_os("ULOS_TORRENT_WORKER").is_some(),
        "ULOS_TORRENT_WORKER required"
    );
    for mode in ["--v2", "--hybrid"] {
        for multi in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let seed_dir = dir.path().join("seed");
            std::fs::create_dir(&seed_dir).unwrap();
            let data = seed_dir.join(if multi { "tree" } else { "file.bin" });
            let payload: Vec<u8> = (0..40_003).map(|i| (i % 251) as u8).collect();
            if multi {
                std::fs::create_dir(&data).unwrap();
                std::fs::write(data.join("a.bin"), &payload).unwrap();
                std::fs::write(data.join("b.bin"), b"second file").unwrap();
            } else {
                std::fs::write(&data, &payload).unwrap();
            }
            let meta = dir.path().join("fixture.torrent");
            create(Path::new(&fixture), &data, &meta, mode);
            let mut seed = Seed(
                Command::new(&fixture)
                    .arg("--seed-test")
                    .arg(&meta)
                    .arg("--data")
                    .arg(&seed_dir)
                    .args(["--listen", "127.0.0.1:0"])
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::null())
                    .spawn()
                    .unwrap(),
            );
            let mut line = String::new();
            BufReader::new(seed.0.stdout.take().unwrap())
                .read_line(&mut line)
                .unwrap();
            let ready: serde_json::Value = serde_json::from_str(&line).unwrap();
            let port = ready["port"].as_u64().unwrap() as u16;
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let tracker_url = format!("http://{}/announce", listener.local_addr().unwrap());
            let tracker = tokio::spawn(async move {
                loop {
                    let (mut socket, _) = listener.accept().await.unwrap();
                    let mut request = [0; 8192];
                    let _ = socket.read(&mut request).await;
                    let mut body = b"d8:intervali1e5:peers6:".to_vec();
                    body.extend([127, 0, 0, 1]);
                    body.extend(port.to_be_bytes());
                    body.push(b'e');
                    let header = format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    let _ = socket.write_all(header.as_bytes()).await;
                    let _ = socket.write_all(&body).await;
                }
            });
            let raw = std::fs::read(&meta).unwrap();
            let mut pos = 0;
            let mut metainfo = decode(&raw, &mut pos);
            assert_eq!(pos, raw.len());
            let B::Dict(ref mut root) = metainfo else {
                panic!("root")
            };
            root.insert(b"announce".to_vec(), B::Bytes(tracker_url.into_bytes()));
            let mut raw = Vec::new();
            encode(&metainfo, &mut raw);
            let source = parse(raw).unwrap();
            assert_eq!(
                source.protocol(),
                if mode == "--v2" {
                    TorrentProtocol::V2
                } else {
                    TorrentProtocol::Hybrid
                }
            );
            assert_eq!(source.is_multi_file(), multi);
            let output_dir = dir.path().join("downloads");
            std::fs::create_dir(&output_dir).unwrap();
            let destination = output_dir.join("published");
            // Exercise the public task control bridge against a real slow swarm,
            // and prove cancellation does not publish or retain staging.
            if mode == "--v2" && !multi {
                let mut slow = TorrentDownloadTask::start(
                    &tokio::runtime::Handle::current(),
                    parse(std::fs::read(&meta).unwrap()).unwrap(),
                    destination.clone(),
                    TorrentOptions {
                        settings: TorrentSettings {
                            allow_upload: true,
                            download_bytes_per_second: Some(1024),
                            ..Default::default()
                        },
                        ..Default::default()
                    },
                );
                tokio::time::timeout(Duration::from_secs(10), async {
                    loop {
                        if slow.telemetry().borrow().phase == TorrentPhase::Downloading
                            && slow.pause()
                        {
                            break;
                        }
                        tokio::time::sleep(Duration::from_millis(20)).await;
                    }
                })
                .await
                .expect("native task did not accept pause");
                tokio::time::timeout(Duration::from_secs(10), async {
                    loop {
                        if matches!(*slow.subscribe().borrow(), TransferState::Paused(_)) {
                            break;
                        }
                        tokio::time::sleep(Duration::from_millis(20)).await;
                    }
                })
                .await
                .expect("native task did not pause");
                assert_eq!(slow.telemetry().borrow().download_bytes_per_second, 0);
                assert!(slow.resume());
                slow.cancel();
                assert_eq!(terminal(&slow).await, TransferState::Cancelled);
                assert_eq!(std::fs::read_dir(&output_dir).unwrap().count(), 0);
            }
            let task = TorrentDownloadTask::start(
                &tokio::runtime::Handle::current(),
                source,
                destination.clone(),
                TorrentOptions {
                    settings: TorrentSettings {
                        allow_upload: true,
                        peer_limit: 20,
                        download_bytes_per_second: Some(1_048_576),
                        ..Default::default()
                    },
                    ..Default::default()
                },
            );
            assert!(task.set_peer_limit(25));
            assert!(task.set_limits(Some(2_097_152), Some(1_048_576)));
            let state = terminal(&task).await;
            tracker.abort();
            assert!(
                matches!(state, TransferState::Completed(_)),
                "{mode}, multi={multi}: {state:?}"
            );
            assert_eq!(
                std::fs::read(if multi {
                    destination.join("a.bin")
                } else {
                    destination.clone()
                })
                .unwrap(),
                payload
            );
            if multi {
                assert_eq!(
                    std::fs::read(destination.join("b.bin")).unwrap(),
                    b"second file"
                );
            }
            assert_eq!(
                std::fs::read_dir(&output_dir).unwrap().count(),
                1,
                "staging must be gone before completion"
            );
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires freshly built native fixture; explicitly run in native CI"]
async fn native_v2_without_upload_consent_fails_before_staging() {
    let fixture = std::env::var_os("ULOS_TORRENT_FIXTURE").expect("ULOS_TORRENT_FIXTURE required");
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("file");
    std::fs::write(&data, b"hello").unwrap();
    let meta = dir.path().join("input.torrent");
    create(Path::new(&fixture), &data, &meta, "--v2");
    let source = parse(std::fs::read(&meta).unwrap()).unwrap();
    let outputs = dir.path().join("out");
    std::fs::create_dir(&outputs).unwrap();
    let task = TorrentDownloadTask::start(
        &tokio::runtime::Handle::current(),
        source,
        outputs.join("result"),
        TorrentOptions::default(),
    );
    assert_eq!(
        terminal(&task).await,
        TransferState::Failed(TorrentError::UploadRequired.to_string())
    );
    assert_eq!(std::fs::read_dir(&outputs).unwrap().count(), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn unsupported_v1_settings_fail_honestly_without_staging() {
    // Valid v1, no trackers: these settings must fail before engine construction,
    // rather than silently falling back to defaults or creating staging files.
    let mut bytes = b"d4:infod6:lengthi1e4:name1:a12:piece lengthi16384e6:pieces20:".to_vec();
    bytes.extend([0; 20]);
    bytes.extend(b"ee");
    let source = parse(bytes).unwrap();
    for settings in [
        TorrentSettings {
            dht: true,
            ..Default::default()
        },
        TorrentSettings {
            download_bytes_per_second: Some(1),
            ..Default::default()
        },
        TorrentSettings {
            peer_limit: 0,
            ..Default::default()
        },
        TorrentSettings {
            allow_upload: true,
            upload_bytes_per_second: Some(0),
            ..Default::default()
        },
    ] {
        let invalid_bounds =
            settings.peer_limit == 0 || settings.upload_bytes_per_second == Some(0);
        let dir = tempfile::tempdir().unwrap();
        let task = TorrentDownloadTask::start(
            &tokio::runtime::Handle::current(),
            source.clone(),
            dir.path().join("out"),
            TorrentOptions {
                settings,
                ..Default::default()
            },
        );
        let expected = if invalid_bounds {
            TorrentError::Limit
        } else {
            TorrentError::UploadRequired
        };
        assert_eq!(
            terminal(&task).await,
            TransferState::Failed(expected.to_string())
        );
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }
}
