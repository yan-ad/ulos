use std::{
    path::Path,
    sync::{Arc, Mutex},
    time::Duration,
};

use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    net::{TcpListener, TcpStream},
    runtime::Handle,
    sync::{oneshot, watch},
    task::{JoinHandle, JoinSet},
    time::timeout,
};
use ulos_core::{
    DownloadReceipt,
    tasks::{HttpDownloadTask, TransferState},
    transfer::{ConnectionStatus, ResumeCapability, TransferOptions, TransferTelemetry},
};

const DEADLINE: Duration = Duration::from_secs(5);
const BODY: &[u8] = b"abcdefgh";

// Probe-aware, bounded local fixture. Range bodies stay gated until the test
// releases them; closing a paused/cancelled response is acknowledged by EOF.
struct Fixture {
    url: String,
    release: watch::Sender<bool>,
    requests: Arc<Mutex<Vec<String>>>,
    stop: oneshot::Sender<()>,
    server: JoinHandle<()>,
}

impl Fixture {
    async fn start(ranges: bool) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!(
            "http://{}/file?token=private",
            listener.local_addr().unwrap()
        );
        let (release, gate) = watch::channel(false);
        let (stop, mut stopped) = oneshot::channel();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let observed = requests.clone();
        let server = tokio::spawn(async move {
            timeout(DEADLINE, async move {
                let mut clients = JoinSet::new();
                loop {
                    tokio::select! {
                        _ = &mut stopped => break,
                        client = listener.accept() => {
                            let (stream, _) = client.unwrap();
                            let requests = observed.clone();
                            let gate = gate.clone();
                            clients.spawn(async move { respond(stream, ranges, gate, requests).await });
                        }
                        result = clients.join_next(), if !clients.is_empty() => {
                            result.unwrap().unwrap();
                        }
                    }
                }
                clients.abort_all();
                while let Some(result) = clients.join_next().await {
                    if let Err(error) = result { assert!(error.is_cancelled()); }
                }
            }).await.expect("Fixture deadline elapsed");
        });
        Self {
            url,
            release,
            requests,
            stop,
            server,
        }
    }

    async fn finish(self) {
        self.stop.send(()).unwrap();
        timeout(DEADLINE, self.server).await.unwrap().unwrap();
    }
}

async fn respond(
    stream: TcpStream,
    ranges: bool,
    mut gate: watch::Receiver<bool>,
    requests: Arc<Mutex<Vec<String>>>,
) {
    let mut reader = BufReader::new(stream);
    let mut headers = String::new();
    loop {
        let mut line = String::new();
        assert_ne!(reader.read_line(&mut line).await.unwrap(), 0);
        headers.push_str(&line);
        assert!(headers.len() < 16_384);
        if line == "\r\n" {
            break;
        }
    }
    let lower = headers.to_ascii_lowercase();
    requests.lock().unwrap().push(lower.clone());
    let range = lower
        .lines()
        .find_map(|line| line.strip_prefix("range: bytes="));
    let (start, end) = if ranges {
        let (start, end) = range
            .expect("Expected a range request")
            .split_once('-')
            .unwrap();
        (
            start.parse::<usize>().unwrap(),
            end.parse::<usize>().unwrap(),
        )
    } else {
        (0, BODY.len() - 1)
    };
    let probe = ranges && start == 0 && end == 0 && !lower.contains("if-range:");
    if ranges && !probe {
        assert!(lower.contains("if-range: \"stable\""));
    }
    let response = if ranges {
        format!(
            "HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nContent-Range: bytes {start}-{end}/{}\r\nETag: \"stable\"\r\nConnection: close\r\n\r\n",
            end - start + 1,
            BODY.len()
        )
    } else {
        format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            BODY.len()
        )
    };
    let mut stream = reader.into_inner();
    stream.write_all(response.as_bytes()).await.unwrap();
    if !probe {
        let mut byte = [0];
        tokio::select! {
            _ = async { while !*gate.borrow_and_update() { gate.changed().await.unwrap(); } } => {}
            result = stream.read(&mut byte) => {
                assert_eq!(result.unwrap(), 0, "Expected response to close on pause/cancel");
                return;
            }
        }
    }
    // A concurrent pause can close a just-released stream.
    let _ = stream.write_all(&BODY[start..=end]).await;
    let _ = stream.shutdown().await;
}

fn options() -> TransferOptions {
    TransferOptions {
        connections: 2,
        ..TransferOptions::default()
    }
}

async fn telemetry_matching(
    receiver: &mut watch::Receiver<TransferTelemetry>,
    matches: impl Fn(&TransferTelemetry) -> bool,
) -> TransferTelemetry {
    timeout(DEADLINE, async {
        loop {
            let value = receiver.borrow_and_update().clone();
            if matches(&value) {
                return value;
            }
            receiver
                .changed()
                .await
                .expect("Telemetry closed before expected update");
        }
    })
    .await
    .expect("Telemetry deadline elapsed")
}

async fn state_matching(
    receiver: &mut watch::Receiver<TransferState>,
    matches: impl Fn(&TransferState) -> bool,
) -> TransferState {
    timeout(DEADLINE, async {
        loop {
            let value = receiver.borrow_and_update().clone();
            if matches(&value) {
                return value;
            }
            assert!(!value.is_terminal(), "Unexpected terminal state: {value:?}");
            receiver
                .changed()
                .await
                .expect("State closed before expected update");
        }
    })
    .await
    .expect("State deadline elapsed")
}

async fn terminal(receiver: &mut watch::Receiver<TransferState>) -> TransferState {
    let result = state_matching(receiver, TransferState::is_terminal).await;
    assert!(!result.is_active());
    assert!(
        timeout(DEADLINE, receiver.changed())
            .await
            .unwrap()
            .is_err()
    );
    assert_eq!(*receiver.borrow(), result);
    result
}

fn assert_clean(path: &Path) {
    assert!(std::fs::read_dir(path).unwrap().all(|entry| {
        !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".ulos-")
    }));
}

async fn ready(task: &HttpDownloadTask) -> watch::Receiver<TransferTelemetry> {
    let mut telemetry = task.telemetry().unwrap();
    telemetry_matching(&mut telemetry, |value| {
        value.resume == ResumeCapability::Supported
            && value.connections.len() == 2
            && value
                .connections
                .iter()
                .all(|row| row.status == ConnectionStatus::Downloading)
    })
    .await;
    telemetry
}

#[tokio::test]
async fn controlled_success_reports_exact_receipt_and_multiple_connections() {
    let fixture = Fixture::start(true).await;
    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("file");
    let task = HttpDownloadTask::start_with_options(
        &Handle::current(),
        fixture.url.clone(),
        destination.clone(),
        options(),
    );
    let mut state = task.subscribe();
    assert_eq!(*state.borrow(), TransferState::Preparing);
    assert_eq!(task.telemetry().unwrap().borrow().destination, destination);
    assert!(!task.pause());
    let telemetry = ready(&task).await;
    fixture.release.send_replace(true);
    assert_eq!(
        terminal(&mut state).await,
        TransferState::Completed(DownloadReceipt {
            destination: destination.clone(),
            downloaded_bytes: 8
        })
    );
    let value = telemetry.borrow().clone();
    assert_eq!(value.progress.downloaded_bytes, 8);
    assert_eq!(value.destination, destination);
    assert!(
        value
            .connections
            .iter()
            .all(|row| row.status == ConnectionStatus::Complete)
    );
    assert_eq!(std::fs::read(destination).unwrap(), BODY);
    assert_eq!(fixture.requests.lock().unwrap().len(), 3);
    assert!(!task.pause());
    assert!(!task.resume());
    assert!(!task.set_rate_limit(Some(5)));
    assert_clean(directory.path());
    fixture.finish().await;
}

#[tokio::test]
async fn pause_is_nonterminal_and_resume_reopens_validated_ranges() {
    let fixture = Fixture::start(true).await;
    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("file");
    let task = HttpDownloadTask::start_with_options(
        &Handle::current(),
        fixture.url.clone(),
        destination.clone(),
        options(),
    );
    let mut state = task.subscribe();
    let mut telemetry = ready(&task).await;
    assert!(!task.resume());
    assert!(task.pause());
    assert!(!task.pause());
    telemetry_matching(&mut telemetry, |value| {
        value.paused
            && value
                .connections
                .iter()
                .all(|row| row.status == ConnectionStatus::Paused)
    })
    .await;
    let paused = state_matching(&mut state, |value| {
        matches!(value, TransferState::Paused(_))
    })
    .await;
    assert!(paused.is_active());
    assert!(!paused.is_terminal());
    assert!(!destination.exists());
    assert!(task.resume());
    assert!(!task.resume());
    telemetry_matching(&mut telemetry, |value| {
        !value.paused
            && value
                .connections
                .iter()
                .all(|row| row.status == ConnectionStatus::Downloading)
    })
    .await;
    fixture.release.send_replace(true);
    assert!(matches!(
        terminal(&mut state).await,
        TransferState::Completed(_)
    ));
    assert_eq!(std::fs::read(destination).unwrap(), BODY);
    assert_eq!(fixture.requests.lock().unwrap().len(), 5);
    assert_clean(directory.path());
    fixture.finish().await;
}

#[tokio::test]
async fn unsupported_and_legacy_tasks_refuse_pause() {
    for advanced in [true, false] {
        let fixture = Fixture::start(false).await;
        let directory = tempfile::tempdir().unwrap();
        let task = if advanced {
            HttpDownloadTask::start_with_options(
                &Handle::current(),
                fixture.url.clone(),
                directory.path().join("file"),
                options(),
            )
        } else {
            HttpDownloadTask::start(
                &Handle::current(),
                fixture.url.clone(),
                directory.path().join("file"),
            )
        };
        let mut state = task.subscribe();
        if advanced {
            telemetry_matching(&mut task.telemetry().unwrap(), |value| {
                value.resume == ResumeCapability::Unsupported
            })
            .await;
        } else {
            assert!(task.telemetry().is_none());
            assert!(!task.set_rate_limit(Some(1)));
        }
        state_matching(&mut state, |value| {
            matches!(value, TransferState::Downloading(_))
        })
        .await;
        assert!(!task.pause());
        assert!(!task.resume());
        fixture.release.send_replace(true);
        assert!(matches!(
            terminal(&mut state).await,
            TransferState::Completed(_)
        ));
        assert_eq!(fixture.requests.lock().unwrap().len(), 1);
        if !advanced {
            assert!(!fixture.requests.lock().unwrap()[0].contains("range:"));
        }
        fixture.finish().await;
    }
}

#[tokio::test]
async fn live_rate_updates_reject_zero_without_mutating_the_budget() {
    let fixture = Fixture::start(true).await;
    let directory = tempfile::tempdir().unwrap();
    let task = HttpDownloadTask::start_with_options(
        &Handle::current(),
        fixture.url.clone(),
        directory.path().join("file"),
        options(),
    );
    let mut state = task.subscribe();
    let mut telemetry = ready(&task).await;
    assert!(task.set_rate_limit(Some(100)));
    assert!(!task.set_rate_limit(Some(0)));
    fixture.release.send_replace(true);
    let value =
        telemetry_matching(&mut telemetry, |value| value.progress.downloaded_bytes > 0).await;
    assert_eq!(value.bytes_per_second, Some(100));
    assert!(task.set_rate_limit(None));
    assert!(matches!(
        terminal(&mut state).await,
        TransferState::Completed(_)
    ));
    assert_eq!(telemetry.borrow().bytes_per_second, None);
    assert_clean(directory.path());
    fixture.finish().await;
}

#[tokio::test]
async fn dropping_a_controlled_task_cancels_and_cleans_owned_staging() {
    let fixture = Fixture::start(true).await;
    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("file");
    let unrelated = directory.path().join("unrelated.part");
    std::fs::write(&unrelated, b"keep").unwrap();
    let task = HttpDownloadTask::start_with_options(
        &Handle::current(),
        fixture.url.clone(),
        destination.clone(),
        options(),
    );
    let mut state = task.subscribe();
    let telemetry = ready(&task).await;
    drop(task);
    assert_eq!(terminal(&mut state).await, TransferState::Cancelled);
    assert!(!destination.exists());
    assert_eq!(std::fs::read(unrelated).unwrap(), b"keep");
    assert_clean(directory.path());
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
    assert_eq!(telemetry.borrow().progress.downloaded_bytes, 0);
    fixture.finish().await;
}
