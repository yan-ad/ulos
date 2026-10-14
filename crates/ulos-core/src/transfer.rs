//! Controlled, session-only HTTP transfers. No checkpoint or crash-resume contract.
//!
//! Ranges require an observed 206 and a strong ETag, not Accept-Ranges. A pause
//! closes range responses and resume verifies the remaining ranges with If-Range.
//! Servers without that contract use one full stream (pause is unavailable).
//! Callbacks run inline, must be short, and must not panic. Dropping `run` aborts
//! its range tasks; staging cleanup is best effort. A publication already inside
//! the filesystem commit may finish after cancellation, as in the legacy engine.
use std::{
    io,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use reqwest::{Client, Response, StatusCode, header, redirect};
use tokio::{
    io::{AsyncSeekExt, AsyncWriteExt},
    sync::watch,
    task::JoinSet,
};
use url::Url;

use crate::{DownloadError, DownloadProgress, DownloadReceipt};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CollisionPolicy {
    #[default]
    Reject,
    Overwrite,
    Rename,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TransferOptions {
    pub connections: usize,
    pub bytes_per_second: Option<u64>,
    pub collision: CollisionPolicy,
}

impl Default for TransferOptions {
    fn default() -> Self {
        Self {
            connections: 1,
            bytes_per_second: None,
            collision: CollisionPolicy::Reject,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResumeCapability {
    Unknown,
    Supported,
    Unsupported,
}

impl ResumeCapability {
    pub fn label(self) -> &'static str {
        match self {
            Self::Unknown => "Unknown",
            Self::Supported => "Supported",
            Self::Unsupported => "Unsupported",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConnectionStatus {
    Connecting,
    Downloading,
    Paused,
    Complete,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConnectionProgress {
    pub id: usize,
    pub start: u64,
    /// Inclusive range end, or unknown for an unbounded full stream.
    pub end: Option<u64>,
    pub downloaded: u64,
    pub status: ConnectionStatus,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TransferTelemetry {
    pub progress: DownloadProgress,
    pub resume: ResumeCapability,
    pub connections: Vec<ConnectionProgress>,
    pub paused: bool,
    pub destination: PathBuf,
    pub bytes_per_second: Option<u64>,
}

impl Default for TransferTelemetry {
    fn default() -> Self {
        Self {
            progress: DownloadProgress {
                downloaded_bytes: 0,
                total_bytes: None,
            },
            resume: ResumeCapability::Unknown,
            connections: Vec::new(),
            paused: false,
            destination: PathBuf::new(),
            bytes_per_second: None,
        }
    }
}

#[derive(Clone, Copy)]
struct Settings {
    paused: bool,
    rate: Option<u64>,
}

/// Thread-safe live session settings; contains no source URL or credentials.
#[derive(Clone)]
pub struct TransferControl {
    settings: watch::Sender<Settings>,
}

impl TransferControl {
    pub fn new(bytes_per_second: Option<u64>) -> Self {
        let (settings, _) = watch::channel(Settings {
            paused: false,
            rate: bytes_per_second,
        });
        Self { settings }
    }
    pub fn pause(&self) {
        self.settings.send_modify(|value| value.paused = true);
    }
    pub fn resume(&self) {
        self.settings.send_modify(|value| value.paused = false);
    }
    /// Zero is normalized to one byte/second; run rejects a zero initial limit.
    pub fn set_rate_limit(&self, rate: Option<u64>) {
        self.settings
            .send_modify(|value| value.rate = rate.map(|rate| rate.max(1)));
    }
    pub fn is_paused(&self) -> bool {
        self.settings.borrow().paused
    }
    pub fn rate_limit(&self) -> Option<u64> {
        self.settings.borrow().rate
    }
}

struct Reports<F> {
    value: Mutex<TransferTelemetry>,
    callback: F,
    control: TransferControl,
}

impl<F: Fn(TransferTelemetry)> Reports<F> {
    fn update(&self, change: impl FnOnce(&mut TransferTelemetry)) {
        let mut value = self.value.lock().unwrap_or_else(|error| error.into_inner());
        change(&mut value);
        let settings = *self.control.settings.borrow();
        value.paused = settings.paused
            && value.resume == ResumeCapability::Supported
            && value
                .connections
                .iter()
                .any(|row| row.status != ConnectionStatus::Complete);
        value.bytes_per_second = settings.rate;
        value.progress.downloaded_bytes = value.connections.iter().map(|row| row.downloaded).sum();
        (self.callback)(value.clone());
    }
    fn row(&self, id: usize, change: impl FnOnce(&mut ConnectionProgress)) {
        self.update(|value| change(&mut value.connections[id]));
    }
}

/// One shared budget for all segments. Each grant is charged *before* writing,
/// with no initial burst. A watch change interrupts outstanding waits. Holding
/// this asynchronous lock across the wait intentionally serializes all grants.
struct Limiter {
    budget: tokio::sync::Mutex<()>,
}
impl Limiter {
    async fn grant(&self, length: usize, control: &TransferControl, can_pause: bool) -> bool {
        let mut settings = control.settings.subscribe();
        loop {
            if can_pause && settings.borrow().paused {
                return false;
            }
            let lock = tokio::select! {
                lock = self.budget.lock() => lock,
                _ = settings.changed() => continue,
            };
            let current = *settings.borrow_and_update();
            if can_pause && current.paused {
                return false;
            }
            let Some(rate) = current.rate else {
                return true;
            };
            let delay = Duration::from_secs_f64(length as f64 / rate.max(1) as f64);
            tokio::select! {
                _ = tokio::time::sleep(delay) => return true,
                _ = settings.changed() => { drop(lock); }
            }
        }
    }
}

fn piece_size(control: &TransferControl) -> usize {
    control
        .rate_limit()
        .map(|rate| (rate / 10).clamp(1, 16 * 1024) as usize)
        .unwrap_or(16 * 1024)
}

fn invalid(message: &'static str) -> DownloadError {
    io::Error::new(io::ErrorKind::InvalidData, message).into()
}
fn network(error: reqwest::Error) -> DownloadError {
    DownloadError::Network(error.without_url())
}

fn client() -> Result<Client, DownloadError> {
    Client::builder()
        .user_agent(concat!("Ulos/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(10))
        .read_timeout(Duration::from_secs(30))
        .referer(false)
        .redirect(redirect::Policy::custom(|attempt| {
            let target = attempt.url();
            if !matches!(target.scheme(), "http" | "https")
                || !target.username().is_empty()
                || target.password().is_some()
                || (target.scheme() == "http"
                    && attempt.previous().iter().any(|url| url.scheme() == "https"))
            {
                attempt.error("Unsafe redirect rejected")
            } else if attempt.previous().len() >= 10 {
                attempt.error("Too many redirects")
            } else {
                attempt.follow()
            }
        }))
        .build()
        .map_err(network)
}

fn identity(response: &Response) -> Result<(), DownloadError> {
    for value in response.headers().get_all(header::CONTENT_ENCODING) {
        if !value
            .to_str()
            .map_err(|_| DownloadError::UnexpectedEncoding)?
            .split(',')
            .all(|part| part.trim().eq_ignore_ascii_case("identity"))
        {
            return Err(DownloadError::UnexpectedEncoding);
        }
    }
    Ok(())
}

fn strong_etag(response: &Response) -> Option<header::HeaderValue> {
    let values: Vec<_> = response.headers().get_all(header::ETAG).iter().collect();
    if values.len() != 1 {
        return None;
    }
    let bytes = values[0].as_bytes();
    // RFC opaque-tag: quoted, no embedded quote, control or whitespace.
    (bytes.len() >= 2
        && bytes.first() == Some(&b'"')
        && bytes.last() == Some(&b'"')
        && bytes[1..bytes.len() - 1]
            .iter()
            .all(|byte| *byte == 0x21 || *byte >= 0x23 && *byte != 0x7f))
    .then(|| values[0].clone())
}

fn content_range(response: &Response) -> Result<(u64, u64, u64), DownloadError> {
    let mut values = response.headers().get_all(header::CONTENT_RANGE).iter();
    let value = values
        .next()
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| invalid("Missing or invalid Content-Range"))?;
    if values.next().is_some() {
        return Err(invalid("Ambiguous Content-Range"));
    }
    let parse = || {
        let (range, total) = value.strip_prefix("bytes ")?.split_once('/')?;
        let (start, end) = range.split_once('-')?;
        let number = |value: &str| {
            if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
                None
            } else {
                value.parse::<u64>().ok()
            }
        };
        let (start, end, total) = (number(start)?, number(end)?, number(total)?);
        (start <= end && end < total).then_some((start, end, total))
    };
    parse().ok_or_else(|| invalid("Malformed Content-Range"))
}

fn check_range(
    response: &Response,
    url: &Url,
    etag: &header::HeaderValue,
    start: u64,
    end: u64,
    total: u64,
) -> Result<(), DownloadError> {
    if response.status() != StatusCode::PARTIAL_CONTENT {
        return Err(invalid("Range response changed or is unsupported"));
    }
    identity(response)?;
    if response.url() != url
        || strong_etag(response).as_ref() != Some(etag)
        || content_range(response)? != (start, end, total)
    {
        return Err(invalid("Range representation or validator changed"));
    }
    if response
        .content_length()
        .is_some_and(|length| length != end - start + 1)
    {
        return Err(DownloadError::LengthMismatch);
    }
    Ok(())
}

struct PublicationGuard(Arc<AtomicBool>);
impl Drop for PublicationGuard {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}

async fn destination_check(
    destination: &Path,
    policy: CollisionPolicy,
) -> Result<(), DownloadError> {
    if destination.file_name().is_none() {
        return Err(DownloadError::InvalidDestination);
    }
    match tokio::fs::symlink_metadata(destination).await {
        Ok(metadata) => match policy {
            CollisionPolicy::Reject => Err(DownloadError::DestinationExists),
            CollisionPolicy::Overwrite if !metadata.file_type().is_file() => {
                Err(DownloadError::InvalidDestination)
            }
            CollisionPolicy::Rename if metadata.is_dir() => Err(DownloadError::InvalidDestination),
            _ => Ok(()),
        },
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn numbered(destination: &Path, number: usize) -> PathBuf {
    let name = crate::naming::sanitize_filename(
        &destination
            .file_name()
            .unwrap_or_default()
            .to_string_lossy(),
    );
    let path = Path::new(&name);
    let extension = path
        .extension()
        .map(|ext| format!(".{}", ext.to_string_lossy()))
        .unwrap_or_default();
    let suffix = format!(" ({number}){extension}");
    let mut stem = path
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    let maximum = crate::naming::MAX_FILENAME_BYTES.saturating_sub(suffix.len());
    while stem.len() > maximum {
        stem.pop();
    }
    destination.with_file_name(crate::naming::sanitize_filename(&format!("{stem}{suffix}")))
}

async fn publish(
    mut temporary: tempfile::TempPath,
    destination: PathBuf,
    policy: CollisionPolicy,
    cancelled: Arc<AtomicBool>,
) -> Result<PathBuf, DownloadError> {
    tokio::task::spawn_blocking(move || {
        for number in 0..=10_000 {
            if cancelled.load(Ordering::Acquire) {
                return Err(DownloadError::Cancelled);
            }
            let candidate = if number == 0 {
                destination.clone()
            } else {
                numbered(&destination, number)
            };
            if policy == CollisionPolicy::Overwrite {
                match std::fs::symlink_metadata(&candidate) {
                    Ok(metadata) if !metadata.file_type().is_file() => {
                        return Err(DownloadError::InvalidDestination);
                    }
                    Ok(_) => {}
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error.into()),
                }
                return temporary
                    .persist(&candidate)
                    .map(|()| candidate)
                    .map_err(|error| DownloadError::Io(error.error));
            }
            match temporary.persist_noclobber(&candidate) {
                Ok(()) => return Ok(candidate),
                Err(error) if error.error.kind() == io::ErrorKind::AlreadyExists => {
                    if policy == CollisionPolicy::Reject {
                        return Err(DownloadError::DestinationExists);
                    }
                    temporary = error.path;
                }
                Err(error) => return Err(error.error.into()),
            }
        }
        Err(DownloadError::DestinationExists)
    })
    .await
    .map_err(|_| DownloadError::WorkerStopped)?
}

async fn wait_resume<F: Fn(TransferTelemetry)>(id: usize, reports: &Reports<F>) {
    let mut settings = reports.control.settings.subscribe();
    while settings.borrow_and_update().paused {
        reports.row(id, |row| row.status = ConnectionStatus::Paused);
        let _ = settings.changed().await;
    }
}

struct Resource {
    client: Client,
    url: Url,
    etag: header::HeaderValue,
    total: u64,
    path: PathBuf,
}

async fn segment<F: Fn(TransferTelemetry)>(
    resource: Arc<Resource>,
    id: usize,
    start: u64,
    end: u64,
    reports: Arc<Reports<F>>,
    limiter: Arc<Limiter>,
) -> Result<(), DownloadError> {
    let mut file = tokio::fs::OpenOptions::new()
        .write(true)
        .open(&resource.path)
        .await?;
    file.seek(io::SeekFrom::Start(start)).await?;
    let mut offset = start;
    let mut settings = reports.control.settings.subscribe();
    'reconnect: while offset <= end {
        wait_resume(id, &reports).await;
        reports.row(id, |row| row.status = ConnectionStatus::Connecting);
        let request = resource
            .client
            .get(resource.url.clone())
            .header(header::ACCEPT_ENCODING, "identity")
            .header(header::RANGE, format!("bytes={offset}-{end}"))
            .header(header::IF_RANGE, resource.etag.clone())
            .send();
        let mut response = tokio::select! {
            response = request => response.map_err(network)?,
            _ = settings.changed() => continue 'reconnect,
        };
        check_range(
            &response,
            &resource.url,
            &resource.etag,
            offset,
            end,
            resource.total,
        )?;
        reports.row(id, |row| row.status = ConnectionStatus::Downloading);
        loop {
            if reports.control.is_paused() && offset <= end {
                continue 'reconnect;
            }
            let chunk = tokio::select! {
                chunk = response.chunk() => chunk.map_err(network)?,
                _ = settings.changed() => {
                    reports.update(|_| {});
                    if reports.control.is_paused() && offset <= end { continue 'reconnect; }
                    continue;
                },
            };
            let Some(chunk) = chunk else {
                break;
            };
            if chunk.len() as u64 > (end + 1).saturating_sub(offset) {
                return Err(DownloadError::LengthMismatch);
            }
            for piece in chunk.chunks(piece_size(&reports.control)) {
                if !limiter.grant(piece.len(), &reports.control, true).await
                    || reports.control.is_paused()
                {
                    continue 'reconnect;
                }
                file.write_all(piece).await?;
                offset += piece.len() as u64;
                reports.row(id, |row| row.downloaded = offset - start);
            }
        }
        if offset != end + 1 {
            return Err(DownloadError::LengthMismatch);
        }
    }
    file.flush().await?;
    reports.row(id, |row| row.status = ConnectionStatus::Complete);
    Ok(())
}

async fn full_stream<F: Fn(TransferTelemetry)>(
    mut response: Response,
    file: &mut tokio::fs::File,
    reports: &Reports<F>,
    limiter: &Limiter,
) -> Result<u64, DownloadError> {
    if response.status() != StatusCode::OK {
        return Err(DownloadError::UnexpectedStatus(response.status()));
    }
    identity(&response)?;
    let total = response.content_length();
    reports.update(|value| {
        value.resume = ResumeCapability::Unsupported;
        value.progress.total_bytes = total;
        value.connections = vec![ConnectionProgress {
            id: 0,
            start: 0,
            end: total.and_then(|length| length.checked_sub(1)),
            downloaded: 0,
            status: ConnectionStatus::Downloading,
        }];
    });
    let mut downloaded = 0u64;
    while let Some(chunk) = response.chunk().await.map_err(network)? {
        let next = downloaded
            .checked_add(chunk.len() as u64)
            .ok_or(DownloadError::LengthMismatch)?;
        if total.is_some_and(|total| next > total) {
            return Err(DownloadError::LengthMismatch);
        }
        for piece in chunk.chunks(piece_size(&reports.control)) {
            limiter.grant(piece.len(), &reports.control, false).await;
            file.write_all(piece).await?;
            downloaded += piece.len() as u64;
            reports.row(0, |row| row.downloaded = downloaded);
        }
    }
    if total.is_some_and(|total| downloaded != total) {
        return Err(DownloadError::LengthMismatch);
    }
    reports.row(0, |row| row.status = ConnectionStatus::Complete);
    Ok(downloaded)
}

/// Run on a Tokio runtime. Connections must be 1..=8; the initial limit must be
/// positive. `control` owns live settings (normally initialized from options).
/// Neither source URLs nor server validators are exposed in telemetry/errors.
pub async fn run(
    source: &str,
    destination: PathBuf,
    options: TransferOptions,
    control: TransferControl,
    on_update: impl Fn(TransferTelemetry) + Send + Sync + 'static,
) -> Result<DownloadReceipt, DownloadError> {
    if !(1..=8).contains(&options.connections)
        || options.bytes_per_second == Some(0)
        || control.rate_limit() == Some(0)
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Invalid connection count or rate limit",
        )
        .into());
    }
    let mut url = Url::parse(source).map_err(|_| DownloadError::InvalidUrl)?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err(DownloadError::InvalidUrl);
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(DownloadError::EmbeddedCredentials);
    }
    url.set_fragment(None);
    destination_check(&destination, options.collision).await?;
    let parent = destination
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf();
    let temporary = tokio::task::spawn_blocking(move || {
        tempfile::Builder::new()
            .prefix(".ulos-")
            .suffix(".part")
            .tempfile_in(parent)
    })
    .await
    .map_err(|_| DownloadError::WorkerStopped)??;
    let (file, temporary) = temporary.into_parts();
    let mut file = tokio::fs::File::from_std(file);
    let cancelled = Arc::new(AtomicBool::new(false));
    let _publication_guard = PublicationGuard(cancelled.clone());
    let reports = Arc::new(Reports {
        value: Mutex::new(TransferTelemetry {
            destination: destination.clone(),
            ..TransferTelemetry::default()
        }),
        callback: on_update,
        control,
    });
    reports.update(|_| {});
    let limiter = Arc::new(Limiter {
        budget: tokio::sync::Mutex::new(()),
    });
    let client = client()?;
    let mut probe = client
        .get(url.clone())
        .header(header::ACCEPT_ENCODING, "identity")
        .header(header::RANGE, "bytes=0-0")
        .send()
        .await
        .map_err(network)?;
    identity(&probe)?;
    let downloaded = if probe.status() == StatusCode::OK {
        full_stream(probe, &mut file, &reports, &limiter).await?
    } else if probe.status() == StatusCode::PARTIAL_CONTENT {
        let (start, end, total) = content_range(&probe)?;
        if (start, end) != (0, 0) || probe.content_length().is_some_and(|length| length != 1) {
            return Err(invalid("Invalid range probe"));
        }
        let etag = strong_etag(&probe);
        let effective_url = probe.url().clone();
        // The probe is deliberately consumed with a hard one-byte bound.
        let mut length = 0;
        while let Some(chunk) = probe.chunk().await.map_err(network)? {
            length += chunk.len();
            if length > 1 {
                return Err(DownloadError::LengthMismatch);
            }
        }
        if length != 1 {
            return Err(DownloadError::LengthMismatch);
        }
        if let Some(etag) = etag {
            let count = options
                .connections
                .min(usize::try_from(total).unwrap_or(usize::MAX));
            let base = total / count as u64;
            let remainder = total % count as u64;
            let mut start = 0;
            let mut rows = Vec::with_capacity(count);
            for id in 0..count {
                let length = base + u64::from((id as u64) < remainder);
                rows.push(ConnectionProgress {
                    id,
                    start,
                    end: Some(start + length - 1),
                    downloaded: 0,
                    status: ConnectionStatus::Connecting,
                });
                start += length;
            }
            file.set_len(total).await?;
            reports.update(|value| {
                value.resume = ResumeCapability::Supported;
                value.progress.total_bytes = Some(total);
                value.connections = rows.clone();
            });
            let resource = Arc::new(Resource {
                client,
                url: effective_url,
                etag,
                total,
                path: temporary.to_path_buf(),
            });
            let mut tasks = JoinSet::new();
            for row in rows {
                tasks.spawn(segment(
                    resource.clone(),
                    row.id,
                    row.start,
                    row.end.unwrap(),
                    reports.clone(),
                    limiter.clone(),
                ));
            }
            while let Some(result) = tasks.join_next().await {
                result.map_err(|_| DownloadError::WorkerStopped)??;
            }
            total
        } else {
            let response = client
                .get(effective_url)
                .header(header::ACCEPT_ENCODING, "identity")
                .send()
                .await
                .map_err(network)?;
            full_stream(response, &mut file, &reports, &limiter).await?
        }
    } else if probe.status() == StatusCode::RANGE_NOT_SATISFIABLE {
        drop(probe);
        let response = client
            .get(url)
            .header(header::ACCEPT_ENCODING, "identity")
            .send()
            .await
            .map_err(network)?;
        full_stream(response, &mut file, &reports, &limiter).await?
    } else {
        return Err(DownloadError::UnexpectedStatus(probe.status()));
    };
    file.flush().await?;
    file.sync_all().await?;
    drop(file);
    let destination = publish(temporary, destination, options.collision, cancelled).await?;
    reports.update(|value| {
        value.destination = destination.clone();
        value.paused = false;
    });
    Ok(DownloadReceipt {
        destination,
        downloaded_bytes: downloaded,
    })
}
