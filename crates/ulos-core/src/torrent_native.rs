//! Private, bounded protocol-1 bridge. No engine diagnostics cross this boundary.
use super::*;
use serde_json::{Value as Json, json};
use std::{io::Write, process::Stdio};
use tokio::{
    io::{AsyncBufRead, AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin, Command},
    time::{Instant as TokioInstant, timeout},
};

const FRAME_LIMIT: usize = 16 * 1024 * 1024;
const HANDSHAKE: Duration = Duration::from_secs(10);
const STALL: Duration = Duration::from_secs(15);
const RATE_MAX: u64 = 1_073_741_824;

fn worker_path() -> Option<PathBuf> {
    #[cfg(any(debug_assertions, test))]
    if let Some(path) = std::env::var_os("ULOS_TORRENT_WORKER") {
        let path = PathBuf::from(path);
        return path.is_absolute().then_some(path);
    }
    let exe = std::env::current_exe().ok()?;
    let parent = exe.parent()?;
    let name = if cfg!(windows) {
        "ulos-torrent-worker.exe"
    } else {
        "ulos-torrent-worker"
    };
    let sibling = parent.join(name);
    if sibling.is_file() {
        return Some(sibling);
    }
    #[cfg(target_os = "macos")]
    {
        let helper = parent.parent()?.join("Helpers").join(name);
        if helper.is_file() {
            return Some(helper);
        }
    }
    None
}

/// Packaging capability only, not a promise that the engine can launch.
pub fn available() -> bool {
    static AVAILABLE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *AVAILABLE.get_or_init(|| worker_path().is_some_and(|p| p.is_file()))
}

// The buffer lives outside the future: cancelling a select branch must not lose
// a partially received JSON line. No unbounded read_line allocation is used.
async fn read_frame<R: AsyncBufRead + Unpin>(
    reader: &mut R,
    pending: &mut Vec<u8>,
) -> Result<Json, TorrentError> {
    loop {
        let bytes = reader.fill_buf().await.map_err(|_| TorrentError::Worker)?;
        if bytes.is_empty() {
            return Err(TorrentError::Worker);
        }
        let end = bytes.iter().position(|b| *b == b'\n');
        let count = end.map_or(bytes.len(), |i| i + 1);
        if pending.len().saturating_add(count) > FRAME_LIMIT {
            return Err(TorrentError::Worker);
        }
        pending.extend_from_slice(&bytes[..count]);
        reader.consume(count);
        if end.is_some() {
            let parsed = serde_json::from_slice(pending).map_err(|_| TorrentError::Worker);
            pending.clear();
            return parsed;
        }
    }
}
fn event(value: &Json) -> Result<&str, TorrentError> {
    value
        .as_object()
        .and_then(|v| v.get("event"))
        .and_then(Json::as_str)
        .ok_or(TorrentError::Worker)
}
fn ready(value: &Json) -> Result<(), TorrentError> {
    let version = value
        .get("version")
        .and_then(Json::as_str)
        .ok_or(TorrentError::Worker)?;
    let parts: Vec<_> = version.split('.').collect();
    if event(value)? != "ready"
        || value.get("protocol").and_then(Json::as_u64) != Some(1)
        || value.get("engine").and_then(Json::as_str) != Some("libtorrent")
        || parts.len() < 3
        || parts[0] != "2"
        || parts.iter().any(|s| {
            s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) || s.parse::<u32>().is_err()
        })
    {
        return Err(TorrentError::Worker);
    }
    Ok(())
}
fn manifest(source: &TorrentSource, value: &Json) -> Result<(), TorrentError> {
    if event(value)? != "manifest"
        || value.get("name").and_then(Json::as_str) != Some(source.name.as_str())
        || value.get("private").and_then(Json::as_bool) != Some(source.is_private)
    {
        return Err(TorrentError::Worker);
    }
    for (key, expected, len) in [
        ("infohash_v1", &source.hash_v1, 40),
        ("infohash_v2", &source.hash_v2, 64),
    ] {
        match (value.get(key), expected) {
            (None, None) => {}
            (Some(actual), Some(expected))
                if actual.as_str().is_some_and(|s| {
                    s == expected
                        && s.len() == len
                        && s.bytes()
                            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                }) => {}
            _ => return Err(TorrentError::Worker),
        }
    }
    let files = value
        .get("files")
        .and_then(Json::as_array)
        .ok_or(TorrentError::Worker)?;
    if files.len() != source.files.len() {
        return Err(TorrentError::Worker);
    }
    let mut actual = BTreeMap::new();
    for file in files {
        let path = file
            .get("path")
            .and_then(Json::as_str)
            .ok_or(TorrentError::Worker)?;
        let length = file
            .get("length")
            .and_then(Json::as_u64)
            .ok_or(TorrentError::Worker)?;
        if actual.insert(path.to_owned(), length).is_some() {
            return Err(TorrentError::Worker);
        }
    }
    let mut expected = BTreeMap::new();
    for file in source.files.iter() {
        // JSON worker paths use '/', including on Windows. Compare components,
        // not a lossy platform display or canonicalized/sanitized path.
        let relative = file
            .path
            .iter()
            .map(|p| p.to_str().ok_or(TorrentError::Worker))
            .collect::<Result<Vec<_>, _>>()?
            .join("/");
        let path = if source.multi {
            format!("{}/{}", source.name, relative)
        } else {
            relative
        };
        expected.insert(path, file.length);
    }
    if actual != expected {
        return Err(TorrentError::Worker);
    }
    Ok(())
}
fn number(value: &Json, key: &str) -> Result<u64, TorrentError> {
    value
        .get(key)
        .and_then(Json::as_u64)
        .ok_or(TorrentError::Worker)
}
fn stats(
    source: &TorrentSource,
    value: &Json,
    initial: &TorrentSettings,
) -> Result<TorrentTelemetry, TorrentError> {
    let phase = match value.get("phase").and_then(Json::as_str) {
        Some("checking") => TorrentPhase::Checking,
        Some("downloading") => TorrentPhase::Downloading,
        Some("paused") => TorrentPhase::Paused,
        Some("publishing") => TorrentPhase::Publishing,
        _ => return Err(TorrentError::Worker),
    };
    let downloaded = number(value, "downloaded_bytes")?;
    if number(value, "total_bytes")? != source.total || downloaded > source.total {
        return Err(TorrentError::Worker);
    }
    let mut sample = TorrentTelemetry {
        downloaded_bytes: downloaded,
        total_bytes: source.total,
        phase,
        uploaded_bytes: number(value, "uploaded_bytes")?,
        download_bytes_per_second: number(value, "download_bytes_per_second")?,
        upload_bytes_per_second: number(value, "upload_bytes_per_second")?,
        peers: number(value, "peers")?
            .try_into()
            .map_err(|_| TorrentError::Worker)?,
        connecting_peers: number(value, "connecting_peers")?
            .try_into()
            .map_err(|_| TorrentError::Worker)?,
        queued_peers: number(value, "queued_peers")?
            .try_into()
            .map_err(|_| TorrentError::Worker)?,
        seen_peers: number(value, "seen_peers")?
            .try_into()
            .map_err(|_| TorrentError::Worker)?,
        stalled_for_secs: number(value, "stalled_for_secs")?,
        tracker_count: number(value, "tracker_count")?
            .try_into()
            .map_err(|_| TorrentError::Worker)?,
        dht_enabled: initial.dht && !source.is_private,
        upload_enabled: initial.allow_upload,
    };
    if sample.phase != TorrentPhase::Downloading {
        sample.download_bytes_per_second = 0;
        sample.upload_bytes_per_second = 0;
    }
    Ok(sample)
}
fn limits(settings: &TorrentSettings) -> Result<(u64, u64), TorrentError> {
    let down = settings.download_bytes_per_second.unwrap_or(0);
    let up = settings.upload_bytes_per_second.unwrap_or(0);
    if down > RATE_MAX || up > RATE_MAX || !(1..=10_000).contains(&settings.peer_limit) {
        return Err(TorrentError::Worker);
    }
    Ok((down, up))
}
async fn send(stdin: &mut ChildStdin, value: Json) -> Result<(), TorrentError> {
    let mut bytes = serde_json::to_vec(&value).map_err(|_| TorrentError::Worker)?;
    bytes.push(b'\n');
    timeout(Duration::from_secs(1), stdin.write_all(&bytes))
        .await
        .map_err(|_| TorrentError::Worker)?
        .map_err(|_| TorrentError::Worker)
}
async fn reap(child: &mut Child, duration: Duration) -> Result<(), TorrentError> {
    match timeout(duration, child.wait()).await {
        Ok(Ok(status)) if status.success() => Ok(()),
        _ => Err(TorrentError::Worker),
    }
}
async fn kill_reap(child: &mut Child) {
    let _ = child.start_kill();
    let _ = child.wait().await;
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn run(
    source: &TorrentSource,
    output: &Path,
    state: &watch::Sender<TransferState>,
    telemetry: &watch::Sender<TorrentTelemetry>,
    cancelled: &mut oneshot::Receiver<()>,
    commands: watch::Receiver<bool>,
    settings: watch::Receiver<TorrentSettings>,
) -> Result<bool, TorrentError> {
    let worker = worker_path()
        .filter(|p| p.is_file())
        .ok_or(TorrentError::NativeUnavailable)?;
    run_with_worker(
        &worker, source, output, state, telemetry, cancelled, commands, settings,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn run_with_worker(
    worker: &Path,
    source: &TorrentSource,
    output: &Path,
    state: &watch::Sender<TransferState>,
    telemetry: &watch::Sender<TorrentTelemetry>,
    cancelled: &mut oneshot::Receiver<()>,
    mut commands: watch::Receiver<bool>,
    mut settings: watch::Receiver<TorrentSettings>,
) -> Result<bool, TorrentError> {
    let initial = settings.borrow_and_update().clone();
    let (down, up) = limits(&initial)?;
    if !initial.allow_upload {
        return Err(TorrentError::UploadRequired);
    }
    let mut metainfo =
        tempfile::NamedTempFile::new_in(output.parent().ok_or(TorrentError::Destination)?)
            .map_err(|_| TorrentError::Worker)?;
    metainfo
        .write_all(&source.bytes)
        .map_err(|_| TorrentError::Worker)?;
    metainfo.flush().map_err(|_| TorrentError::Worker)?;
    let mut command = Command::new(worker);
    #[cfg(windows)]
    command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW for the bundled worker.
    let mut child = command
        .arg("--metainfo")
        .arg(metainfo.path())
        .arg("--output")
        .arg(output)
        .arg("--dht")
        .arg(if initial.dht && !source.is_private {
            "1"
        } else {
            "0"
        })
        .arg("--upload")
        .arg(if initial.allow_upload { "1" } else { "0" })
        .arg("--download-limit")
        .arg(down.to_string())
        .arg("--upload-limit")
        .arg(up.to_string())
        .arg("--peer-limit")
        .arg(initial.peer_limit.to_string())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|_| TorrentError::NativeUnavailable)?;
    let result = async {
        let mut stdin = child.stdin.take().ok_or(TorrentError::Worker)?;
        let mut stdout = BufReader::new(child.stdout.take().ok_or(TorrentError::Worker)?);
        let mut pending = Vec::new();
        // One deadline covers both initial messages. Cancellation never waits for
        // an unfinished stdout read or a worker-controlled diagnostic.
        let handshake = async {
            ready(&read_frame(&mut stdout, &mut pending).await?)?;
            manifest(source, &read_frame(&mut stdout, &mut pending).await?)
        };
        tokio::select! {
            biased;
            _ = &mut *cancelled => { let _ = send(&mut stdin, json!({"command":"cancel"})).await; let _ = reap(&mut child, Duration::from_secs(3)).await; return Ok(false); },
            result = timeout(HANDSHAKE, handshake) => { result.map_err(|_| TorrentError::Worker)??; }
        }
        if !initial.allow_upload { return Err(TorrentError::UploadRequired); }
        send(&mut stdin, json!({"command":"start"})).await?;
        if *commands.borrow_and_update() { send(&mut stdin, json!({"command":"pause"})).await?; }
        let mut command_open = true;
        let mut settings_open = true;
        let mut deadline = TokioInstant::now() + STALL;
        loop {
            tokio::select! {
                biased;
                _ = &mut *cancelled => {
                    let _ = send(&mut stdin, json!({"command":"cancel"})).await;
                    let _ = reap(&mut child, Duration::from_secs(3)).await;
                    return Ok(false);
                },
                _ = tokio::time::sleep_until(deadline) => return Err(TorrentError::Worker),
                changed = commands.changed(), if command_open => {
                    if changed.is_err() { command_open = false; continue; }
                    let paused = *commands.borrow_and_update();
                    send(&mut stdin, json!({"command":if paused {"pause"} else {"resume"}})).await?;
                },
                changed = settings.changed(), if settings_open => {
                    if changed.is_err() { settings_open = false; continue; }
                    let next = settings.borrow_and_update().clone();
                    let (down, up) = limits(&next)?;
                    // Discovery and upload consent remain launch-only.
                    send(&mut stdin, json!({"command":"peers","limit":next.peer_limit})).await?;
                    send(&mut stdin, json!({"command":"limits","download_bytes_per_second":down,"upload_bytes_per_second":up})).await?;
                },
                frame = read_frame(&mut stdout, &mut pending) => {
                    let frame = frame?;
                    deadline = TokioInstant::now() + STALL;
                    match event(&frame)? {
                        "stats" => {
                            let sample = stats(source, &frame, &initial)?;
                            let progress = DownloadProgress { downloaded_bytes:sample.downloaded_bytes, total_bytes:Some(source.total) };
                            state.send_replace(if sample.phase == TorrentPhase::Paused { TransferState::Paused(progress) } else { TransferState::Downloading(progress) });
                            telemetry.send_replace(sample);
                        },
                        "complete" => { reap(&mut child, Duration::from_secs(10)).await?; return Ok(true); },
                        "cancelled" => { reap(&mut child, Duration::from_secs(3)).await?; return Ok(false); },
                        "error" => return Err(TorrentError::Engine),
                        _ => return Err(TorrentError::Worker),
                    }
                }
            }
        }
    }.await;
    // Includes malformed output, EOF, cancellation timeout, and nonzero exits.
    // Always reap before returning control to verification/publication/cleanup.
    kill_reap(&mut child).await;
    drop(metainfo);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source() -> TorrentSource {
        let mut bytes = b"d4:infod6:lengthi1e4:name1:a12:piece lengthi16384e6:pieces20:".to_vec();
        bytes.extend_from_slice(&[0; 20]);
        bytes.extend_from_slice(b"ee");
        super::super::parse(bytes).unwrap()
    }
    fn inventory(source: &TorrentSource) -> Json {
        let mut value = json!({"event":"manifest","name":source.name,"private":source.is_private,"files":[{"path":"a","length":1}]});
        if let Some(hash) = &source.hash_v1 {
            value["infohash_v1"] = json!(hash);
        }
        if let Some(hash) = &source.hash_v2 {
            value["infohash_v2"] = json!(hash);
        }
        value
    }
    #[test]
    fn handshake_is_versioned() {
        assert!(
            ready(&json!({"event":"ready","protocol":1,"engine":"libtorrent","version":"2.0.15"}))
                .is_ok()
        );
        for version in ["1.2.3", "3.0.0", "2", "2.evil.0", "2.0.-1"] {
            assert!(
                ready(
                    &json!({"event":"ready","protocol":1,"engine":"libtorrent","version":version})
                )
                .is_err()
            );
        }
    }
    #[test]
    fn manifest_exact_paths_hashes_and_privacy() {
        let source = source();
        let good = inventory(&source);
        assert!(manifest(&source, &good).is_ok());
        for path in ["../a", "/a", "a/../a", "a\\a", "different", "a/", "./a"] {
            let mut bad = good.clone();
            bad["files"][0]["path"] = json!(path);
            assert!(manifest(&source, &bad).is_err());
        }
        let mut bad = good.clone();
        bad["files"] = json!([]);
        assert!(manifest(&source, &bad).is_err());
        let mut bad = good.clone();
        bad["files"] = json!([{"path":"a","length":1},{"path":"a","length":1}]);
        assert!(manifest(&source, &bad).is_err());
        let mut bad = good.clone();
        bad["private"] = json!(!source.is_private);
        assert!(manifest(&source, &bad).is_err());
        let mut bad = good.clone();
        bad.as_object_mut().unwrap().remove("infohash_v1");
        assert!(manifest(&source, &bad).is_err());
        let mut bad = good;
        bad["infohash_v2"] = json!("0".repeat(64));
        assert!(manifest(&source, &bad).is_err());
    }
    #[test]
    fn v2_and_hybrid_require_every_hash_and_multifile_root() {
        let mut source = source();
        source.hash_v1 = None;
        source.hash_v2 = Some("a".repeat(64));
        let good = inventory(&source);
        assert!(manifest(&source, &good).is_ok());
        let mut bad = good.clone();
        bad.as_object_mut().unwrap().remove("infohash_v2");
        assert!(manifest(&source, &bad).is_err());
        source.hash_v1 = Some("b".repeat(40));
        assert!(manifest(&source, &good).is_err());
        let mut good = inventory(&source);
        source.multi = true;
        assert!(manifest(&source, &good).is_err());
        good["files"][0]["path"] = json!("a/a");
        assert!(manifest(&source, &good).is_ok());
    }

    #[tokio::test]
    async fn frames_are_bounded_and_partial_eof_is_rejected() {
        let mut reader = BufReader::new(&b"{\"event\":\"complete\"}\n{\"event\":"[..]);
        let mut pending = Vec::new();
        assert_eq!(
            event(&read_frame(&mut reader, &mut pending).await.unwrap()).unwrap(),
            "complete"
        );
        assert!(read_frame(&mut reader, &mut pending).await.is_err());
        let bytes = vec![b'x'; FRAME_LIMIT + 1];
        let mut reader = BufReader::new(bytes.as_slice());
        pending.clear();
        assert!(read_frame(&mut reader, &mut pending).await.is_err());
        assert!(pending.len() <= FRAME_LIMIT);
    }
    #[test]
    fn stats_reject_invalid_numbers_and_reset_non_download_rates() {
        let source = source();
        let mut frame = json!({"phase":"paused","downloaded_bytes":1,"total_bytes":1,"uploaded_bytes":2,"download_bytes_per_second":99,"upload_bytes_per_second":99,"peers":0,"connecting_peers":0,"queued_peers":0,"seen_peers":0,"stalled_for_secs":0,"tracker_count":0});
        let sample = stats(&source, &frame, &TorrentSettings::default()).unwrap();
        assert_eq!(sample.download_bytes_per_second, 0);
        assert_eq!(sample.upload_bytes_per_second, 0);
        for (key, bad) in [
            ("peers", json!(-1)),
            ("total_bytes", json!(2)),
            ("downloaded_bytes", json!(2)),
            ("seen_peers", json!(u64::MAX)),
        ] {
            let mut value = frame.clone();
            value[key] = bad;
            assert!(stats(&source, &value, &TorrentSettings::default()).is_err());
        }
        frame["phase"] = json!("invented");
        assert!(stats(&source, &frame, &TorrentSettings::default()).is_err());
    }
    #[cfg(unix)]
    async fn fake(body: &str, invalid: bool) -> (Result<bool, TorrentError>, bool) {
        use std::os::unix::fs::PermissionsExt;
        let temp = tempfile::tempdir().unwrap();
        let source = source();
        let mut manifest = inventory(&source);
        if invalid {
            manifest["files"][0]["path"] = json!("../evil");
        }
        let worker = temp.path().join("worker");
        let marker = temp.path().join("started");
        std::fs::write(&worker, format!("#!/bin/sh\nprintf '%s\\n' '{{\"event\":\"ready\",\"protocol\":1,\"engine\":\"libtorrent\",\"version\":\"2.0.15\"}}' '{}'\nIFS= read -r command || exit 0\nprintf '%s' \"$command\" > '{}'\n{}\n", manifest, marker.display(), body)).unwrap();
        std::fs::set_permissions(&worker, std::fs::Permissions::from_mode(0o700)).unwrap();
        let output = temp.path().join("output");
        std::fs::create_dir(&output).unwrap();
        let (state, _) = watch::channel(TransferState::Preparing);
        let (telemetry, _) = watch::channel(TorrentTelemetry::default());
        let (_cancel, mut cancelled) = oneshot::channel();
        let (_commands, commands) = watch::channel(false);
        let (_settings, settings) = watch::channel(TorrentSettings {
            allow_upload: true,
            ..TorrentSettings::default()
        });
        let result = run_with_worker(
            &worker,
            &source,
            &output,
            &state,
            &telemetry,
            &mut cancelled,
            commands,
            settings,
        )
        .await;
        let started = marker.exists();
        (result, started)
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn child_barrier_and_success_exit_are_required() {
        let (result, started) = fake("printf '%s\\n' '{\"event\":\"complete\"}'", false).await;
        assert!(matches!(result, Ok(true)));
        assert!(started);
        let (result, started) = fake("exit 0", true).await;
        assert!(matches!(result, Err(TorrentError::Worker)));
        assert!(!started);
        let (result, _) = fake("printf '%s\\n' '{\"event\":\"complete\"}'; exit 1", false).await;
        assert!(matches!(result, Err(TorrentError::Worker)));
        let (result, _) = fake("exit 0", false).await;
        assert!(matches!(result, Err(TorrentError::Worker)));
    }
}
