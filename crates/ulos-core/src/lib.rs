use std::{
    io,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use reqwest::{Client, StatusCode, header, redirect};
use thiserror::Error;
use tokio::io::AsyncWriteExt;
use url::Url;

#[derive(Error, Debug)]
pub enum DownloadError {
    #[error("Enter a valid HTTP or HTTPS URL")]
    InvalidUrl,
    #[error("URLs containing usernames or passwords are not supported")]
    EmbeddedCredentials,
    #[error("Choose an output file, not a directory")]
    InvalidDestination,
    #[error("The destination already exists; choose a different filename")]
    DestinationExists,
    #[error("Network request failed: {0}")]
    Network(#[source] reqwest::Error),
    #[error("The server returned HTTP {0}; expected a complete HTTP 200 response")]
    UnexpectedStatus(StatusCode),
    #[error("The server returned an encoded response despite requesting identity encoding")]
    UnexpectedEncoding,
    #[error("The transfer length did not match the server's declared length")]
    LengthMismatch,
    #[error("Filesystem operation failed: {0}")]
    Io(#[from] io::Error),
    #[error("Filesystem worker stopped unexpectedly")]
    WorkerStopped,
    #[error("Transfer cancelled before publication")]
    Cancelled,
}

struct PublicationGuard(Arc<AtomicBool>);

impl Drop for PublicationGuard {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}

pub struct DownloadRequest {
    url: Url,
    destination: PathBuf,
}

impl DownloadRequest {
    pub fn new(url: &str, destination: impl Into<PathBuf>) -> Result<Self, DownloadError> {
        let mut url = Url::parse(url).map_err(|_| DownloadError::InvalidUrl)?;
        if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
            return Err(DownloadError::InvalidUrl);
        }
        if !url.username().is_empty() || url.password().is_some() {
            return Err(DownloadError::EmbeddedCredentials);
        }
        url.set_fragment(None);
        let destination = destination.into();
        if destination.file_name().is_none() || destination.is_dir() {
            return Err(DownloadError::InvalidDestination);
        }
        Ok(Self { url, destination })
    }

    pub fn destination(&self) -> &std::path::Path {
        &self.destination
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DownloadProgress {
    pub downloaded_bytes: u64,
    pub total_bytes: Option<u64>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct DownloadReceipt {
    pub destination: PathBuf,
    pub downloaded_bytes: u64,
}

pub struct Downloader {
    client: Client,
}

impl Downloader {
    pub fn new() -> Result<Self, DownloadError> {
        let policy = redirect::Policy::custom(|attempt| {
            let target = attempt.url();
            let downgrade = target.scheme() == "http"
                && attempt.previous().iter().any(|url| url.scheme() == "https");
            if !matches!(target.scheme(), "http" | "https")
                || !target.username().is_empty()
                || target.password().is_some()
                || downgrade
            {
                attempt.error("Unsafe redirect rejected")
            } else if attempt.previous().len() >= 10 {
                attempt.error("Too many redirects")
            } else {
                attempt.follow()
            }
        });
        let client = Client::builder()
            .user_agent(concat!("Ulos/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(Duration::from_secs(10))
            .read_timeout(Duration::from_secs(30))
            .redirect(policy)
            .referer(false)
            .build()
            .map_err(network_error)?;
        Ok(Self { client })
    }

    /// Progress callbacks run inline and must not block the transfer task.
    /// Dropping this future cancels the transfer; temporary-file cleanup is best effort.
    /// Publication already in progress may complete after cancellation.
    pub async fn download(
        &self,
        request: &DownloadRequest,
        mut on_progress: impl FnMut(DownloadProgress),
    ) -> Result<DownloadReceipt, DownloadError> {
        match tokio::fs::symlink_metadata(&request.destination).await {
            Ok(_) => return Err(DownloadError::DestinationExists),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        let parent = request
            .destination
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or_else(|| std::path::Path::new("."));
        let parent = parent.to_path_buf();
        let temporary = tokio::task::spawn_blocking(move || {
            tempfile::Builder::new()
                .prefix(".ulos-")
                .suffix(".part")
                .tempfile_in(parent)
        })
        .await
        .map_err(|_| DownloadError::WorkerStopped)??;
        let (file, temporary_path) = temporary.into_parts();
        let mut file = tokio::fs::File::from_std(file);
        let mut response = self
            .client
            .get(request.url.clone())
            .header(header::ACCEPT_ENCODING, "identity")
            .send()
            .await
            .map_err(network_error)?;
        if response.status() != StatusCode::OK {
            return Err(DownloadError::UnexpectedStatus(response.status()));
        }
        for encoding in response.headers().get_all(header::CONTENT_ENCODING) {
            let value = encoding
                .to_str()
                .map_err(|_| DownloadError::UnexpectedEncoding)?;
            if !value
                .split(',')
                .all(|token| token.trim().eq_ignore_ascii_case("identity"))
            {
                return Err(DownloadError::UnexpectedEncoding);
            }
        }
        let total_bytes = response.content_length();
        let mut downloaded_bytes = 0u64;
        on_progress(DownloadProgress {
            downloaded_bytes,
            total_bytes,
        });
        while let Some(chunk) = response.chunk().await.map_err(network_error)? {
            downloaded_bytes = downloaded_bytes
                .checked_add(chunk.len() as u64)
                .ok_or(DownloadError::LengthMismatch)?;
            if total_bytes.is_some_and(|total| downloaded_bytes > total) {
                return Err(DownloadError::LengthMismatch);
            }
            file.write_all(&chunk).await?;
            on_progress(DownloadProgress {
                downloaded_bytes,
                total_bytes,
            });
        }
        if total_bytes.is_some_and(|total| downloaded_bytes != total) {
            return Err(DownloadError::LengthMismatch);
        }
        file.flush().await?;
        file.sync_all().await?;
        drop(file);
        let destination = request.destination.clone();
        let cancelled = Arc::new(AtomicBool::new(false));
        let _publication_guard = PublicationGuard(cancelled.clone());
        tokio::task::spawn_blocking(move || {
            if cancelled.load(Ordering::Acquire) {
                return Err(DownloadError::Cancelled);
            }
            // Unlike rename on Unix, this never replaces a destination created mid-transfer.
            temporary_path
                .persist_noclobber(destination)
                .map_err(|error| {
                    if error.error.kind() == io::ErrorKind::AlreadyExists {
                        DownloadError::DestinationExists
                    } else {
                        DownloadError::Io(error.error)
                    }
                })
        })
        .await
        .map_err(|_| DownloadError::WorkerStopped)??;
        Ok(DownloadReceipt {
            destination: request.destination.clone(),
            downloaded_bytes,
        })
    }
}

fn network_error(error: reqwest::Error) -> DownloadError {
    DownloadError::Network(error.without_url())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_non_http_urls() {
        for url in ["file:///etc/passwd", "ftp://example.org/file", "not a URL"] {
            assert!(matches!(
                DownloadRequest::new(url, "file.bin"),
                Err(DownloadError::InvalidUrl)
            ));
        }
    }

    #[test]
    fn rejects_embedded_credentials() {
        assert!(matches!(
            DownloadRequest::new("https://user:secret@example.org/file", "file.bin"),
            Err(DownloadError::EmbeddedCredentials)
        ));
    }

    #[test]
    fn removes_fragments_and_preserves_queries() {
        let request =
            DownloadRequest::new("https://example.org/file?token=value#section", "file.bin")
                .unwrap();
        assert_eq!(request.url.fragment(), None);
        assert_eq!(request.url.query(), Some("token=value"));
    }

    #[test]
    fn rejects_directory_destinations() {
        let dir = tempfile::tempdir().unwrap();
        assert!(matches!(
            DownloadRequest::new("https://example.org/file", dir.path()),
            Err(DownloadError::InvalidDestination)
        ));
    }
}
