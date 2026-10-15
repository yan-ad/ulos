//! Metadata-only inspection. Callers own trigger, debounce, and cancellation policy;
//! clipboard detection alone must never initiate a request.
//! HEAD advertisements cannot establish verified range/resume support.
use std::time::Duration;

use reqwest::{StatusCode, header};
use url::Url;

use crate::{
    DownloadError,
    transfer::{self, ResumeCapability},
};

/// Metadata only: source URLs, credentials and validators are not retained.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DownloadPreview {
    pub size: Option<u64>,
    pub resume: ResumeCapability,
}

/// Inspect using HEAD only, without consuming a response body or writing files.
/// A server rejecting HEAD returns unknown metadata; no fallback GET is issued.
/// Reported size is advisory and only retained for identity-encoded responses.
/// Resume remains unknown until an actual transfer verifies ranges and validators.
pub async fn inspect(source: &str) -> Result<DownloadPreview, DownloadError> {
    let mut url = Url::parse(source).map_err(|_| DownloadError::InvalidUrl)?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err(DownloadError::InvalidUrl);
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(DownloadError::EmbeddedCredentials);
    }
    url.set_fragment(None);
    let client = transfer::client_builder()
        .connect_timeout(Duration::from_secs(8))
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|error| DownloadError::Network(error.without_url()))?;
    let response = client
        .head(url)
        .header(header::ACCEPT_ENCODING, "identity")
        .send()
        .await
        .map_err(|error| DownloadError::Network(error.without_url()))?;
    let unknown = DownloadPreview {
        size: None,
        resume: ResumeCapability::Unknown,
    };
    if matches!(
        response.status(),
        StatusCode::METHOD_NOT_ALLOWED | StatusCode::NOT_IMPLEMENTED
    ) {
        return Ok(unknown);
    }
    if response.status() != StatusCode::OK {
        return Err(DownloadError::UnexpectedStatus(response.status()));
    }
    if transfer::identity(&response).is_err() {
        return Ok(unknown);
    }
    // HEAD's body size hint is zero, not the resource length. Read the header
    // explicitly, preserving a real zero and rejecting ambiguous/invalid values.
    let mut lengths = response.headers().get_all(header::CONTENT_LENGTH).iter();
    let size = lengths.next().and_then(|value| {
        let text = value.to_str().ok()?;
        if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        text.parse::<u64>().ok()
    });
    Ok(DownloadPreview {
        size: if lengths.next().is_none() { size } else { None },
        ..unknown
    })
}
