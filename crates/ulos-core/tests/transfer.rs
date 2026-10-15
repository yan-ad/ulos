//! Local-only controlled transfer contract tests. No external HTTP service.
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::{Barrier, watch},
    task::JoinHandle,
};
use ulos_core::{
    DownloadError,
    transfer::{
        self, CollisionPolicy, MAX_CONNECTIONS, ResumeCapability, TransferControl, TransferOptions,
        TransferTelemetry,
    },
};

#[derive(Clone, Copy)]
enum Mode {
    Range,
    Full,
    Weak,
    Malformed,
    Changed,
    Truncated,
}

struct Server {
    source: String,
    requests: Arc<Mutex<Vec<String>>>,
    maximum: Arc<AtomicUsize>,
    task: JoinHandle<()>,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn server(mode: Mode, body: Vec<u8>, concurrent: usize, slow: bool) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let source = format!(
        "http://{}/file?private=SECRET",
        listener.local_addr().unwrap()
    );
    let requests = Arc::new(Mutex::new(Vec::new()));
    let maximum = Arc::new(AtomicUsize::new(0));
    let active = Arc::new(AtomicUsize::new(0));
    let barrier = Arc::new(Barrier::new(concurrent.max(1)));
    let recorded = requests.clone();
    let observed = maximum.clone();
    let body = Arc::new(body);
    let task = tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                break;
            };
            let (recorded, observed, active, barrier, body) = (
                recorded.clone(),
                observed.clone(),
                active.clone(),
                barrier.clone(),
                body.clone(),
            );
            tokio::spawn(async move {
                let mut bytes = Vec::new();
                loop {
                    let mut byte = [0];
                    if stream.read_exact(&mut byte).await.is_err() {
                        return;
                    }
                    bytes.push(byte[0]);
                    if bytes.ends_with(b"\r\n\r\n") {
                        break;
                    }
                    if bytes.len() > 16_384 {
                        return;
                    }
                }
                let request = String::from_utf8(bytes).unwrap();
                recorded.lock().unwrap().push(request.clone());
                let lowercase = request.to_ascii_lowercase();
                let resumed = lowercase.contains("if-range:");
                let range = lowercase
                    .lines()
                    .find_map(|line| line.strip_prefix("range: bytes="));
                let partial = range.is_some()
                    && !matches!(mode, Mode::Full)
                    && !(resumed && matches!(mode, Mode::Changed));
                let (start, end) = if partial {
                    let (start, end) = range.unwrap().split_once('-').unwrap();
                    (
                        start.parse::<usize>().unwrap(),
                        end.parse::<usize>().unwrap(),
                    )
                } else {
                    (0, body.len().saturating_sub(1))
                };
                let length = if body.is_empty() { 0 } else { end - start + 1 };
                let etag = if matches!(mode, Mode::Weak) {
                    "W/\"stable\""
                } else {
                    "\"stable\""
                };
                if resumed {
                    let count = active.fetch_add(1, Ordering::SeqCst) + 1;
                    observed.fetch_max(count, Ordering::SeqCst);
                    if concurrent > 1 {
                        barrier.wait().await;
                    }
                }
                let mut headers = format!(
                    "HTTP/1.1 {}\r\nContent-Length: {length}\r\nETag: {etag}\r\nConnection: close\r\n",
                    if partial {
                        "206 Partial Content"
                    } else {
                        "200 OK"
                    }
                );
                if partial {
                    let declared_start = if resumed && matches!(mode, Mode::Malformed) {
                        start + 1
                    } else {
                        start
                    };
                    headers.push_str(&format!(
                        "Content-Range: bytes {declared_start}-{end}/{}\r\n",
                        body.len()
                    ));
                }
                headers.push_str("\r\n");
                if stream.write_all(headers.as_bytes()).await.is_ok() && length > 0 {
                    let mut selected = &body[start..=end];
                    if resumed && matches!(mode, Mode::Truncated) {
                        selected = &selected[..selected.len() / 2];
                    }
                    for piece in selected.chunks(if slow && resumed { 20 } else { 16_384 }) {
                        if stream.write_all(piece).await.is_err() {
                            break;
                        }
                        if slow && resumed {
                            tokio::time::sleep(Duration::from_millis(15)).await;
                        }
                    }
                }
                if resumed {
                    active.fetch_sub(1, Ordering::SeqCst);
                }
            });
        }
    });
    Server {
        source,
        requests,
        maximum,
        task,
    }
}

async fn bounded(
    source: &str,
    destination: std::path::PathBuf,
    options: TransferOptions,
    control: TransferControl,
    callback: impl Fn(TransferTelemetry) + Send + Sync + 'static,
) -> Result<ulos_core::DownloadReceipt, DownloadError> {
    tokio::time::timeout(
        Duration::from_secs(5),
        transfer::run(source, destination, options, control, callback),
    )
    .await
    .expect("transfer timed out")
}

#[tokio::test]
async fn verified_ranges_reassemble_and_are_actually_concurrent() {
    let body: Vec<_> = (0..=255).cycle().take(1027).collect();
    let server = server(Mode::Range, body.clone(), 4, false).await;
    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("output.bin");
    let (sender, receiver) = watch::channel(TransferTelemetry::default());
    let receipt = bounded(
        &server.source,
        destination.clone(),
        TransferOptions {
            connections: 4,
            ..Default::default()
        },
        TransferControl::new(None),
        move |value| {
            sender.send_replace(value);
        },
    )
    .await
    .unwrap();
    assert_eq!(receipt.downloaded_bytes, body.len() as u64);
    assert_eq!(tokio::fs::read(destination).await.unwrap(), body);
    assert_eq!(receiver.borrow().resume, ResumeCapability::Supported);
    assert_eq!(receiver.borrow().connections.len(), 4);
    assert_eq!(server.maximum.load(Ordering::SeqCst), 4);
    let requests = server.requests.lock().unwrap();
    assert_eq!(requests.len(), 5);
    assert!(requests.iter().skip(1).all(|request| {
        request
            .to_ascii_lowercase()
            .contains("if-range: \"stable\"")
    }));
}

#[tokio::test]
async fn sixteen_verified_connections_are_bounded_concurrent_and_reassemble() {
    assert_eq!(MAX_CONNECTIONS, 16);
    let body: Vec<_> = (0..=255).cycle().take(4099).collect();
    let server = server(Mode::Range, body.clone(), MAX_CONNECTIONS, false).await;
    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("sixteen.bin");
    let (sender, receiver) = watch::channel(TransferTelemetry::default());
    let receipt = bounded(
        &server.source,
        destination.clone(),
        TransferOptions {
            connections: MAX_CONNECTIONS,
            ..Default::default()
        },
        TransferControl::new(None),
        move |value| {
            sender.send_replace(value);
        },
    )
    .await
    .unwrap();
    assert_eq!(receipt.downloaded_bytes, body.len() as u64);
    assert_eq!(tokio::fs::read(destination).await.unwrap(), body);
    assert_eq!(receiver.borrow().resume, ResumeCapability::Supported);
    assert_eq!(receiver.borrow().connections.len(), MAX_CONNECTIONS);
    assert_eq!(server.maximum.load(Ordering::SeqCst), MAX_CONNECTIONS);
    assert_eq!(server.requests.lock().unwrap().len(), MAX_CONNECTIONS + 1);
}

#[tokio::test]
async fn full_response_is_reused_without_fake_segments() {
    let server = server(Mode::Full, b"full stream".to_vec(), 1, false).await;
    let directory = tempfile::tempdir().unwrap();
    let (sender, receiver) = watch::channel(TransferTelemetry::default());
    bounded(
        &server.source,
        directory.path().join("full"),
        TransferOptions {
            connections: 8,
            ..Default::default()
        },
        TransferControl::new(None),
        move |value| {
            sender.send_replace(value);
        },
    )
    .await
    .unwrap();
    assert_eq!(server.requests.lock().unwrap().len(), 1);
    assert_eq!(receiver.borrow().resume, ResumeCapability::Unsupported);
    assert_eq!(receiver.borrow().connections.len(), 1);
}

#[tokio::test]
async fn weak_validator_uses_a_complete_response() {
    let server = server(Mode::Weak, b"weak".to_vec(), 1, false).await;
    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("weak");
    bounded(
        &server.source,
        destination.clone(),
        TransferOptions::default(),
        TransferControl::new(None),
        |_| {},
    )
    .await
    .unwrap();
    assert_eq!(tokio::fs::read(destination).await.unwrap(), b"weak");
    let requests = server.requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert!(!requests[1].to_ascii_lowercase().contains("range:"));
}

#[tokio::test]
async fn invalid_ranges_and_changed_or_truncated_responses_preserve_old_file() {
    for mode in [Mode::Malformed, Mode::Changed, Mode::Truncated] {
        let server = server(mode, b"replacement".to_vec(), 1, false).await;
        let directory = tempfile::tempdir().unwrap();
        let destination = directory.path().join("old");
        tokio::fs::write(&destination, b"old bytes").await.unwrap();
        let error = bounded(
            &server.source,
            destination.clone(),
            TransferOptions {
                collision: CollisionPolicy::Overwrite,
                ..Default::default()
            },
            TransferControl::new(None),
            |_| {},
        )
        .await
        .unwrap_err();
        assert!(!error.to_string().contains("SECRET"));
        assert_eq!(tokio::fs::read(destination).await.unwrap(), b"old bytes");
    }
}

#[tokio::test]
async fn pause_closes_stream_and_resume_uses_written_offset_and_validator() {
    let body = vec![7; 400];
    let server = server(Mode::Range, body.clone(), 1, true).await;
    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("paused");
    let control = TransferControl::new(None);
    let (sender, mut receiver) = watch::channel(TransferTelemetry::default());
    let source = server.source.clone();
    let path = destination.clone();
    let task_control = control.clone();
    let task = tokio::spawn(async move {
        bounded(
            &source,
            path,
            TransferOptions::default(),
            task_control,
            move |value| {
                sender.send_replace(value);
            },
        )
        .await
    });
    tokio::time::timeout(Duration::from_secs(2), async {
        while receiver.borrow().progress.downloaded_bytes == 0 {
            receiver.changed().await.unwrap();
        }
        control.pause();
        while !receiver.borrow().paused {
            receiver.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
    let paused = receiver.borrow().progress.downloaded_bytes;
    tokio::time::sleep(Duration::from_millis(80)).await;
    assert_eq!(receiver.borrow().progress.downloaded_bytes, paused);
    assert!(!destination.exists());
    control.resume();
    task.await.unwrap().unwrap();
    assert_eq!(tokio::fs::read(destination).await.unwrap(), body);
    let requests = server.requests.lock().unwrap();
    assert!(requests.iter().any(|request| {
        request
            .to_ascii_lowercase()
            .contains(&format!("range: bytes={paused}-399"))
    }));
}

#[tokio::test]
async fn shared_limit_charges_all_connections_without_initial_burst() {
    let server = server(Mode::Range, vec![8; 200], 4, false).await;
    let directory = tempfile::tempdir().unwrap();
    let start = Instant::now();
    bounded(
        &server.source,
        directory.path().join("limited"),
        TransferOptions {
            connections: 4,
            bytes_per_second: Some(1000),
            ..Default::default()
        },
        TransferControl::new(Some(1000)),
        |_| {},
    )
    .await
    .unwrap();
    assert!(start.elapsed() >= Duration::from_millis(190));
}

#[tokio::test]
async fn live_limit_change_interrupts_a_slow_grant() {
    let server = server(Mode::Range, vec![8; 20], 1, false).await;
    let directory = tempfile::tempdir().unwrap();
    let control = TransferControl::new(Some(1));
    let (sender, mut receiver) = watch::channel(TransferTelemetry::default());
    let source = server.source.clone();
    let destination = directory.path().join("dynamic");
    let task_control = control.clone();
    let task = tokio::spawn(async move {
        bounded(
            &source,
            destination,
            TransferOptions {
                bytes_per_second: Some(1),
                ..Default::default()
            },
            task_control,
            move |value| {
                sender.send_replace(value);
            },
        )
        .await
    });
    tokio::time::timeout(Duration::from_secs(2), async {
        while receiver
            .borrow()
            .connections
            .first()
            .is_none_or(|row| row.status != transfer::ConnectionStatus::Downloading)
        {
            receiver.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
    control.set_rate_limit(None);
    tokio::time::timeout(Duration::from_secs(1), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn collision_policies_are_explicit_and_rename_publishes_unique_filename() {
    let server = server(Mode::Full, b"new".to_vec(), 1, false).await;
    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("file.bin");
    tokio::fs::write(&destination, b"old").await.unwrap();
    assert!(matches!(
        bounded(
            &server.source,
            destination.clone(),
            TransferOptions::default(),
            TransferControl::new(None),
            |_| {}
        )
        .await,
        Err(DownloadError::DestinationExists)
    ));
    assert!(server.requests.lock().unwrap().is_empty());
    let receipt = bounded(
        &server.source,
        destination.clone(),
        TransferOptions {
            collision: CollisionPolicy::Rename,
            ..Default::default()
        },
        TransferControl::new(None),
        |_| {},
    )
    .await
    .unwrap();
    assert_eq!(receipt.destination, directory.path().join("file (1).bin"));
    assert_eq!(tokio::fs::read(&destination).await.unwrap(), b"old");
    bounded(
        &server.source,
        destination.clone(),
        TransferOptions {
            collision: CollisionPolicy::Overwrite,
            ..Default::default()
        },
        TransferControl::new(None),
        |_| {},
    )
    .await
    .unwrap();
    assert_eq!(tokio::fs::read(destination).await.unwrap(), b"new");
}

#[tokio::test]
async fn dropping_active_transfer_does_not_publish() {
    let server = server(Mode::Range, vec![9; 400], 1, true).await;
    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("cancelled");
    let (sender, mut receiver) = watch::channel(TransferTelemetry::default());
    let source = server.source.clone();
    let path = destination.clone();
    let task = tokio::spawn(async move {
        transfer::run(
            &source,
            path,
            TransferOptions::default(),
            TransferControl::new(None),
            move |value| {
                sender.send_replace(value);
            },
        )
        .await
    });
    tokio::time::timeout(Duration::from_secs(2), async {
        while receiver.borrow().progress.downloaded_bytes == 0 {
            receiver.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
    task.abort();
    let _ = task.await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(!destination.exists());
}

#[cfg(unix)]
#[tokio::test]
async fn overwrite_refuses_symlinks() {
    let directory = tempfile::tempdir().unwrap();
    let original = directory.path().join("original");
    let link = directory.path().join("link");
    tokio::fs::write(&original, b"unchanged").await.unwrap();
    std::os::unix::fs::symlink(&original, &link).unwrap();
    assert!(matches!(
        bounded(
            "http://127.0.0.1:1/file",
            link,
            TransferOptions {
                collision: CollisionPolicy::Overwrite,
                ..Default::default()
            },
            TransferControl::new(None),
            |_| {}
        )
        .await,
        Err(DownloadError::InvalidDestination)
    ));
    assert_eq!(tokio::fs::read(original).await.unwrap(), b"unchanged");
}

#[tokio::test]
async fn invalid_options_and_credentials_fail_before_network() {
    let directory = tempfile::tempdir().unwrap();
    for connections in [0, MAX_CONNECTIONS + 1, usize::MAX] {
        assert!(
            bounded(
                "http://127.0.0.1:1/file",
                directory.path().join("invalid"),
                TransferOptions {
                    connections,
                    ..Default::default()
                },
                TransferControl::new(None),
                |_| {}
            )
            .await
            .is_err()
        );
    }
    assert!(matches!(
        bounded(
            "http://user:SECRET@127.0.0.1:1/file",
            directory.path().join("invalid"),
            TransferOptions::default(),
            TransferControl::new(None),
            |_| {}
        )
        .await,
        Err(DownloadError::EmbeddedCredentials)
    ));
}
