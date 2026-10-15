//! Local-only explicit HEAD preview contracts. No downloads or OS dispatch.
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    task::JoinHandle,
};
use ulos_core::{
    DownloadError,
    preview::{self, DownloadPreview},
    transfer::ResumeCapability,
};

struct Server {
    source: String,
    requests: Arc<Mutex<Vec<String>>>,
    task: JoinHandle<()>,
}

#[tokio::test]
async fn malformed_or_ambiguous_lengths_never_become_an_advisory_size() {
    for headers in [
        "Content-Length: nope\r\n",
        "Content-Length: -1\r\n",
        "Content-Length: +42\r\n",
        "Content-Length: 18446744073709551616\r\n",
        "Content-Length: 42, 43\r\n",
        "Content-Length: 42\r\nContent-Length: 43\r\n",
    ] {
        let server = server("200 OK", headers).await;
        match inspect(&server.source).await {
            Ok(metadata) => assert_eq!(metadata.size, None),
            // Hyper may reject invalid framing before headers reach inspect.
            Err(error) => {
                assert!(matches!(error, DownloadError::Network(_)));
                assert!(!format!("{error:?}").contains("SECRET"));
            }
        }
        assert_head_only(&server);
    }
}

#[tokio::test]
async fn safe_redirect_preserves_head_without_get_or_resume_claims() {
    let destination = server("200 OK", "Content-Length: 73\r\nAccept-Ranges: bytes\r\n").await;
    let redirect = server(
        "302 Found",
        &format!("Location: {}\r\nContent-Length: 0\r\n", destination.source),
    )
    .await;
    let metadata = inspect(&redirect.source).await.unwrap();
    assert_eq!(metadata.size, Some(73));
    assert_eq!(metadata.resume, ResumeCapability::Unknown);
    assert_head_only(&redirect);
    assert_head_only(&destination);
}

#[tokio::test]
async fn head_completes_while_server_keeps_body_and_connection_open() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let source = format!("http://{}/file", listener.local_addr().unwrap());
    let (release, hold) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut request = Vec::new();
        while !request.ends_with(b"\r\n\r\n") {
            request.push(stream.read_u8().await.unwrap());
        }
        assert!(request.starts_with(b"HEAD /file HTTP/1.1\r\n"));
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 999999\r\n\r\n")
            .await
            .unwrap();
        // No resource bytes and no EOF until inspection has already completed.
        let _ = hold.await;
        drop(stream);
    });
    let result = inspect(&source).await;
    let _ = release.send(());
    task.await.unwrap();
    assert_eq!(result.unwrap().size, Some(999999));
}
impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn server(status: &str, headers: &str) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let source = format!(
        "http://{}/file?private=SECRET#fragment",
        listener.local_addr().unwrap()
    );
    let requests = Arc::new(Mutex::new(Vec::new()));
    let recorded = requests.clone();
    let response = format!("HTTP/1.1 {status}\r\n{headers}Connection: close\r\n\r\n");
    let task = tokio::spawn(async move {
        loop {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut bytes = Vec::new();
            loop {
                let mut byte = [0];
                if stream.read_exact(&mut byte).await.is_err() {
                    break;
                }
                bytes.push(byte[0]);
                if bytes.ends_with(b"\r\n\r\n") || bytes.len() > 16_384 {
                    break;
                }
            }
            recorded
                .lock()
                .unwrap()
                .push(String::from_utf8(bytes).unwrap());
            stream.write_all(response.as_bytes()).await.unwrap();
            // Send no resource bytes at all, even when Content-Length is large.
        }
    });
    Server {
        source,
        requests,
        task,
    }
}

async fn inspect(source: &str) -> Result<DownloadPreview, DownloadError> {
    tokio::time::timeout(Duration::from_secs(3), preview::inspect(source))
        .await
        .expect("preview must not wait for a response body")
}

fn assert_head_only(server: &Server) {
    let requests = server.requests.lock().unwrap();
    assert_eq!(requests.len(), 1, "must not fall back to a GET");
    let request = requests[0].to_ascii_lowercase();
    assert!(request.starts_with("head /file?private=secret http/1.1\r\n"));
    assert!(request.contains("accept-encoding: identity\r\n"));
    assert!(!request.contains("authorization:"));
    assert!(!request.contains("referer:"));
    assert!(!request.contains("range:"));
    assert!(!request.contains("fragment"));
}

#[tokio::test]
async fn head_size_is_advisory_and_range_advertisements_never_prove_resume() {
    for length in [0, 123456789] {
        let server = server(
            "200 OK",
            &format!("Content-Length: {length}\r\nAccept-Ranges: bytes\r\nETag: \"stable\"\r\n"),
        )
        .await;
        let result = inspect(&server.source).await.unwrap();
        assert_eq!(
            result,
            DownloadPreview {
                size: Some(length),
                resume: ResumeCapability::Unknown
            }
        );
        assert!(!format!("{result:?}").contains("SECRET"));
        assert_head_only(&server);
    }
}

#[tokio::test]
async fn missing_length_or_encoded_head_is_unknown() {
    for headers in [
        "",
        "Content-Length: 42\r\nContent-Encoding: gzip\r\n",
        "Content-Length: 42\r\nContent-Encoding: identity\r\nContent-Encoding: gzip\r\n",
    ] {
        let server = server("200 OK", headers).await;
        assert_eq!(
            inspect(&server.source).await.unwrap(),
            DownloadPreview {
                size: None,
                resume: ResumeCapability::Unknown
            }
        );
        assert_head_only(&server);
    }
    let server = server(
        "200 OK",
        "Content-Length: 42\r\nContent-Encoding: identity\r\n",
    )
    .await;
    assert_eq!(inspect(&server.source).await.unwrap().size, Some(42));
    assert_head_only(&server);
}

#[tokio::test]
async fn unsupported_head_never_falls_back_to_get() {
    for status in ["405 Method Not Allowed", "501 Not Implemented"] {
        let server = server(status, "Content-Length: 42\r\n").await;
        assert_eq!(
            inspect(&server.source).await.unwrap(),
            DownloadPreview {
                size: None,
                resume: ResumeCapability::Unknown
            }
        );
        assert_head_only(&server);
    }
}

#[tokio::test]
async fn only_complete_success_status_is_accepted() {
    for status in [
        "206 Partial Content",
        "404 Not Found",
        "500 Internal Server Error",
    ] {
        let server = server(status, "Content-Length: 42\r\n").await;
        let error = inspect(&server.source).await.unwrap_err();
        assert!(matches!(error, DownloadError::UnexpectedStatus(_)));
        assert!(!error.to_string().contains("SECRET"));
        assert_head_only(&server);
    }
}

#[tokio::test]
async fn invalid_sources_and_credentials_are_rejected_before_network() {
    for source in ["not a URL", "file:///tmp/private", "ftp://127.0.0.1/file"] {
        assert!(matches!(
            inspect(source).await,
            Err(DownloadError::InvalidUrl)
        ));
    }
    for source in [
        "http://user:SECRET@127.0.0.1/file",
        "http://user@127.0.0.1/file",
    ] {
        assert!(matches!(
            inspect(source).await,
            Err(DownloadError::EmbeddedCredentials)
        ));
    }
}

#[tokio::test]
async fn unsafe_redirects_and_network_errors_are_sanitized() {
    for location in ["file:///tmp/SECRET", "http://user:SECRET@127.0.0.1/file"] {
        let server = server(
            "302 Found",
            &format!("Location: {location}\r\nContent-Length: 0\r\n"),
        )
        .await;
        let error = inspect(&server.source).await.unwrap_err();
        // Reqwest can leave unsupported-scheme redirects unfollowed (302),
        // or reject them through the shared policy as a network error.
        assert!(matches!(
            error,
            DownloadError::Network(_) | DownloadError::UnexpectedStatus(_)
        ));
        assert!(!error.to_string().contains("SECRET"));
        assert!(!format!("{error:?}").contains("SECRET"));
        assert_head_only(&server);
    }
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let source = format!(
        "http://{}/file?private=SECRET",
        listener.local_addr().unwrap()
    );
    drop(listener);
    let error = inspect(&source).await.unwrap_err();
    assert!(matches!(error, DownloadError::Network(_)));
    assert!(!error.to_string().contains("SECRET"));
    assert!(!format!("{error:?}").contains("SECRET"));
}
