//! Explicit, bounded metadata discovery. Validation never starts a network task.
//!
//! Dropping the resolving future signals an owned task, which closes stdin,
//! kills/reaps its child, then removes its private staging directory. The caller
//! must keep its Tokio runtime alive until cleanup can run (runtime shutdown is
//! not a synchronous process-join guarantee). No payload transfer is queued.
use super::{MAX_METAINFO_BYTES, TorrentError, TorrentSource, parse};
use percent_encoding::{NON_ALPHANUMERIC, percent_decode_str, utf8_percent_encode};
use serde_json::Value as Json;
use std::{
    net::SocketAddr,
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tokio::{
    io::{AsyncBufRead, AsyncBufReadExt, BufReader},
    process::Command,
    sync::oneshot,
    time::timeout,
};

const MAX_MAGNET_BYTES: usize = 16 * 1024;
const FRAME_LIMIT: usize = 2 * MAX_METAINFO_BYTES + 4096;
const HANDSHAKE: Duration = Duration::from_secs(10);
const RESOLUTION: Duration = Duration::from_secs(125);
const EXIT: Duration = Duration::from_secs(5);

struct Magnet {
    v1: Option<String>,
    v2: Option<String>,
    canonical: String,
    trackers: Vec<String>,
    peers: Vec<SocketAddr>,
}

/// Validate the supported magnet subset, offline, with no filesystem access.
///
/// Supports one BEP 9 v1 and/or BEP 52 v2 identity, an advisory display name,
/// HTTP(S)/UDP trackers, and numeric explicit peers. Source URLs, web seeds,
/// keywords and all other extensions are rejected rather than implicitly fetched.
pub fn validate_magnet(value: &str) -> Result<(), TorrentError> {
    magnet(value).map(|_| ())
}

fn decode(value: &str) -> Result<String, TorrentError> {
    let bytes = value.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            if i + 2 >= bytes.len() || !bytes[i + 1..i + 3].iter().all(u8::is_ascii_hexdigit) {
                return Err(TorrentError::InvalidMetadata);
            }
            i += 3;
        } else {
            i += 1;
        }
    }
    // Magnet query values use percent escapes; '+' is not silently reinterpreted.
    let result = percent_decode_str(value)
        .decode_utf8()
        .map_err(|_| TorrentError::InvalidMetadata)?;
    if result.chars().any(char::is_control) {
        return Err(TorrentError::InvalidMetadata);
    }
    Ok(result.into_owned())
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8] = b"0123456789abcdef";
    let mut result = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        result.push(char::from(DIGITS[usize::from(b >> 4)]));
        result.push(char::from(DIGITS[usize::from(b & 15)]));
    }
    result
}

fn v1_hash(value: &str) -> Result<String, TorrentError> {
    if value.len() == 40 && value.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Ok(value.to_ascii_lowercase());
    }
    if value.len() != 32 {
        return Err(TorrentError::InvalidMetadata);
    }
    let mut bytes = Vec::with_capacity(20);
    let mut acc = 0u32;
    let mut bits = 0;
    for byte in value.bytes() {
        let digit = match byte.to_ascii_uppercase() {
            b'A'..=b'Z' => byte.to_ascii_uppercase() - b'A',
            b'2'..=b'7' => byte - b'2' + 26,
            _ => return Err(TorrentError::InvalidMetadata),
        };
        acc = (acc << 5) | u32::from(digit);
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            bytes.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    if bits != 0 || bytes.len() != 20 {
        return Err(TorrentError::InvalidMetadata);
    }
    Ok(hex(&bytes))
}

fn tracker(value: &str) -> Result<(), TorrentError> {
    if value.len() > 4096 {
        return Err(TorrentError::Limit);
    }
    // Check source syntax as well as the URL parser's normalized representation:
    // url may erase empty userinfo or normalize backslashes/ASCII whitespace.
    let authority = value
        .split_once("://")
        .map(|(_, rest)| rest.split(['/', '?', '#']).next().unwrap_or(""));
    if value.bytes().any(|b| b <= b' ' || b == 127 || b == b'\\')
        || authority.is_none_or(|a| a.contains('@') || a.contains('%'))
    {
        return Err(TorrentError::UnsafeTracker);
    }
    let url = url::Url::parse(value).map_err(|_| TorrentError::UnsafeTracker)?;
    if !matches!(url.scheme(), "http" | "https" | "udp")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || url.host_str().is_none()
        || url.port() == Some(0)
        || (url.scheme() == "udp" && url.port().is_none())
    {
        return Err(TorrentError::UnsafeTracker);
    }
    Ok(())
}

fn magnet(value: &str) -> Result<Magnet, TorrentError> {
    if value.len() > MAX_MAGNET_BYTES {
        return Err(TorrentError::Limit);
    }
    if value.bytes().any(|b| b <= b' ' || b == 127) || value.contains('#') {
        return Err(TorrentError::InvalidMetadata);
    }
    let query = value
        .strip_prefix("magnet:?")
        .ok_or(TorrentError::InvalidMetadata)?;
    let mut result = Magnet {
        v1: None,
        v2: None,
        canonical: String::from("magnet:?"),
        trackers: Vec::new(),
        peers: Vec::new(),
    };
    let mut display_name = false;
    let mut trackers = 0;
    let mut peers = 0;
    for (index, pair) in query.split('&').enumerate() {
        if index >= 132 {
            return Err(TorrentError::Limit);
        }
        let (key, value) = pair.split_once('=').ok_or(TorrentError::InvalidMetadata)?;
        let value = decode(value)?;
        let canonical = match key {
            "xt" => {
                if let Some(hash) = value.strip_prefix("urn:btih:") {
                    let hash = v1_hash(hash)?;
                    if result.v1.replace(hash.clone()).is_some() {
                        return Err(TorrentError::InvalidMetadata);
                    }
                    format!("urn:btih:{hash}")
                } else if let Some(hash) = value.strip_prefix("urn:btmh:1220") {
                    if hash.len() != 64
                        || !hash.bytes().all(|b| b.is_ascii_hexdigit())
                        || result.v2.is_some()
                    {
                        return Err(TorrentError::InvalidMetadata);
                    }
                    let hash = hash.to_ascii_lowercase();
                    result.v2 = Some(hash.clone());
                    format!("urn:btmh:1220{hash}")
                } else {
                    return Err(TorrentError::InvalidMetadata);
                }
            }
            "dn" => {
                if display_name || super::component(value.as_bytes()).is_err() {
                    return Err(TorrentError::InvalidMetadata);
                }
                display_name = true;
                value
            }
            "tr" => {
                trackers += 1;
                if trackers > 64 {
                    return Err(TorrentError::Limit);
                }
                tracker(&value)?;
                result.trackers.push(value.clone());
                value
            }
            "x.pe" => {
                peers += 1;
                if peers > 64 {
                    return Err(TorrentError::Limit);
                }
                let peer: SocketAddr = value.parse().map_err(|_| TorrentError::InvalidMetadata)?;
                if peer.port() == 0 || peer.ip().is_unspecified() || peer.ip().is_multicast() {
                    return Err(TorrentError::InvalidMetadata);
                }
                result.peers.push(peer);
                peer.to_string()
            }
            _ => return Err(TorrentError::InvalidMetadata),
        };
        if index != 0 {
            result.canonical.push('&');
        }
        result.canonical.push_str(key);
        result.canonical.push('=');
        result
            .canonical
            .push_str(&utf8_percent_encode(&canonical, NON_ALPHANUMERIC).to_string());
    }
    if result.canonical.len() > MAX_MAGNET_BYTES {
        return Err(TorrentError::Limit);
    }
    if result.v1.is_none() && result.v2.is_none() {
        return Err(TorrentError::InvalidMetadata);
    }
    Ok(result)
}

// Match the native transfer bridge's policy: debug/test absolute override only;
// release builds search only bundled siblings/Helpers, never the shell PATH.
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

struct CancelOnDrop(Option<oneshot::Sender<()>>);
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        if let Some(sender) = self.0.take() {
            let _ = sender.send(());
        }
    }
}

/// Resolve metadata only, after offline validation and explicit user consent.
/// DHT is an opt-in discovery policy, not a property known from the magnet; a
/// torrent's private flag cannot be known until its metadata has been obtained.
pub async fn resolve_magnet(value: String, dht: bool) -> Result<TorrentSource, TorrentError> {
    let magnet = magnet(&value)?;
    let worker = worker_path()
        .filter(|p| p.is_file())
        .ok_or(TorrentError::NativeUnavailable)?;
    resolve_with_worker(worker, magnet, dht).await
}

async fn resolve_with_worker(
    worker: PathBuf,
    magnet: Magnet,
    dht: bool,
) -> Result<TorrentSource, TorrentError> {
    let (sender, receiver) = oneshot::channel();
    let mut cancel = CancelOnDrop(Some(sender));
    let task = tokio::spawn(async move { run(&worker, magnet, dht, receiver).await });
    let result = task.await.map_err(|_| TorrentError::Worker);
    // Disarm only after the child-owning task has finished its cleanup.
    cancel.0.take();
    result?
}

async fn read_frame<R: AsyncBufRead + Unpin>(reader: &mut R) -> Result<Json, TorrentError> {
    let mut pending = Vec::new();
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
            return serde_json::from_slice(&pending).map_err(|_| TorrentError::Worker);
        }
    }
}

fn ready(value: &Json) -> Result<(), TorrentError> {
    let version = value
        .get("version")
        .and_then(Json::as_str)
        .ok_or(TorrentError::Worker)?;
    let parts: Vec<_> = version.split('.').collect();
    if value.get("event").and_then(Json::as_str) != Some("ready")
        || value.get("protocol").and_then(Json::as_u64) != Some(1)
        || value.get("engine").and_then(Json::as_str) != Some("libtorrent")
        || parts.len() < 3
        || parts[0] != "2"
        || parts.iter().any(|part| {
            part.is_empty()
                || !part.bytes().all(|b| b.is_ascii_digit())
                || part.parse::<u32>().is_err()
        })
    {
        return Err(TorrentError::Worker);
    }
    Ok(())
}

fn metadata(value: &Json, magnet: &Magnet) -> Result<TorrentSource, TorrentError> {
    if value.get("event").and_then(Json::as_str) != Some("metadata") {
        return Err(TorrentError::Worker);
    }
    let encoded = value
        .get("metainfo_hex")
        .and_then(Json::as_str)
        .ok_or(TorrentError::Worker)?;
    if encoded.len() > 2 * MAX_METAINFO_BYTES {
        return Err(TorrentError::Limit);
    }
    if encoded.is_empty()
        || !encoded.len().is_multiple_of(2)
        || !encoded
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(TorrentError::Worker);
    }
    fn digit(b: u8) -> u8 {
        if b <= b'9' { b - b'0' } else { b - b'a' + 10 }
    }
    let bytes = encoded
        .as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|p| (digit(p[0]) << 4) | digit(p[1]))
        .collect();
    let mut source = parse(bytes)?;
    if magnet
        .v1
        .as_ref()
        .is_some_and(|hash| source.hash_v1.as_ref() != Some(hash))
        || magnet
            .v2
            .as_ref()
            .is_some_and(|hash| source.hash_v2.as_ref() != Some(hash))
    {
        return Err(TorrentError::Verification);
    }
    // BEP 9 carries only info, and resume-data export need not retain the
    // magnet's discovery hints. Preserve the approved trackers and numeric
    // peers in the reviewed source; neither is an upload/DHT opt-in.
    if !magnet.trackers.is_empty() {
        let mut parser = super::Parser {
            bytes: &source.bytes,
            pos: 0,
            nodes: 0,
            info: None,
        };
        let root = parser.value(0)?;
        let mut entries = root.dict()?.clone();
        let mut trackers = std::collections::BTreeSet::new();
        if let Some(value) = entries.get(b"announce".as_slice()) {
            trackers.insert(value.bytes()?.to_vec());
        }
        if let Some(tiers) = entries.get(b"announce-list".as_slice()) {
            for tier in tiers.list()? {
                for tracker in tier.list()? {
                    trackers.insert(tracker.bytes()?.to_vec());
                }
            }
        }
        trackers.extend(magnet.trackers.iter().map(|s| s.as_bytes().to_vec()));
        if trackers.len() > super::MAX_TRACKERS {
            return Err(TorrentError::Limit);
        }
        entries.remove(b"announce".as_slice());
        entries.insert(
            b"announce-list",
            super::Value::List(
                trackers
                    .iter()
                    .map(|s| super::Value::List(vec![super::Value::Bytes(s)]))
                    .collect(),
            ),
        );
        fn encode(value: &super::Value<'_>, out: &mut Vec<u8>) {
            use super::Value;
            match value {
                Value::Int(n) => out.extend_from_slice(format!("i{n}e").as_bytes()),
                Value::Bytes(b) => {
                    out.extend_from_slice(format!("{}:", b.len()).as_bytes());
                    out.extend_from_slice(b);
                }
                Value::List(values) => {
                    out.push(b'l');
                    for v in values {
                        encode(v, out);
                    }
                    out.push(b'e');
                }
                Value::Dict(values) => {
                    out.push(b'd');
                    for (k, v) in values {
                        encode(&Value::Bytes(k), out);
                        encode(v, out);
                    }
                    out.push(b'e');
                }
            }
        }
        let mut bytes = Vec::new();
        encode(&super::Value::Dict(entries), &mut bytes);
        source = parse(bytes)?;
    }
    source.initial_peers = magnet.peers.clone().into();
    Ok(source)
}

async fn run(
    worker: &Path,
    magnet: Magnet,
    dht: bool,
    mut cancel: oneshot::Receiver<()>,
) -> Result<TorrentSource, TorrentError> {
    let staging = tempfile::Builder::new()
        .prefix("ulos-magnet-")
        .tempdir()
        .map_err(|_| TorrentError::Worker)?;
    let mut command = Command::new(worker);
    #[cfg(windows)]
    command.creation_flags(0x0800_0000);
    let mut child = command
        .arg("--resolve-magnet")
        .arg(&magnet.canonical)
        .arg("--output")
        .arg(staging.path())
        .arg("--dht")
        .arg(if dht { "1" } else { "0" })
        .arg("--timeout")
        .arg("120")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|_| TorrentError::NativeUnavailable)?;
    let stdin = child.stdin.take();
    // A successfully spawned piped process always has stdout; keep even this
    // defensive error inside the cleanup path.
    let exchange = async {
        let stdout = child.stdout.take().ok_or(TorrentError::Worker)?;
        let mut reader = BufReader::new(stdout);
        let frame = timeout(HANDSHAKE, read_frame(&mut reader))
            .await
            .map_err(|_| TorrentError::Worker)??;
        ready(&frame)?;
        let frame = read_frame(&mut reader).await?;
        if frame.get("event").and_then(Json::as_str) != Some("metadata") {
            // Includes error/cancelled and unknown events: never expose diagnostics.
            return Err(TorrentError::Worker);
        }
        // Drain to EOF under the exit deadline, rejecting extra frames and
        // avoiding a malicious child blocking in stdout before process exit.
        let status = timeout(EXIT, async {
            let mut tail = [0u8; 1];
            use tokio::io::AsyncReadExt;
            if reader
                .read(&mut tail)
                .await
                .map_err(|_| TorrentError::Worker)?
                != 0
            {
                return Err(TorrentError::Worker);
            }
            child.wait().await.map_err(|_| TorrentError::Worker)
        })
        .await
        .map_err(|_| TorrentError::Worker)??;
        if !status.success() {
            return Err(TorrentError::Worker);
        }
        metadata(&frame, &magnet)
    };
    let result = tokio::select! {
        biased;
        _ = &mut cancel => Err(TorrentError::Worker),
        result = timeout(RESOLUTION, exchange) => result.unwrap_or(Err(TorrentError::Worker)),
    };
    drop(stdin); // EOF is the worker's cancellation channel.
    // Always reap before dropping the private directory, including cancellation,
    // parser rejection, handshake errors, and all deadline failures.
    let _ = child.start_kill();
    let _ = child.wait().await;
    let cleanup = staging.close();
    if cleanup.is_err() {
        return Err(TorrentError::Cleanup);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const V1: &str = "0123456789abcdef0123456789abcdef01234567";
    fn uri() -> String {
        format!("magnet:?xt=urn:btih:{V1}")
    }

    #[test]
    fn accepts_v1_base32_v2_and_hybrid() {
        validate_magnet(&uri()).unwrap();
        let decoded = magnet("magnet:?xt=urn:btih:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA").unwrap();
        assert_eq!(
            decoded.v1.as_deref(),
            Some("0000000000000000000000000000000000000000")
        );
        // Non-zero base32 tests all five-bit groups, not merely allowed syntax.
        let decoded = magnet("magnet:?xt=urn:btih:77777777777777777777777777777777").unwrap();
        assert_eq!(
            decoded.v1.as_deref(),
            Some("ffffffffffffffffffffffffffffffffffffffff")
        );
        let v2 = format!("magnet:?xt=urn:btmh:1220{}", "Ab".repeat(32));
        validate_magnet(&v2).unwrap();
        validate_magnet(&format!("{}&xt=urn:btmh:1220{}", uri(), "ab".repeat(32))).unwrap();
        validate_magnet(&format!(
            "{}&dn=Hello%20world&tr=https%3A%2F%2Ftracker.test%2Fa&x.pe=%5B%3A%3A1%5D%3A1234",
            uri()
        ))
        .unwrap();
    }

    #[test]
    fn rejects_unsupported_ambiguous_or_malformed_input() {
        for value in [
            "",
            "https://example.test/",
            "MAGNET:?xt=urn:btih:a",
            "magnet://host?xt=urn:btih:a",
            "magnet:?dn=only",
            "magnet:?xt=urn:btih:0000000000000000000000000000000!",
            "magnet:?xt=urn:btmh:13200000000000000000000000000000000000000000000000000000000000000000",
        ] {
            assert!(validate_magnet(value).is_err(), "{value}");
        }
        for suffix in [
            "#fragment",
            "&",
            "&dn=%ff",
            "&dn=%",
            "&dn=%0a",
            "&dn=..",
            "&dn=foo%2Fbar",
            "&dn=foo&dn=bar",
            "&xs=https://example.test/",
            "&as=http://example.test/",
            "&ws=https://example.test/",
            "&kt=keyword",
            "&unknown=1",
            "&x.pe=host.test:1",
            "&x.pe=127.0.0.1:0",
            "&x.pe=http://127.0.0.1:1",
            "&x.pe=0.0.0.0:1",
        ] {
            assert!(
                validate_magnet(&format!("{}{suffix}", uri())).is_err(),
                "{suffix}"
            );
        }
        assert!(validate_magnet(&format!("{}&xt=urn:btih:{V1}", uri())).is_err());
        assert!(validate_magnet(&format!("{}&xt=urn:btih:{}", uri(), "a".repeat(40))).is_err());
    }

    #[test]
    fn tracker_source_and_normalized_url_are_both_checked() {
        for tracker in [
            "http://user:pass@tracker.test/a",
            "http://@tracker.test/a",
            "http://tracker.test/a#fragment",
            "http://tracker.test/a#",
            "http://%40tracker.test/a",
            "http://tracker.test\\@other.test/a",
            "http:tracker.test/a",
            "ftp://tracker.test/a",
            "https://tracker.test:0/a",
            "udp://tracker.test/a",
            "http://tracker.test/\n",
        ] {
            assert!(super::tracker(tracker).is_err(), "{tracker}");
        }
        for tracker in [
            "http://tracker.test/a",
            "https://tracker.test/a?passkey=abc",
            "udp://tracker.test:1234/a",
        ] {
            super::tracker(tracker).unwrap();
        }
        assert!(
            validate_magnet(&format!("{}&tr=http%3A%2F%2F%40tracker.test%2Fa", uri())).is_err()
        );
    }

    #[test]
    fn bounds_queries_trackers_and_peers() {
        assert!(matches!(
            validate_magnet(&"x".repeat(MAX_MAGNET_BYTES + 1)),
            Err(TorrentError::Limit)
        ));
        assert!(validate_magnet(&format!("{}{}", uri(), "&tr=http://t.test/a".repeat(64))).is_ok());
        assert!(
            validate_magnet(&format!("{}{}", uri(), "&tr=http://t.test/a".repeat(65))).is_err()
        );
        assert!(
            validate_magnet(&format!("{}&tr=http://t.test/{}", uri(), "a".repeat(4096))).is_err()
        );
        assert!(validate_magnet(&format!("{}{}", uri(), "&x.pe=127.0.0.1:1".repeat(65))).is_err());
    }

    fn fixture() -> Vec<u8> {
        // One-byte v1 file; this function creates metadata only, never a payload.
        use sha1::{Digest, Sha1};
        let mut bytes = b"d4:infod6:lengthi1e4:name1:a12:piece lengthi16384e6:pieces20:".to_vec();
        bytes.extend_from_slice(&Sha1::digest(b"a"));
        bytes.extend_from_slice(b"ee");
        bytes
    }
    fn fixture_magnet(bytes: &[u8]) -> Magnet {
        let source = parse(bytes.to_vec()).unwrap();
        magnet(&format!("magnet:?xt=urn:btih:{}", source.hash_v1.unwrap())).unwrap()
    }

    #[test]
    fn metadata_is_strict_and_bound_to_requested_identity() {
        let bytes = fixture();
        let magnet = fixture_magnet(&bytes);
        metadata(
            &json!({"event":"metadata", "metainfo_hex":hex(&bytes)}),
            &magnet,
        )
        .unwrap();
        assert!(matches!(
            metadata(
                &json!({"event":"metadata", "metainfo_hex":hex(&bytes)}),
                &super::magnet(&uri()).unwrap()
            ),
            Err(TorrentError::Verification)
        ));
        for value in ["", "0", "GG", "AA", "0000"] {
            assert!(metadata(&json!({"event":"metadata", "metainfo_hex":value}), &magnet).is_err());
        }
        assert!(
            metadata(
                &json!({"event":"error", "metainfo_hex":hex(&bytes)}),
                &magnet
            )
            .is_err()
        );
        let mut hybrid = fixture_magnet(&bytes);
        hybrid.v2 = Some("00".repeat(32));
        assert!(matches!(
            metadata(
                &json!({"event":"metadata", "metainfo_hex":hex(&bytes)}),
                &hybrid
            ),
            Err(TorrentError::Verification)
        ));
    }

    #[test]
    fn v2_identity_is_info_hash_not_file_pieces_root() {
        use sha2::{Digest, Sha256};
        let root = Sha256::digest(b"a");
        let mut info = b"d9:file treed1:ad0:d6:lengthi1e11:pieces root32:".to_vec();
        info.extend_from_slice(&root);
        info.extend_from_slice(b"eee12:meta versioni2e4:name1:a12:piece lengthi16384ee");
        let expected = hex(&Sha256::digest(&info));
        let mut bytes = b"d4:info".to_vec();
        bytes.extend_from_slice(&info);
        bytes.extend_from_slice(b"12:piece layersdee");
        let requested = magnet(&format!("magnet:?xt=urn:btmh:1220{expected}")).unwrap();
        let frame = json!({"event":"metadata", "metainfo_hex":hex(&bytes)});
        let source = metadata(&frame, &requested).unwrap();
        assert_eq!(source.hash_v2.as_deref(), Some(expected.as_str()));
        let wrong = magnet(&format!("magnet:?xt=urn:btmh:1220{}", hex(&root))).unwrap();
        assert!(matches!(
            metadata(&frame, &wrong),
            Err(TorrentError::Verification)
        ));
    }

    #[test]
    fn ready_negotiates_exact_protocol_and_engine_family() {
        ready(&json!({"event":"ready","protocol":1,"engine":"libtorrent","version":"2.0.11"}))
            .unwrap();
        for version in [
            "1.2.3",
            "2.0",
            "2.0.1-dev",
            "2..1",
            "2.0.99999999999999999999999",
        ] {
            assert!(
                ready(
                    &json!({"event":"ready","protocol":1,"engine":"libtorrent","version":version})
                )
                .is_err()
            );
        }
        assert!(
            ready(&json!({"event":"ready","protocol":2,"engine":"libtorrent","version":"2.0.11"}))
                .is_err()
        );
    }

    #[tokio::test]
    async fn frames_reject_eof_invalid_json_and_oversize() {
        for bytes in [b"".as_slice(), b"not json\n", b"{}"] {
            assert!(read_frame(&mut BufReader::new(bytes)).await.is_err());
        }
        let bytes = vec![b'a'; FRAME_LIMIT + 1];
        assert!(
            read_frame(&mut BufReader::new(bytes.as_slice()))
                .await
                .is_err()
        );
    }

    #[cfg(unix)]
    fn worker(script: &str) -> (tempfile::TempDir, PathBuf) {
        use std::os::unix::fs::PermissionsExt;
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("worker");
        std::fs::write(&path, format!("#!/bin/sh\n{script}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        (temp, path)
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn stub_success_waits_for_clean_exit_and_rejects_trailing_events() {
        let bytes = fixture();
        let ready =
            r#"echo '{"event":"ready","protocol":1,"engine":"libtorrent","version":"2.0.11"}'"#;
        let output = format!(
            "echo '{{\"event\":\"metadata\",\"metainfo_hex\":\"{}\"}}'",
            hex(&bytes)
        );
        for (tail, succeeds) in [
            ("exit 0", true),
            ("exit 7", false),
            ("echo '{}'; exit 0", false),
        ] {
            let (_temp, path) = worker(&format!("{ready}\n{output}\n{tail}"));
            let result = resolve_with_worker(path, fixture_magnet(&bytes), false).await;
            assert_eq!(result.is_ok(), succeeds);
        }
        let (_temp, path) = worker("echo '{\"event\":\"unknown\"}'");
        assert!(
            resolve_with_worker(path, fixture_magnet(&bytes), false)
                .await
                .is_err()
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn silent_stub_hits_handshake_deadline_and_cleans_up() {
        let (temp, path) = worker("printf '%s\\n' \"$4\" > \"$0.staging\"\nwhile :; do :; done");
        let staging_file = temp.path().join("worker.staging");
        let result = timeout(
            Duration::from_secs(15),
            resolve_with_worker(path, fixture_magnet(&fixture()), false),
        )
        .await
        .unwrap();
        assert!(matches!(result, Err(TorrentError::Worker)));
        let staging = PathBuf::from(std::fs::read_to_string(staging_file).unwrap().trim());
        assert!(!staging.exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn dropped_future_kills_reaps_then_removes_staging() {
        let (temp, path) = worker(
            "printf '%s\\n' \"$$\" > \"$0.pid\"\nprintf '%s\\n' \"$4\" > \"$0.staging\"\nwhile :; do :; done",
        );
        let pid_file = temp.path().join("worker.pid");
        let staging_file = temp.path().join("worker.staging");
        let task = tokio::spawn(resolve_with_worker(path, fixture_magnet(&fixture()), false));
        timeout(Duration::from_secs(5), async {
            while !staging_file.is_file() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        let pid = std::fs::read_to_string(pid_file).unwrap();
        let staging = PathBuf::from(std::fs::read_to_string(staging_file).unwrap().trim());
        assert!(staging.is_dir());
        task.abort();
        let _ = task.await;
        timeout(Duration::from_secs(5), async {
            while staging.exists() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        // kill -0 fails only after the owned cleanup task has reaped the child.
        assert!(
            !std::process::Command::new("kill")
                .arg("-0")
                .arg(pid.trim())
                .stderr(Stdio::null())
                .status()
                .unwrap()
                .success()
        );
    }
}
