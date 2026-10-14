use std::{path::Path, time::Duration};

use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    net::{TcpListener, TcpStream},
    runtime::Handle,
    sync::{oneshot, watch},
    task::JoinHandle,
    time::timeout,
};
use ulos_core::{
    DownloadProgress, DownloadReceipt,
    tasks::{HttpDownloadTask, TransferState},
};

const DEADLINE: Duration = Duration::from_secs(5);

async fn read_headers(stream: &mut TcpStream) {
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    let mut bytes = 0;
    loop {
        line.clear();
        let count = reader.read_line(&mut line).await.unwrap();
        assert_ne!(count, 0, "Request ended before its headers");
        bytes += count;
        assert!(bytes < 16_384, "Request exceeds fixture limit");
        if line == "\r\n" {
            return;
        }
    }
}

async fn serve(response: &'static [u8]) -> (String, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!(
        "http://{}/file?token=private",
        listener.local_addr().unwrap()
    );
    let server = tokio::spawn(async move {
        timeout(DEADLINE, async {
            let (mut stream, _) = listener.accept().await.unwrap();
            read_headers(&mut stream).await;
            stream.write_all(response).await.unwrap();
            stream.shutdown().await.unwrap();
        })
        .await
        .expect("Fixture deadline elapsed");
    });
    (url, server)
}

// Hold the response body until the test acknowledges Downloading. Cancellation
// tests instead wait for the client to close its socket, with no timing sleeps.
async fn serve_gated() -> (String, oneshot::Sender<()>, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/file", listener.local_addr().unwrap());
    let (release, released) = oneshot::channel();
    let server = tokio::spawn(async move {
        timeout(DEADLINE, async {
            let (mut stream, _) = listener.accept().await.unwrap();
            read_headers(&mut stream).await;
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\n")
                .await
                .unwrap();
            let mut byte = [0];
            tokio::select! {
                result = released => {
                    result.expect("Fixture release dropped");
                    stream.write_all(b"hello").await.unwrap();
                    stream.shutdown().await.unwrap();
                }
                result = stream.read(&mut byte) => {
                    assert_eq!(result.unwrap(), 0, "Expected cancelled connection to close");
                }
            }
        })
        .await
        .expect("Gated fixture deadline elapsed");
    });
    (url, release, server)
}

async fn next_matching(
    state: &mut watch::Receiver<TransferState>,
    matches: impl Fn(&TransferState) -> bool,
) -> TransferState {
    timeout(DEADLINE, async {
        loop {
            let latest = state.borrow_and_update().clone();
            if matches(&latest) {
                return latest;
            }
            assert!(
                !latest.is_terminal(),
                "Unexpected terminal state: {latest:?}"
            );
            state
                .changed()
                .await
                .expect("Worker ended without expected state");
        }
    })
    .await
    .expect("Transfer state deadline elapsed")
}

async fn terminal(state: &mut watch::Receiver<TransferState>) -> TransferState {
    let terminal = next_matching(state, TransferState::is_terminal).await;
    assert!(!terminal.is_active());
    assert!(
        timeout(DEADLINE, state.changed()).await.unwrap().is_err(),
        "Worker published after terminal state"
    );
    assert_eq!(*state.borrow(), terminal);
    terminal
}

fn assert_clean(directory: &Path) {
    assert!(std::fs::read_dir(directory).unwrap().all(|entry| {
        !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .ends_with(".part")
    }));
}

#[tokio::test]
async fn invalid_url_fails_without_exposing_source_or_creating_files() {
    let directory = tempfile::tempdir().unwrap();
    let task = HttpDownloadTask::start(
        &Handle::current(),
        "not a URL?token=private".into(),
        directory.path().join("file"),
    );
    let mut state = task.subscribe();
    assert_eq!(*state.borrow(), TransferState::Preparing);
    assert!(state.borrow().is_active());
    let result = terminal(&mut state).await;
    assert!(
        matches!(&result, TransferState::Failed(message) if message == "Enter a valid HTTP or HTTPS URL")
    );
    assert!(!format!("{result:?}").contains("private"));
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
}

#[tokio::test]
async fn success_reports_progress_and_exact_receipt_and_bytes() {
    let (url, release, server) = serve_gated().await;
    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("file");
    let mut task = HttpDownloadTask::start(&Handle::current(), url, destination.clone());
    let mut state = task.subscribe();
    assert_eq!(*state.borrow(), TransferState::Preparing);
    let progress = next_matching(&mut state, |state| {
        matches!(state, TransferState::Downloading(_))
    })
    .await;
    assert_eq!(
        progress,
        TransferState::Downloading(DownloadProgress {
            downloaded_bytes: 0,
            total_bytes: Some(5)
        })
    );
    release.send(()).unwrap();
    assert_eq!(
        terminal(&mut state).await,
        TransferState::Completed(DownloadReceipt {
            destination: destination.clone(),
            downloaded_bytes: 5
        })
    );
    timeout(DEADLINE, server).await.unwrap().unwrap();
    assert_eq!(std::fs::read(destination).unwrap(), b"hello");
    task.cancel();
    task.cancel();
    assert!(matches!(*state.borrow(), TransferState::Completed(_)));
    assert_clean(directory.path());
}

#[tokio::test]
async fn http_error_fails_and_cleans_owned_staging() {
    let (url, server) =
        serve(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await;
    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("file");
    let task = HttpDownloadTask::start(&Handle::current(), url, destination.clone());
    let mut state = task.subscribe();
    assert!(
        matches!(terminal(&mut state).await, TransferState::Failed(message) if message.contains("404") && !message.contains("private"))
    );
    timeout(DEADLINE, server).await.unwrap().unwrap();
    assert!(!destination.exists());
    assert_clean(directory.path());
}

async fn cancel_slow_transfer(drop_handle: bool) {
    let (url, _release, server) = serve_gated().await;
    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("file");
    let unrelated = directory.path().join("unrelated.part");
    std::fs::write(&unrelated, b"keep").unwrap();
    let mut task = HttpDownloadTask::start(&Handle::current(), url, destination.clone());
    let mut state = task.subscribe();
    next_matching(&mut state, |state| {
        matches!(state, TransferState::Downloading(_))
    })
    .await;
    if drop_handle {
        drop(task);
    } else {
        task.cancel();
        task.cancel();
    }
    assert_eq!(terminal(&mut state).await, TransferState::Cancelled);
    timeout(DEADLINE, server).await.unwrap().unwrap();
    // This fixture never releases its body, so publication cannot start. This
    // assertion is fixture-specific, not a general cancellation guarantee.
    assert!(!destination.exists());
    assert_eq!(std::fs::read(&unrelated).unwrap(), b"keep");
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
}

#[tokio::test]
async fn cancellation_is_idempotent_and_cleans_owned_staging() {
    cancel_slow_transfer(false).await;
}

#[tokio::test]
async fn dropping_handle_cancels_even_with_a_live_subscriber() {
    cancel_slow_transfer(true).await;
}

#[tokio::test]
async fn concurrent_tasks_publish_independent_outputs() {
    let (first_url, first_server) =
        serve(b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\nConnection: close\r\n\r\none").await;
    let (second_url, second_server) =
        serve(b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\nConnection: close\r\n\r\ntwo").await;
    let directory = tempfile::tempdir().unwrap();
    let first = directory.path().join("one");
    let second = directory.path().join("two");
    let first_task = HttpDownloadTask::start(&Handle::current(), first_url, first.clone());
    let second_task = HttpDownloadTask::start(&Handle::current(), second_url, second.clone());
    let mut first_state = first_task.subscribe();
    let mut second_state = second_task.subscribe();
    let (first_result, second_result) =
        tokio::join!(terminal(&mut first_state), terminal(&mut second_state));
    assert!(matches!(first_result, TransferState::Completed(_)));
    assert!(matches!(second_result, TransferState::Completed(_)));
    timeout(DEADLINE, first_server).await.unwrap().unwrap();
    timeout(DEADLINE, second_server).await.unwrap().unwrap();
    assert_eq!(std::fs::read(first).unwrap(), b"one");
    assert_eq!(std::fs::read(second).unwrap(), b"two");
    assert_clean(directory.path());
}
