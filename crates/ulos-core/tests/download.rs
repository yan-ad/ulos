use std::{path::Path, time::Duration};

use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::TcpListener,
};
use ulos_core::{DownloadError, DownloadRequest, Downloader};

async fn serve(response: &'static [u8]) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        read_headers(&mut stream).await;
        stream.write_all(response).await.unwrap();
        stream.shutdown().await.unwrap();
    });
    format!("http://{address}/file")
}

async fn read_headers(stream: &mut tokio::net::TcpStream) -> String {
    tokio::time::timeout(Duration::from_secs(3), async {
        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        let mut length = 0;
        let mut headers = String::new();
        loop {
            line.clear();
            let count = reader.read_line(&mut line).await.unwrap();
            assert_ne!(count, 0, "Request ended before its headers");
            length += count;
            headers.push_str(&line);
            assert!(length < 16_384, "Request headers exceeded fixture limit");
            if line == "\r\n" {
                break;
            }
        }
        headers
    })
    .await
    .expect("Timed out reading fixture request")
}

#[tokio::test]
async fn rejects_repeated_encoded_headers() {
    let url = serve(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nContent-Encoding: identity\r\nContent-Encoding: gzip\r\nConnection: close\r\n\r\nhello").await;
    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("file.bin");
    let request = DownloadRequest::new(&url, &destination).unwrap();
    assert!(matches!(
        Downloader::new().unwrap().download(&request, |_| {}).await,
        Err(DownloadError::UnexpectedEncoding)
    ));
    assert!(!destination.exists());
    assert_no_parts(directory.path());
}

#[tokio::test]
async fn cross_origin_redirect_does_not_send_a_referer() {
    let target = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let target_url = format!("http://{}/file", target.local_addr().unwrap());
    let target_server = tokio::spawn(async move {
        let (mut stream, _) = target.accept().await.unwrap();
        let headers = read_headers(&mut stream).await;
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\nhello")
            .await
            .unwrap();
        headers
    });
    let source = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!(
        "http://{}/file?token=secret-value",
        source.local_addr().unwrap()
    );
    let source_server = tokio::spawn(async move {
        let (mut stream, _) = source.accept().await.unwrap();
        read_headers(&mut stream).await;
        let response = format!(
            "HTTP/1.1 302 Found\r\nLocation: {target_url}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        );
        stream.write_all(response.as_bytes()).await.unwrap();
    });
    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("file.bin");
    let request = DownloadRequest::new(&url, &destination).unwrap();
    tokio::time::timeout(
        Duration::from_secs(5),
        Downloader::new().unwrap().download(&request, |_| {}),
    )
    .await
    .unwrap()
    .unwrap();
    let headers = tokio::time::timeout(Duration::from_secs(5), target_server)
        .await
        .unwrap()
        .unwrap();
    assert!(!headers.to_ascii_lowercase().contains("referer:"));
    assert!(!headers.contains("secret-value"));
    source_server.await.unwrap();
    assert_eq!(std::fs::read(destination).unwrap(), b"hello");
}

#[tokio::test]
async fn rejects_unsolicited_partial_responses() {
    let url = serve(b"HTTP/1.1 206 Partial Content\r\nContent-Length: 5\r\nContent-Range: bytes 0-4/10\r\nConnection: close\r\n\r\nhello").await;
    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("file.bin");
    let request = DownloadRequest::new(&url, &destination).unwrap();
    let result = Downloader::new().unwrap().download(&request, |_| {}).await;
    assert!(
        matches!(result, Err(DownloadError::UnexpectedStatus(status)) if status.as_u16() == 206)
    );
    assert!(!destination.exists());
    assert_no_parts(directory.path());
}

#[tokio::test]
async fn rejects_encoded_responses() {
    let url = serve(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nContent-Encoding: gzip\r\nConnection: close\r\n\r\nhello").await;
    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("file.bin");
    let request = DownloadRequest::new(&url, &destination).unwrap();
    let result = Downloader::new().unwrap().download(&request, |_| {}).await;
    assert!(matches!(result, Err(DownloadError::UnexpectedEncoding)));
    assert!(!destination.exists());
    assert_no_parts(directory.path());
}

#[tokio::test]
async fn rejects_redirects_to_non_http_schemes() {
    let url = serve(b"HTTP/1.1 302 Found\r\nLocation: file:///etc/passwd\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await;
    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("file.bin");
    let request = DownloadRequest::new(&url, &destination).unwrap();
    assert!(
        Downloader::new()
            .unwrap()
            .download(&request, |_| {})
            .await
            .is_err()
    );
    assert!(!destination.exists());
    assert_no_parts(directory.path());
}

#[tokio::test]
async fn accepts_empty_files() {
    let url = serve(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await;
    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("empty.bin");
    let request = DownloadRequest::new(&url, &destination).unwrap();
    let receipt = Downloader::new()
        .unwrap()
        .download(&request, |_| {})
        .await
        .unwrap();
    assert_eq!(receipt.downloaded_bytes, 0);
    assert!(std::fs::read(destination).unwrap().is_empty());
    assert_no_parts(directory.path());
}

#[tokio::test]
async fn reports_missing_parent_without_creating_it() {
    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("missing/file.bin");
    let request = DownloadRequest::new("http://127.0.0.1:1/file", &destination).unwrap();
    assert!(matches!(
        Downloader::new().unwrap().download(&request, |_| {}).await,
        Err(DownloadError::Io(_))
    ));
    assert!(!directory.path().join("missing").exists());
}

fn assert_no_parts(directory: &Path) {
    assert!(std::fs::read_dir(directory).unwrap().all(|entry| {
        !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .ends_with(".part")
    }));
}

#[tokio::test]
async fn streams_and_reports_progress() {
    let url =
        serve(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\nhello").await;
    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("file.bin");
    let request = DownloadRequest::new(&url, &destination).unwrap();
    let mut progress = Vec::new();
    let receipt = Downloader::new()
        .unwrap()
        .download(&request, |event| progress.push(event))
        .await
        .unwrap();
    assert_eq!(std::fs::read(&destination).unwrap(), b"hello");
    assert_eq!(receipt.downloaded_bytes, 5);
    assert_eq!(progress.first().unwrap().downloaded_bytes, 0);
    assert_eq!(progress.last().unwrap().downloaded_bytes, 5);
    assert_eq!(progress.last().unwrap().total_bytes, Some(5));
    assert_no_parts(directory.path());
}

#[tokio::test]
async fn accepts_unknown_length() {
    let url = serve(b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\nhello").await;
    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("file.bin");
    let request = DownloadRequest::new(&url, &destination).unwrap();
    Downloader::new()
        .unwrap()
        .download(&request, |event| assert_eq!(event.total_bytes, None))
        .await
        .unwrap();
    assert_eq!(std::fs::read(destination).unwrap(), b"hello");
}

#[tokio::test]
async fn refuses_existing_destinations() {
    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("file.bin");
    std::fs::write(&destination, b"original").unwrap();
    let request = DownloadRequest::new("http://127.0.0.1:1/file", &destination).unwrap();
    let result = Downloader::new().unwrap().download(&request, |_| {}).await;
    assert!(matches!(result, Err(DownloadError::DestinationExists)));
    assert_eq!(std::fs::read(destination).unwrap(), b"original");
    assert_no_parts(directory.path());
}

#[tokio::test]
async fn refuses_destination_created_mid_transfer() {
    let url =
        serve(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\nhello").await;
    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("file.bin");
    let request = DownloadRequest::new(&url, &destination).unwrap();
    let result = Downloader::new()
        .unwrap()
        .download(&request, |event| {
            if event.downloaded_bytes == 0 {
                std::fs::write(&destination, b"original").unwrap();
            }
        })
        .await;
    assert!(matches!(result, Err(DownloadError::DestinationExists)));
    assert_eq!(std::fs::read(destination).unwrap(), b"original");
    assert_no_parts(directory.path());
}

#[tokio::test]
async fn rejects_http_errors_and_cleans_up() {
    let url =
        serve(b"HTTP/1.1 404 Not Found\r\nContent-Length: 5\r\nConnection: close\r\n\r\nerror")
            .await;
    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("file.bin");
    let request = DownloadRequest::new(&url, &destination).unwrap();
    let result = Downloader::new().unwrap().download(&request, |_| {}).await;
    assert!(
        matches!(result, Err(DownloadError::UnexpectedStatus(status)) if status.as_u16() == 404)
    );
    assert!(!destination.exists());
    assert_no_parts(directory.path());
}

#[tokio::test]
async fn rejects_truncated_responses_and_cleans_up() {
    let url =
        serve(b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\nConnection: close\r\n\r\nshort").await;
    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("file.bin");
    let request = DownloadRequest::new(&url, &destination).unwrap();
    assert!(
        Downloader::new()
            .unwrap()
            .download(&request, |_| {})
            .await
            .is_err()
    );
    assert!(!destination.exists());
    assert_no_parts(directory.path());
}

#[tokio::test]
async fn cancellation_cleans_up_without_publishing() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/file", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        read_headers(&mut stream).await;
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\n")
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_secs(10)).await;
    });
    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("file.bin");
    let request = DownloadRequest::new(&url, &destination).unwrap();
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let mut started_tx = Some(started_tx);
    let downloader = Downloader::new().unwrap();
    {
        let transfer = downloader.download(&request, |_| {
            if let Some(tx) = started_tx.take() {
                let _ = tx.send(());
            }
        });
        tokio::pin!(transfer);
        tokio::select! {
            result = tokio::time::timeout(Duration::from_secs(3), started_rx) => {
                result.expect("Server did not start the transfer").expect("Progress sender dropped");
            },
            result = &mut transfer => panic!("Transfer ended before cancellation: {result:?}"),
        }
    }
    server.abort();
    assert!(!destination.exists());
    assert_no_parts(directory.path());
}
