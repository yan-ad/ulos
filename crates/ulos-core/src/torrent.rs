//! Tracker-only, explicitly started v1 `.torrent` downloads (including v1 hybrids).
//!
//! Preview is bounded and entirely offline. Sources deliberately have no Debug
//! implementation: tracker queries can contain secrets. Uploads are disabled by
//! both the engine feature and session option; reciprocity-dependent swarms may
//! therefore not make progress. No magnet, DHT, LSD, listener, UPnP, persistence,
//! HTTP server, seeding, or rate-control API is enabled here.
//!
//! Each worker owns a private Tokio runtime. After `Session::stop` it drops that
//! runtime, joining blocking work and destroying remaining engine tasks/file
//! handles *before* verification, publication or cleanup. The caller must keep
//! its runtime alive until terminal state; Drop only requests cancellation.
//! Cancellation during the atomic publication step may lose to success. Staging
//! and destination parents must not be modified by a hostile local process.
//! Staging cleanup is explicitly attempted before terminal state. A cleanup failure
//! before publication reports `Cleanup` (also on cancellation). After publication,
//! cleanup is best effort: a valid published receipt always wins; staging remnants
//! may remain if the filesystem refuses removal. Terminal success is not a promise
//! that staging cleanup succeeded.

pub use crate::transfer::CollisionPolicy;
use crate::{DownloadProgress, DownloadReceipt, naming::sanitize_filename, tasks::TransferState};
use sha1::{Digest, Sha1};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    io::Read,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tokio::{
    runtime::Handle,
    sync::{oneshot, watch},
};
use unicode_casefold::UnicodeCaseFold;
use unicode_normalization::UnicodeNormalization;

pub const MAX_METAINFO_BYTES: usize = 8 * 1024 * 1024;
const MAX_FILES: usize = 4096;
const MAX_PIECES: usize = 262_144;
const MAX_TRACKERS: usize = 64;

/// Stable, redacted errors. Never contains a path, tracker, or engine diagnostic.
#[derive(Debug, thiserror::Error)]
pub enum TorrentError {
    #[error("Torrent metadata exceeds safety limits")]
    Limit,
    #[error("Invalid or unsupported torrent metadata")]
    InvalidMetadata,
    #[error("Torrent contains an unsafe or conflicting file path")]
    UnsafePath,
    #[error("Torrent contains an unsupported or credential-bearing tracker")]
    UnsafeTracker,
    #[error("Could not read a regular torrent file")]
    Read,
    #[error("Invalid torrent destination")]
    Destination,
    #[error("Torrent destination already exists")]
    DestinationExists,
    #[error("Torrent overwrite is not supported")]
    OverwriteUnsupported,
    #[error("Torrent engine could not complete the transfer")]
    Engine,
    #[error("Torrent data verification failed")]
    Verification,
    #[error("Could not publish torrent without replacing existing data")]
    Publication,
    #[error("Could not remove torrent staging data")]
    Cleanup,
    #[error("Torrent worker could not run")]
    Worker,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TorrentFile {
    /// Relative to the selected destination directory for multi-file torrents;
    /// the metadata name for a single-file torrent. Never includes the root name.
    pub path: PathBuf,
    pub length: u64,
}

#[derive(Clone, PartialEq, Eq)]
pub struct TorrentSource {
    bytes: Arc<[u8]>,
    name: String,
    files: Arc<[TorrentFile]>,
    total: u64,
    multi: bool,
    trackers: usize,
    hash: String,
    piece_length: usize,
    pieces: Arc<[u8]>,
}
impl TorrentSource {
    pub fn name(&self) -> &str {
        &self.name
    }
    pub fn total_bytes(&self) -> u64 {
        self.total
    }
    pub fn file_count(&self) -> usize {
        self.files.len()
    }
    pub fn is_multi_file(&self) -> bool {
        self.multi
    }
    pub fn files(&self) -> &[TorrentFile] {
        &self.files
    }
    pub fn tracker_count(&self) -> usize {
        self.trackers
    }
    pub fn info_hash(&self) -> String {
        self.hash.clone()
    }
}

// A small strict bencode parser bounds depth and node count BEFORE handing any
// untrusted metadata to the engine's recursive deserializer. Borrow byte strings.
#[derive(Debug)]
enum Value<'a> {
    Int(u64),
    Bytes(&'a [u8]),
    List(Vec<Value<'a>>),
    Dict(BTreeMap<&'a [u8], Value<'a>>),
}
impl<'a> Value<'a> {
    fn dict(&self) -> Result<&BTreeMap<&'a [u8], Value<'a>>, TorrentError> {
        if let Self::Dict(v) = self {
            Ok(v)
        } else {
            Err(TorrentError::InvalidMetadata)
        }
    }
    fn bytes(&self) -> Result<&'a [u8], TorrentError> {
        if let Self::Bytes(v) = self {
            Ok(v)
        } else {
            Err(TorrentError::InvalidMetadata)
        }
    }
    fn int(&self) -> Result<u64, TorrentError> {
        if let Self::Int(v) = self {
            Ok(*v)
        } else {
            Err(TorrentError::InvalidMetadata)
        }
    }
    fn list(&self) -> Result<&[Value<'a>], TorrentError> {
        if let Self::List(v) = self {
            Ok(v)
        } else {
            Err(TorrentError::InvalidMetadata)
        }
    }
}
fn field<'a, 'b>(
    d: &'b BTreeMap<&'a [u8], Value<'a>>,
    key: &[u8],
) -> Result<&'b Value<'a>, TorrentError> {
    d.get(key).ok_or(TorrentError::InvalidMetadata)
}
struct Parser<'a> {
    bytes: &'a [u8],
    pos: usize,
    nodes: usize,
    info: Option<std::ops::Range<usize>>,
}
impl<'a> Parser<'a> {
    fn string(&mut self) -> Result<&'a [u8], TorrentError> {
        let start = self.pos;
        while self.bytes.get(self.pos).is_some_and(u8::is_ascii_digit) {
            self.pos += 1;
        }
        let digits = &self.bytes[start..self.pos];
        if digits.is_empty()
            || digits.len() > 8
            || (digits.len() > 1 && digits[0] == b'0')
            || self.bytes.get(self.pos) != Some(&b':')
        {
            return Err(TorrentError::InvalidMetadata);
        }
        let len = std::str::from_utf8(digits)
            .ok()
            .and_then(|s| s.parse::<usize>().ok())
            .ok_or(TorrentError::InvalidMetadata)?;
        self.pos += 1;
        let end = self
            .pos
            .checked_add(len)
            .filter(|e| *e <= self.bytes.len())
            .ok_or(TorrentError::InvalidMetadata)?;
        let value = &self.bytes[self.pos..end];
        self.pos = end;
        Ok(value)
    }
    fn value(&mut self, depth: usize) -> Result<Value<'a>, TorrentError> {
        self.nodes += 1;
        if depth > 32 || self.nodes > 65_536 {
            return Err(TorrentError::Limit);
        }
        match self.bytes.get(self.pos).copied() {
            Some(b'0'..=b'9') => Ok(Value::Bytes(self.string()?)),
            Some(b'i') => {
                self.pos += 1;
                let start = self.pos;
                while self.bytes.get(self.pos).is_some_and(u8::is_ascii_digit) {
                    self.pos += 1;
                }
                let digits = &self.bytes[start..self.pos];
                if digits.is_empty()
                    || digits.len() > 20
                    || (digits.len() > 1 && digits[0] == b'0')
                    || self.bytes.get(self.pos) != Some(&b'e')
                {
                    return Err(TorrentError::InvalidMetadata);
                }
                self.pos += 1;
                Ok(Value::Int(
                    std::str::from_utf8(digits)
                        .ok()
                        .and_then(|s| s.parse().ok())
                        .ok_or(TorrentError::InvalidMetadata)?,
                ))
            }
            Some(b'l') => {
                self.pos += 1;
                let mut v = Vec::new();
                while self.bytes.get(self.pos) != Some(&b'e') {
                    v.push(self.value(depth + 1)?);
                }
                self.pos += 1;
                Ok(Value::List(v))
            }
            Some(b'd') => {
                self.pos += 1;
                let mut v = BTreeMap::new();
                let mut previous: Option<&[u8]> = None;
                while self.bytes.get(self.pos) != Some(&b'e') {
                    let key = self.string()?;
                    if previous.is_some_and(|p| p >= key) {
                        return Err(TorrentError::InvalidMetadata);
                    }
                    previous = Some(key);
                    let start = self.pos;
                    let value = self.value(depth + 1)?;
                    if depth == 0 && key == b"info" {
                        self.info = Some(start..self.pos);
                    }
                    v.insert(key, value);
                }
                self.pos += 1;
                Ok(Value::Dict(v))
            }
            _ => Err(TorrentError::InvalidMetadata),
        }
    }
}
fn component(bytes: &[u8]) -> Result<String, TorrentError> {
    let s = std::str::from_utf8(bytes).map_err(|_| TorrentError::UnsafePath)?;
    if s.len() > 180
        || sanitize_filename(s) != s
        || s.chars()
            .any(|c| matches!(c, '\u{200b}'..='\u{200d}' | '\u{feff}'))
    {
        return Err(TorrentError::UnsafePath);
    }
    Ok(s.to_owned())
}
fn regular(d: &BTreeMap<&[u8], Value<'_>>) -> Result<(), TorrentError> {
    if d.contains_key(b"symlink path".as_slice())
        || d.contains_key(b"path.utf-8".as_slice())
        || d.contains_key(b"name.utf-8".as_slice())
        || d.get(b"attr".as_slice())
            .map(|v| v.bytes().map(|v| v.contains(&b'l') || v.contains(&b'p')))
            .transpose()?
            .unwrap_or(false)
    {
        return Err(TorrentError::UnsafePath);
    }
    Ok(())
}

/// Validates v1 metadata without touching disk or the network. Pure v2, negative
/// bencode integers, non-canonical dictionaries, padding and symlink files are
/// intentionally outside this initial strict profile. Empty total is rejected.
pub fn parse(bytes: Vec<u8>) -> Result<TorrentSource, TorrentError> {
    if bytes.len() > MAX_METAINFO_BYTES {
        return Err(TorrentError::Limit);
    }
    let mut parser = Parser {
        bytes: &bytes,
        pos: 0,
        nodes: 0,
        info: None,
    };
    let root = parser.value(0)?;
    if parser.pos != bytes.len() {
        return Err(TorrentError::InvalidMetadata);
    }
    let d = root.dict()?;
    if let Some(enc) = d.get(b"encoding".as_slice())
        && !enc.bytes()?.eq_ignore_ascii_case(b"utf-8")
    {
        return Err(TorrentError::InvalidMetadata);
    }
    let info = field(d, b"info")?.dict()?;
    regular(info)?;
    let name = component(field(info, b"name")?.bytes()?)?;
    let piece_length = field(info, b"piece length")?.int()?;
    if !(16_384..=64 * 1024 * 1024).contains(&piece_length) || !piece_length.is_power_of_two() {
        return Err(TorrentError::Limit);
    }
    let pieces = field(info, b"pieces")?.bytes()?;
    if pieces.len() % 20 != 0 || pieces.len() / 20 > MAX_PIECES {
        return Err(TorrentError::Limit);
    }
    let multi = info.contains_key(b"files".as_slice());
    let files = if multi {
        if info.contains_key(b"length".as_slice()) {
            return Err(TorrentError::InvalidMetadata);
        }
        let list = field(info, b"files")?.list()?;
        if list.is_empty() || list.len() > MAX_FILES {
            return Err(TorrentError::Limit);
        }
        let mut files = Vec::with_capacity(list.len());
        for f in list {
            let f = f.dict()?;
            regular(f)?;
            let parts = field(f, b"path")?.list()?;
            if parts.is_empty() || parts.len() > 32 {
                return Err(TorrentError::UnsafePath);
            }
            let mut path = PathBuf::new();
            for p in parts {
                path.push(component(p.bytes()?)?);
            }
            if path.as_os_str().len() > 2048 {
                return Err(TorrentError::UnsafePath);
            }
            files.push(TorrentFile {
                path,
                length: field(f, b"length")?.int()?,
            });
        }
        files
    } else {
        vec![TorrentFile {
            path: PathBuf::from(&name),
            length: field(info, b"length")?.int()?,
        }]
    };
    // Unicode canonical normalization + full case folding protects case-insensitive
    // filesystems, and directory-prefix conflicts are rejected before engine I/O.
    let mut paths = BTreeSet::new();
    let mut directories = BTreeMap::new();
    let mut total = 0u64;
    for file in &files {
        total = total.checked_add(file.length).ok_or(TorrentError::Limit)?;
        let key: String = file
            .path
            .to_str()
            .ok_or(TorrentError::UnsafePath)?
            .nfc()
            .case_fold()
            .collect();
        if paths.contains(&key) || directories.contains_key(&key) {
            return Err(TorrentError::UnsafePath);
        }
        let original = file.path.to_str().ok_or(TorrentError::UnsafePath)?;
        for (i, _) in original.match_indices(std::path::MAIN_SEPARATOR) {
            let spelling = &original[..i];
            let parent: String = spelling.nfc().case_fold().collect();
            if paths.contains(&parent)
                || directories
                    .get(&parent)
                    .is_some_and(|previous| previous != spelling)
            {
                return Err(TorrentError::UnsafePath);
            }
            directories.insert(parent, spelling.to_owned());
        }
        paths.insert(key);
    }
    if total == 0
        || total > i64::MAX as u64
        || total.div_ceil(piece_length) != (pieces.len() / 20) as u64
    {
        return Err(TorrentError::InvalidMetadata);
    }
    let mut trackers = BTreeSet::new();
    let mut tracker_entries = 0usize;
    let mut add_tracker = |v: &Value<'_>| -> Result<(), TorrentError> {
        tracker_entries += 1;
        if tracker_entries > MAX_TRACKERS {
            return Err(TorrentError::Limit);
        }
        let raw = std::str::from_utf8(v.bytes()?).map_err(|_| TorrentError::UnsafeTracker)?;
        if raw.len() > 4096 {
            return Err(TorrentError::Limit);
        }
        let u = url::Url::parse(raw).map_err(|_| TorrentError::UnsafeTracker)?;
        if !matches!(u.scheme(), "http" | "https" | "udp")
            || u.host_str().is_none()
            || !u.username().is_empty()
            || u.password().is_some()
            || u.fragment().is_some()
            || raw
                .split("://")
                .nth(1)
                .and_then(|v| v.split(['/', '?', '#']).next())
                .is_some_and(|v| v.contains('@'))
        {
            return Err(TorrentError::UnsafeTracker);
        }
        trackers.insert(u.to_string());
        Ok(())
    };
    if let Some(a) = d.get(b"announce".as_slice()) {
        add_tracker(a)?;
    }
    if let Some(tiers) = d.get(b"announce-list".as_slice()) {
        for tier in tiers.list()? {
            for tracker in tier.list()? {
                add_tracker(tracker)?;
            }
        }
    }
    let hash = format!(
        "{:x}",
        Sha1::digest(&bytes[parser.info.ok_or(TorrentError::InvalidMetadata)?])
    );
    // The engine is invoked only after the bounded, strict validation above.
    let _quiet = tracing::subscriber::set_default(tracing::subscriber::NoSubscriber::default());
    let meta = librqbit::torrent_from_bytes(&bytes).map_err(|_| TorrentError::InvalidMetadata)?;
    let validated = meta
        .info
        .data
        .validate()
        .map_err(|_| TorrentError::InvalidMetadata)?;
    // Engine encoding detection must produce precisely the approved manifest,
    // never a lossy reinterpretation of an accepted UTF-8 path.
    let engine_files: Vec<_> = validated.iter_file_details().collect();
    if engine_files.len() != files.len()
        || engine_files
            .iter()
            .zip(&files)
            .any(|(a, b)| a.filename.to_pathbuf() != b.path || a.len != b.length)
    {
        return Err(TorrentError::UnsafePath);
    }
    let pieces = Arc::from(pieces);
    Ok(TorrentSource {
        bytes: bytes.into(),
        name,
        files: files.into(),
        total,
        multi,
        trackers: trackers.len(),
        hash,
        piece_length: piece_length as usize,
        pieces,
    })
}

/// Bounded worker-side regular-file read. Symlinks (including a replacement
/// between inspection and open on Unix) are rejected; no tracker is contacted.
pub async fn load(path: PathBuf) -> Result<TorrentSource, TorrentError> {
    tokio::task::spawn_blocking(move || {
        let meta = fs::symlink_metadata(&path).map_err(|_| TorrentError::Read)?;
        if !meta.is_file() || meta.len() > MAX_METAINFO_BYTES as u64 {
            return Err(TorrentError::Read);
        }
        let mut opts = OpenOptions::new();
        opts.read(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.custom_flags(
                rustix::fs::OFlags::NOFOLLOW.bits() as i32
                    | rustix::fs::OFlags::NONBLOCK.bits() as i32,
            );
        }
        let file = opts.open(path).map_err(|_| TorrentError::Read)?;
        if !file.metadata().map_err(|_| TorrentError::Read)?.is_file() {
            return Err(TorrentError::Read);
        }
        let mut bytes = Vec::new();
        file.take((MAX_METAINFO_BYTES + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|_| TorrentError::Read)?;
        parse(bytes)
    })
    .await
    .map_err(|_| TorrentError::Worker)?
}

#[derive(Clone, Default)]
pub struct TorrentOptions {
    pub collision: CollisionPolicy,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TorrentPhase {
    Preparing,
    Checking,
    Downloading,
    Paused,
    Publishing,
}
#[derive(Clone, Debug, PartialEq)]
pub struct TorrentTelemetry {
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
    pub peers: u32,
    pub uploaded_bytes: u64,
    pub phase: TorrentPhase,
}

pub struct TorrentDownloadTask {
    state: watch::Receiver<TransferState>,
    telemetry: watch::Receiver<TorrentTelemetry>,
    cancellation: Option<oneshot::Sender<()>>,
    control: watch::Sender<bool>,
}
impl TorrentDownloadTask {
    /// Destination is the complete final filename/directory, not its parent.
    pub fn start(
        runtime: &Handle,
        source: TorrentSource,
        destination: PathBuf,
        options: TorrentOptions,
    ) -> Self {
        let (state_tx, state) = watch::channel(TransferState::Preparing);
        let (telemetry_tx, telemetry) = watch::channel(TorrentTelemetry {
            downloaded_bytes: 0,
            total_bytes: source.total,
            peers: 0,
            uploaded_bytes: 0,
            phase: TorrentPhase::Preparing,
        });
        let (cancellation, cancelled) = oneshot::channel();
        let (control, commands) = watch::channel(false);
        runtime.spawn(async move {
            let updates = state_tx.clone();
            let result = tokio::task::spawn_blocking(move || {
                // Engine diagnostics may carry tracker queries. Suppress tracing
                // locally on every owned engine thread, never install a global subscriber.
                let _quiet =
                    tracing::subscriber::set_default(tracing::subscriber::NoSubscriber::default());
                worker(
                    source,
                    destination,
                    options,
                    updates,
                    telemetry_tx,
                    cancelled,
                    commands,
                )
            })
            .await;
            let terminal = match result {
                Ok(Ok(Some(receipt))) => TransferState::Completed(receipt),
                Ok(Ok(None)) => TransferState::Cancelled,
                Ok(Err(e)) => TransferState::Failed(e.to_string()),
                Err(_) => TransferState::Failed(TorrentError::Worker.to_string()),
            };
            state_tx.send_replace(terminal);
        });
        Self {
            state,
            telemetry,
            cancellation: Some(cancellation),
            control,
        }
    }
    pub fn subscribe(&self) -> watch::Receiver<TransferState> {
        self.state.clone()
    }
    pub fn telemetry(&self) -> watch::Receiver<TorrentTelemetry> {
        self.telemetry.clone()
    }
    /// True means a command was accepted, not that the async pause has completed.
    /// Observe Paused before treating the swarm as paused. The queue slot is held.
    pub fn pause(&self) -> bool {
        if !matches!(*self.state.borrow(), TransferState::Downloading(_))
            || *self.control.borrow()
            || self.telemetry.borrow().phase != TorrentPhase::Downloading
        {
            return false;
        }
        self.control.send_replace(true);
        true
    }
    pub fn resume(&self) -> bool {
        if self.state.borrow().is_terminal() || !*self.control.borrow() {
            return false;
        }
        self.control.send_replace(false);
        true
    }
    pub fn cancel(&mut self) {
        if let Some(tx) = self.cancellation.take() {
            let _ = tx.send(());
        }
    }
}
impl Drop for TorrentDownloadTask {
    fn drop(&mut self) {
        self.cancel();
    }
}

thread_local! {static QUIET: std::cell::RefCell<Option<tracing::subscriber::DefaultGuard>> = const { std::cell::RefCell::new(None) };}

fn worker(
    source: TorrentSource,
    destination: PathBuf,
    options: TorrentOptions,
    state: watch::Sender<TransferState>,
    telemetry: watch::Sender<TorrentTelemetry>,
    mut cancelled: oneshot::Receiver<()>,
    commands: watch::Receiver<bool>,
) -> Result<Option<DownloadReceipt>, TorrentError> {
    if options.collision == CollisionPolicy::Overwrite {
        return Err(TorrentError::OverwriteUnsupported);
    }
    let parent = destination
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let parent = fs::canonicalize(parent).map_err(|_| TorrentError::Destination)?;
    if !parent.is_dir() {
        return Err(TorrentError::Destination);
    }
    let name = destination
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or(TorrentError::Destination)?;
    component(name.as_bytes()).map_err(|_| TorrentError::Destination)?;
    let destination = parent.join(name);
    if options.collision == CollisionPolicy::Reject && fs::symlink_metadata(&destination).is_ok() {
        return Err(TorrentError::DestinationExists);
    }
    let staging = tempfile::Builder::new()
        .prefix(".ulos-torrent-")
        .tempdir_in(&parent)
        .map_err(|_| TorrentError::Destination)?;
    let result = (|| {
        let output = staging.path().join("payload");
        fs::create_dir(&output).map_err(|_| TorrentError::Destination)?;
        if cancellation_requested(&mut cancelled) {
            return Ok(None);
        }
        let engine = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .on_thread_start(|| {
                QUIET.with(|guard| {
                    *guard.borrow_mut() = Some(tracing::subscriber::set_default(
                        tracing::subscriber::NoSubscriber::default(),
                    ))
                });
            })
            .build()
            .map_err(|_| TorrentError::Worker)?;
        let result = engine.block_on(run_engine(
            &source,
            &output,
            &state,
            &telemetry,
            &mut cancelled,
            commands,
        ));
        // Session::stop alone has a timed heuristic. Runtime destruction is the
        // ownership barrier: all child futures and blocking I/O are gone afterward.
        drop(engine);
        if !result? {
            return Ok(None);
        }
        if cancellation_requested(&mut cancelled) {
            return Ok(None);
        }
        telemetry.send_modify(|t| {
            t.phase = TorrentPhase::Publishing;
            t.peers = 0;
        });
        if !verify(&source, &output, || cancellation_requested(&mut cancelled))? {
            return Ok(None);
        }
        if cancellation_requested(&mut cancelled) {
            return Ok(None);
        }
        let payload = if source.multi {
            output
        } else {
            output.join(&source.name)
        };
        let published = publish(&payload, &destination, source.multi, options.collision)?;
        Ok(Some(DownloadReceipt {
            destination: published,
            downloaded_bytes: source.total,
        }))
    })();
    finish_cleanup(result, staging.close())
}

// Cleanup errors must not turn an already published download into a failure.
fn finish_cleanup(
    result: Result<Option<DownloadReceipt>, TorrentError>,
    cleanup: std::io::Result<()>,
) -> Result<Option<DownloadReceipt>, TorrentError> {
    if cleanup.is_err() && !matches!(result, Ok(Some(_))) {
        return Err(TorrentError::Cleanup);
    }
    result
}

fn cancellation_requested(cancelled: &mut oneshot::Receiver<()>) -> bool {
    cancelled.try_recv() != Err(oneshot::error::TryRecvError::Empty)
}

async fn run_engine(
    source: &TorrentSource,
    output: &Path,
    state: &watch::Sender<TransferState>,
    telemetry: &watch::Sender<TorrentTelemetry>,
    cancelled: &mut oneshot::Receiver<()>,
    mut commands: watch::Receiver<bool>,
) -> Result<bool, TorrentError> {
    let opts = librqbit::SessionOptions {
        dht: None,
        disable_local_service_discovery: true,
        disable_upload: true,
        persistence: None,
        fastresume: false,
        listen: None,
        peer_limit: Some(40),
        ..Default::default()
    };
    // Construction itself has no trackers/peers yet. Always retain the session
    // through add cancellation so partial initialization is also torn down.
    let session = librqbit::Session::new_with_opts(output.to_path_buf(), opts)
        .await
        .map_err(|_| TorrentError::Engine)?;
    let result=async {
        let added=tokio::select! {
            biased;
            _=&mut *cancelled=>return Ok(false),
            r=session.add_torrent(librqbit::AddTorrent::from_bytes(source.bytes.to_vec()),Some(librqbit::AddTorrentOptions {output_folder:Some(output.to_str().ok_or(TorrentError::Destination)?.to_owned()),..Default::default()}))=>r.map_err(|_|TorrentError::Engine)?,
        };
        let handle=added.into_handle().ok_or(TorrentError::Engine)?;
        let mut tick=tokio::time::interval(Duration::from_millis(100));
        loop {
            tokio::select! {
                biased;
                _=&mut *cancelled=>return Ok(false),
                _=tick.tick()=>{},
                command=commands.changed()=>{if command.is_ok() {
                    let paused=*commands.borrow_and_update();
                    if paused {session.pause(&handle).await.map_err(|_|TorrentError::Engine)?;} else {session.unpause(&handle).await.map_err(|_|TorrentError::Engine)?;}
                }},
            }
            let stats=handle.stats();
            let phase=match stats.state {
                librqbit::TorrentStatsState::Initializing{..}=>TorrentPhase::Checking,
                librqbit::TorrentStatsState::Live=>TorrentPhase::Downloading,
                librqbit::TorrentStatsState::Paused=>TorrentPhase::Paused,
                librqbit::TorrentStatsState::Error=>return Err(TorrentError::Engine),
            };
            // Initializing progress measures bytes scanned, not verified data.
            let downloaded=if phase==TorrentPhase::Checking {0} else {stats.progress_bytes.min(source.total)};
            let progress=DownloadProgress {downloaded_bytes:downloaded,total_bytes:Some(source.total)};
            telemetry.send_replace(TorrentTelemetry {downloaded_bytes:downloaded,total_bytes:source.total,peers:stats.live.as_ref().map(|s|s.snapshot.peer_stats.live).unwrap_or(0),uploaded_bytes:stats.uploaded_bytes,phase:phase.clone()});
            state.send_replace(if phase==TorrentPhase::Paused {TransferState::Paused(progress)} else {TransferState::Downloading(progress)});
            if stats.finished {return Ok(true);}
        }
    }.await;
    session.stop().await;
    drop(session);
    result
}

// False is cancellation, not a verification failure. Checks bound cooperative
// latency to one directory operation, 64 KiB read/hash, or filesystem sync; an
// individual blocking OS operation cannot itself be interrupted.
fn verify(
    source: &TorrentSource,
    output: &Path,
    mut cancelled: impl FnMut() -> bool,
) -> Result<bool, TorrentError> {
    let expected: BTreeSet<PathBuf> = source.files.iter().map(|f| f.path.clone()).collect();
    let mut expected_dirs = BTreeSet::new();
    for file in &expected {
        if cancelled() {
            return Ok(false);
        }
        for parent in file
            .ancestors()
            .skip(1)
            .filter(|p| !p.as_os_str().is_empty())
        {
            expected_dirs.insert(parent.to_path_buf());
        }
    }
    let mut pending = vec![PathBuf::new()];
    while let Some(relative) = pending.pop() {
        if cancelled() {
            return Ok(false);
        }
        for entry in fs::read_dir(output.join(&relative)).map_err(|_| TorrentError::Verification)? {
            if cancelled() {
                return Ok(false);
            }
            let entry = entry.map_err(|_| TorrentError::Verification)?;
            let path = relative.join(entry.file_name());
            let kind = entry.file_type().map_err(|_| TorrentError::Verification)?;
            if kind.is_dir() && expected_dirs.contains(&path) {
                pending.push(path);
            } else if !kind.is_file() || !expected.contains(&path) {
                return Err(TorrentError::Verification);
            }
        }
    }
    let mut piece = Sha1::new();
    let mut in_piece = 0usize;
    let mut index = 0usize;
    let mut buffer = vec![0u8; 64 * 1024];
    for entry in source.files.iter() {
        if cancelled() {
            return Ok(false);
        }
        let path = output.join(&entry.path);
        let meta = fs::symlink_metadata(&path).map_err(|_| TorrentError::Verification)?;
        if !meta.is_file() || meta.len() != entry.length {
            return Err(TorrentError::Verification);
        }
        let mut file = File::open(path).map_err(|_| TorrentError::Verification)?;
        let mut remaining = entry.length;
        while remaining > 0 {
            if cancelled() {
                return Ok(false);
            }
            let n = buffer
                .len()
                .min(source.piece_length - in_piece)
                .min(remaining as usize);
            file.read_exact(&mut buffer[..n])
                .map_err(|_| TorrentError::Verification)?;
            piece.update(&buffer[..n]);
            remaining -= n as u64;
            in_piece += n;
            if in_piece == source.piece_length {
                if piece.finalize_reset().as_slice() != &source.pieces[index * 20..(index + 1) * 20]
                {
                    return Err(TorrentError::Verification);
                }
                index += 1;
                in_piece = 0;
            }
        }
        if cancelled() {
            return Ok(false);
        }
        file.sync_all().map_err(|_| TorrentError::Verification)?;
    }
    if cancelled() {
        return Ok(false);
    }
    if in_piece > 0 && piece.finalize().as_slice() != &source.pieces[index * 20..(index + 1) * 20] {
        return Err(TorrentError::Verification);
    }
    Ok(true)
}
fn publish(
    payload: &Path,
    destination: &Path,
    multi: bool,
    policy: CollisionPolicy,
) -> Result<PathBuf, TorrentError> {
    for n in 0..=1000 {
        let candidate = if n == 0 {
            destination.to_path_buf()
        } else {
            let name = destination
                .file_name()
                .and_then(|s| s.to_str())
                .ok_or(TorrentError::Destination)?;
            let suffix = format!(" ({n})");
            let (stem, ext) = if !multi {
                name.rsplit_once('.')
                    .filter(|(_, ext)| ext.len() <= 20)
                    .map(|(s, e)| (s, format!(".{e}")))
                    .unwrap_or((name, String::new()))
            } else {
                (name, String::new())
            };
            let max = 180 - suffix.len() - ext.len();
            let mut end = stem.len().min(max);
            while !stem.is_char_boundary(end) {
                end -= 1;
            }
            destination.with_file_name(format!(
                "{}{suffix}{ext}",
                stem[..end].trim_end_matches([' ', '.'])
            ))
        };
        let result = if multi {
            rename_directory(payload, &candidate)
        } else {
            fs::hard_link(payload, &candidate)
        };
        match result {
            Ok(()) => return Ok(candidate),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                if policy != CollisionPolicy::Rename {
                    return Err(TorrentError::DestinationExists);
                }
            }
            Err(_) => return Err(TorrentError::Publication),
        }
    }
    Err(TorrentError::DestinationExists)
}
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn rename_directory(from: &Path, to: &Path) -> std::io::Result<()> {
    rustix::fs::renameat_with(
        rustix::fs::CWD,
        from,
        rustix::fs::CWD,
        to,
        rustix::fs::RenameFlags::NOREPLACE,
    )
    .map_err(Into::into)
}
#[cfg(windows)]
fn rename_directory(from: &Path, to: &Path) -> std::io::Result<()> {
    fs::rename(from, to)
}
#[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
fn rename_directory(_from: &Path, _to: &Path) -> std::io::Result<()> {
    Err(std::io::ErrorKind::Unsupported.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn publication_is_no_replace_even_when_target_arrives_late() {
        let dir = tempfile::tempdir().unwrap();
        for multi in [false, true] {
            let payload = dir.path().join(if multi { "tree" } else { "file" });
            let target = dir
                .path()
                .join(if multi { "target-tree" } else { "target-file" });
            if multi {
                fs::create_dir(&payload).unwrap();
                fs::create_dir(&target).unwrap();
                fs::write(target.join("original"), b"original").unwrap();
            } else {
                fs::write(&payload, b"new").unwrap();
                fs::write(&target, b"original").unwrap();
            }
            assert!(publish(&payload, &target, multi, CollisionPolicy::Reject).is_err());
            assert!(payload.exists());
            assert_eq!(
                fs::read(if multi {
                    target.join("original")
                } else {
                    target
                })
                .unwrap(),
                b"original"
            );
        }
    }
    #[test]
    fn verification_cancellation_interrupts_chunks_without_publication() {
        let dir = tempfile::tempdir().unwrap();
        // Two bounded chunks, with deliberately invalid hashes: cancellation
        // must win before reaching the end of this first 128 KiB piece.
        let data = vec![0; 128 * 1024];
        fs::write(dir.path().join("file"), &data).unwrap();
        let source = TorrentSource {
            bytes: Arc::from([]),
            name: "file".into(),
            files: Arc::from([TorrentFile {
                path: "file".into(),
                length: data.len() as u64,
            }]),
            total: data.len() as u64,
            multi: false,
            trackers: 0,
            hash: String::new(),
            piece_length: data.len(),
            pieces: Arc::from([0; 20]),
        };
        let (tx, mut rx) = oneshot::channel();
        let mut tx = Some(tx);
        let mut checks = 0;
        let verified = verify(&source, dir.path(), || {
            checks += 1;
            // Manifest, root, directory entry, file, first chunk, second chunk.
            // Send through the same channel/predicate used by the real worker.
            if checks == 6 {
                tx.take().unwrap().send(()).unwrap();
            }
            cancellation_requested(&mut rx)
        })
        .unwrap();
        assert!(!verified);
        assert_eq!(checks, 6);
        assert!(matches!(
            verify(&source, dir.path(), || false),
            Err(TorrentError::Verification)
        ));
    }

    #[test]
    fn cleanup_failure_is_redacted_and_preserves_published_receipt() {
        for result in [Ok(None), Err(TorrentError::Verification)] {
            let error =
                finish_cleanup(result, Err(std::io::Error::other("secret path"))).unwrap_err();
            assert!(matches!(error, TorrentError::Cleanup));
            assert_eq!(error.to_string(), "Could not remove torrent staging data");
        }
        let receipt = DownloadReceipt {
            destination: PathBuf::from("published"),
            downloaded_bytes: 3,
        };
        let result = finish_cleanup(Ok(Some(receipt)), Err(std::io::Error::other("secret path")))
            .unwrap()
            .unwrap();
        assert_eq!(result.destination, PathBuf::from("published"));
        assert_eq!(result.downloaded_bytes, 3);
        assert!(matches!(finish_cleanup(Ok(None), Ok(())), Ok(None)));
        assert!(matches!(
            finish_cleanup(Err(TorrentError::Engine), Ok(())),
            Err(TorrentError::Engine)
        ));
    }

    #[test]
    fn verification_rejects_wrong_piece_and_length() {
        let dir = tempfile::tempdir().unwrap();
        let path = PathBuf::from("file");
        let source = TorrentSource {
            bytes: Arc::from([]),
            name: "file".into(),
            files: Arc::from([TorrentFile {
                path: path.clone(),
                length: 3,
            }]),
            total: 3,
            multi: false,
            trackers: 0,
            hash: String::new(),
            piece_length: 16384,
            pieces: Arc::from(Sha1::digest(b"abc").to_vec()),
        };
        fs::write(dir.path().join(&path), b"bad").unwrap();
        assert!(verify(&source, dir.path(), || false).is_err());
        fs::write(dir.path().join(&path), b"ab").unwrap();
        assert!(verify(&source, dir.path(), || false).is_err());
        fs::write(dir.path().join(&path), b"abc").unwrap();
        assert!(verify(&source, dir.path(), || false).unwrap());
    }
}
