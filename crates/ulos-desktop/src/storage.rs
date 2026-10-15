//! Privacy-preserving history. Only destination and progress are retained; no source,
//! credentials, cookies, validators, or retry intent are part of this schema.
//! Completed is a historical event, not a guarantee that the file still exists.
use rusqlite::{Connection, params};
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::{runtime::Handle, sync::watch};

pub const MAX_HISTORY_RECORDS: usize = 500;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HistoryStatus {
    Added,
    Connecting,
    Downloading,
    Paused,
    Completed,
    Failed,
    Cancelled,
    Interrupted,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HistoryRecord {
    pub id: u64,
    pub destination: PathBuf,
    pub added_at: SystemTime,
    pub status: HistoryStatus,
    pub downloaded: u64,
    pub total: Option<u64>,
    pub is_torrent: bool,
    pub output_directory: bool,
}

impl HistoryStatus {
    fn encoded(self) -> &'static str {
        match self {
            Self::Added => "added",
            Self::Connecting => "connecting",
            Self::Downloading => "downloading",
            Self::Paused => "paused",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Interrupted => "interrupted",
        }
    }
    fn restored(value: &str) -> Result<Self, String> {
        match value {
            "added" | "connecting" | "downloading" | "paused" | "interrupted" => {
                Ok(Self::Interrupted)
            }
            "completed" => Ok(Self::Completed),
            "failed" => Ok(Self::Failed),
            "cancelled" => Ok(Self::Cancelled),
            _ => Err("History contains an invalid status.".into()),
        }
    }
}

// Native path blobs are deliberately OS-specific, not a portable interchange format.
#[cfg(unix)]
fn encode_path(path: &Path) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt;
    path.as_os_str().as_bytes().to_vec()
}
#[cfg(unix)]
fn decode_path(bytes: Vec<u8>) -> Result<PathBuf, String> {
    use std::os::unix::ffi::OsStringExt;
    Ok(std::ffi::OsString::from_vec(bytes).into())
}
#[cfg(windows)]
fn encode_path(path: &Path) -> Vec<u8> {
    use std::os::windows::ffi::OsStrExt;
    path.as_os_str()
        .encode_wide()
        .flat_map(u16::to_le_bytes)
        .collect()
}
#[cfg(windows)]
fn decode_path(bytes: Vec<u8>) -> Result<PathBuf, String> {
    use std::os::windows::ffi::OsStringExt;
    if bytes.len() % 2 != 0 {
        return Err("History contains an invalid native path.".into());
    }
    let wide: Vec<_> = bytes
        .chunks_exact(2)
        .map(|b| u16::from_le_bytes([b[0], b[1]]))
        .collect();
    Ok(std::ffi::OsString::from_wide(&wide).into())
}

fn sql_error(_: rusqlite::Error) -> String {
    "Unable to read or write the history database.".into()
}
fn parse_number(value: &str) -> Result<u64, String> {
    value
        .parse()
        .map_err(|_| "History contains an invalid number.".into())
}

/// Synchronous SQLite access: construct and use this only on a blocking worker.
pub struct HistoryStore {
    connection: Connection,
}
impl HistoryStore {
    pub fn open(path: &Path) -> Result<Self, String> {
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            let mut builder = std::fs::DirBuilder::new();
            builder.recursive(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                builder.mode(0o700);
            }
            builder
                .create(parent)
                .map_err(|_| "Unable to create the history directory.".to_string())?;
        }
        // Pre-create privately, before SQLite can place any records in the file.
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        match options.open(path) {
            Ok(_) => (),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                let metadata = std::fs::symlink_metadata(path)
                    .map_err(|_| "Unable to inspect the history database.".to_string())?;
                if !metadata.is_file() || metadata.file_type().is_symlink() {
                    return Err(
                        "History database must be a regular file, not a symbolic link.".into(),
                    );
                }
            }
            Err(_) => return Err("Unable to open the history database.".into()),
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
                .map_err(|_| "Unable to protect the history database.".to_string())?;
        }
        let mut connection = Connection::open(path).map_err(sql_error)?;
        connection
            .busy_timeout(Duration::from_secs(2))
            .map_err(sql_error)?;
        let version: u32 = connection
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .map_err(sql_error)?;
        match version {
            0 => {
                let tx = connection.transaction().map_err(sql_error)?;
                tx.execute_batch("CREATE TABLE history (id TEXT PRIMARY KEY NOT NULL, path BLOB NOT NULL, added_secs TEXT NOT NULL, added_nanos INTEGER NOT NULL, status TEXT NOT NULL, downloaded TEXT NOT NULL, total TEXT, is_torrent INTEGER NOT NULL DEFAULT 0 CHECK(is_torrent IN (0,1)), output_directory INTEGER NOT NULL DEFAULT 0 CHECK(output_directory IN (0,1))); PRAGMA user_version=2;").map_err(sql_error)?;
                tx.commit().map_err(sql_error)?;
            }
            1 => {
                let tx = connection.transaction().map_err(sql_error)?;
                tx.execute_batch("ALTER TABLE history ADD COLUMN is_torrent INTEGER NOT NULL DEFAULT 0 CHECK(is_torrent IN (0,1)); ALTER TABLE history ADD COLUMN output_directory INTEGER NOT NULL DEFAULT 0 CHECK(output_directory IN (0,1)); PRAGMA user_version=2;").map_err(sql_error)?;
                tx.commit().map_err(sql_error)?;
            }
            2 => (),
            _ => return Err("History was created by an unsupported application version.".into()),
        }
        Ok(Self { connection })
    }
    pub fn replace(&mut self, records: &[HistoryRecord]) -> Result<(), String> {
        if records.len() > MAX_HISTORY_RECORDS {
            return Err("History is limited to 500 downloads.".into());
        }
        let mut ids = std::collections::HashSet::new();
        let mut prepared = Vec::with_capacity(records.len());
        for record in records {
            if !ids.insert(record.id) {
                return Err("History contains duplicate download IDs.".into());
            }
            let elapsed = record
                .added_at
                .duration_since(UNIX_EPOCH)
                .map_err(|_| "History contains an invalid timestamp.".to_string())?;
            if record.destination.as_os_str().is_empty() {
                return Err("History contains an invalid destination.".into());
            }
            // Resolve relative paths while the original session's working directory is known.
            let path = if record.destination.is_absolute() {
                record.destination.clone()
            } else {
                std::env::current_dir()
                    .map_err(|_| "Unable to resolve the download destination.".to_string())?
                    .join(&record.destination)
            };
            if path.as_os_str().is_empty() {
                return Err("History contains an invalid destination.".into());
            }
            prepared.push((record, elapsed, encode_path(&path)));
        }
        let tx = self.connection.transaction().map_err(sql_error)?;
        tx.execute("DELETE FROM history", []).map_err(sql_error)?;
        for (record, elapsed, path) in prepared {
            tx.execute(
                "INSERT INTO history (id,path,added_secs,added_nanos,status,downloaded,total,is_torrent,output_directory) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    format!("{:020}", record.id),
                    path,
                    elapsed.as_secs().to_string(),
                    elapsed.subsec_nanos(),
                    record.status.encoded(),
                    record.downloaded.to_string(),
                    record.total.map(|v| v.to_string()),
                    record.is_torrent,
                    record.output_directory
                ],
            )
            .map_err(sql_error)?;
        }
        tx.commit().map_err(sql_error)
    }
    pub fn load(&self) -> Result<Vec<HistoryRecord>, String> {
        let mut statement = self.connection.prepare("SELECT id,path,added_secs,added_nanos,status,downloaded,total,is_torrent,output_directory FROM history ORDER BY id LIMIT 501").map_err(sql_error)?;
        let mut rows = statement.query([]).map_err(sql_error)?;
        let mut records = Vec::new();
        while let Some(row) = rows.next().map_err(sql_error)? {
            if records.len() == MAX_HISTORY_RECORDS {
                return Err("History exceeds the 500 download limit.".into());
            }
            let id: String = row.get(0).map_err(sql_error)?;
            let path: Vec<u8> = row.get(1).map_err(sql_error)?;
            let secs: String = row.get(2).map_err(sql_error)?;
            let nanos: i64 = row.get(3).map_err(sql_error)?;
            let status: String = row.get(4).map_err(sql_error)?;
            let downloaded: String = row.get(5).map_err(sql_error)?;
            let total: Option<String> = row.get(6).map_err(sql_error)?;
            let is_torrent: i64 = row.get(7).map_err(sql_error)?;
            let output_directory: i64 = row.get(8).map_err(sql_error)?;
            if ![0, 1].contains(&is_torrent) || ![0, 1].contains(&output_directory) {
                return Err("History contains invalid output flags.".into());
            }
            if !(0..1_000_000_000).contains(&nanos) {
                return Err("History contains an invalid timestamp.".into());
            }
            let added_at = UNIX_EPOCH
                .checked_add(Duration::new(parse_number(&secs)?, nanos as u32))
                .ok_or_else(|| "History contains an invalid timestamp.".to_string())?;
            records.push(HistoryRecord {
                id: parse_number(&id)?,
                destination: decode_path(path)?,
                added_at,
                status: HistoryStatus::restored(&status)?,
                downloaded: parse_number(&downloaded)?,
                total: total.as_deref().map(parse_number).transpose()?,
                is_torrent: is_torrent != 0,
                output_directory: output_directory != 0,
            });
        }
        Ok(records)
    }
}

pub async fn load_history(path: PathBuf) -> Result<Vec<HistoryRecord>, String> {
    tokio::task::spawn_blocking(move || HistoryStore::open(&path)?.load())
        .await
        .map_err(|_| "History worker stopped unexpectedly.".to_string())?
}

/// Latest-snapshot, bounded, serial writer. Construction never writes an empty list.
/// Call `flush()` and await it before shutting down the runtime to commit pending saves.
#[derive(Clone)]
pub struct HistoryWriter {
    snapshots: watch::Sender<Option<(u64, Vec<HistoryRecord>)>>,
    acknowledgements: watch::Receiver<(u64, Result<(), String>)>,
    revision: Arc<AtomicU64>,
}
impl HistoryWriter {
    pub fn new(runtime: &Handle, path: PathBuf) -> Self {
        let (snapshots, mut receiver) = watch::channel::<Option<(u64, Vec<HistoryRecord>)>>(None);
        let (acknowledge, acknowledgements) = watch::channel((0, Ok(())));
        runtime.spawn(async move {
            while receiver.changed().await.is_ok() {
                let pending = receiver.borrow_and_update().clone();
                if let Some((revision, records)) = pending {
                    let path = path.clone();
                    let result = tokio::task::spawn_blocking(move || {
                        HistoryStore::open(&path)?.replace(&records)
                    })
                    .await
                    .unwrap_or_else(|_| Err("History worker stopped unexpectedly.".into()));
                    let _ = acknowledge.send_replace((revision, result));
                }
            }
        });
        Self {
            snapshots,
            acknowledgements,
            revision: Arc::new(AtomicU64::new(0)),
        }
    }
    pub fn save(&self, records: Vec<HistoryRecord>) -> u64 {
        // Serialize clone callers with the watch sender's modify lock so revisions
        // cannot be published out of order even when multiple threads save.
        let mut revision = 0;
        self.snapshots.send_modify(|snapshot| {
            revision = self.revision.fetch_add(1, Ordering::SeqCst) + 1;
            *snapshot = Some((revision, records));
        });
        revision
    }
    pub fn subscribe(&self) -> watch::Receiver<(u64, Result<(), String>)> {
        self.acknowledgements.clone()
    }
    pub fn flush(&self) -> impl std::future::Future<Output = Result<(), String>> + Send + 'static {
        let target = self.revision.load(Ordering::SeqCst);
        let mut receiver = self.acknowledgements.clone();
        async move {
            loop {
                let (revision, result) = receiver.borrow_and_update().clone();
                if revision >= target {
                    return result;
                }
                receiver
                    .changed()
                    .await
                    .map_err(|_| "History writer stopped before saving.".to_string())?;
            }
        }
    }
}

/// Explicit user-requested deletion only. Never recursively deletes a directory.
/// Missing files are idempotent success; symlinks (including ancestors) are refused.
pub async fn delete_downloaded_file(path: PathBuf) -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        let path = if path.is_absolute() {
            path
        } else {
            std::env::current_dir()
                .map_err(|_| "Unable to resolve the download destination.".to_string())?
                .join(path)
        };
        for ancestor in path.ancestors() {
            match std::fs::symlink_metadata(ancestor) {
                Ok(metadata) if metadata.file_type().is_symlink() => {
                    return Err("Refusing to delete through a symbolic link.".into());
                }
                Ok(_) => (),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
                Err(_) => return Err("Unable to inspect the download destination.".into()),
            }
        }
        let metadata = std::fs::symlink_metadata(&path)
            .map_err(|_| "Unable to inspect the download destination.".to_string())?;
        if !metadata.is_file() {
            return Err("Only regular downloaded files can be deleted.".into());
        }
        match std::fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(_) => Err("Unable to delete the downloaded file.".into()),
        }
    })
    .await
    .map_err(|_| "File deletion worker stopped unexpectedly.".to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;
    fn record(path: PathBuf) -> HistoryRecord {
        HistoryRecord {
            id: u64::MAX,
            destination: path,
            added_at: UNIX_EPOCH + Duration::new(123, 456),
            status: HistoryStatus::Completed,
            downloaded: u64::MAX,
            total: Some(u64::MAX),
            is_torrent: false,
            output_directory: false,
        }
    }
    #[test]
    fn durable_full_width_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested/history.sqlite");
        let original = record(dir.path().join("finished.bin"));
        HistoryStore::open(&path)
            .unwrap()
            .replace(std::slice::from_ref(&original))
            .unwrap();
        assert_eq!(
            HistoryStore::open(&path).unwrap().load().unwrap(),
            vec![original]
        );
    }
    #[test]
    fn version_one_migration_preserves_rows_and_defaults_flags() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("v1.sqlite");
        let connection = Connection::open(&path).unwrap();
        connection.execute_batch("CREATE TABLE history (id TEXT PRIMARY KEY NOT NULL, path BLOB NOT NULL, added_secs TEXT NOT NULL, added_nanos INTEGER NOT NULL, status TEXT NOT NULL, downloaded TEXT NOT NULL, total TEXT); PRAGMA user_version=1;").unwrap();
        let original = record(dir.path().join("old.bin"));
        connection
            .execute(
                "INSERT INTO history VALUES (?1,?2,'123',456,'completed',?3,?3)",
                params![
                    format!("{:020}", original.id),
                    encode_path(&original.destination),
                    u64::MAX.to_string()
                ],
            )
            .unwrap();
        drop(connection);
        let mut store = HistoryStore::open(&path).unwrap();
        assert_eq!(store.load().unwrap(), vec![original.clone()]);
        assert_eq!(
            store
                .connection
                .pragma_query_value(None, "user_version", |row| row.get::<_, u32>(0))
                .unwrap(),
            2
        );
        let mut torrent = original;
        torrent.is_torrent = true;
        torrent.output_directory = true;
        torrent.destination = dir.path().join("absent-directory");
        store.replace(std::slice::from_ref(&torrent)).unwrap();
        assert_eq!(store.load().unwrap(), vec![torrent]);
        for column in ["is_torrent", "output_directory"] {
            assert!(
                store
                    .connection
                    .execute(&format!("UPDATE history SET {column}=2"), [])
                    .is_err()
            );
        }
    }

    #[test]
    fn failed_migration_rolls_back_columns_and_version_without_losing_rows() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("invalid-v1.sqlite");
        let connection = Connection::open(&path).unwrap();
        connection.execute_batch("CREATE TABLE history (id TEXT, output_directory INTEGER); INSERT INTO history VALUES ('preserved',0); PRAGMA user_version=1;").unwrap();
        drop(connection);
        assert!(HistoryStore::open(&path).is_err());
        let connection = Connection::open(&path).unwrap();
        assert_eq!(
            connection
                .pragma_query_value(None, "user_version", |row| row.get::<_, u32>(0))
                .unwrap(),
            1
        );
        assert_eq!(
            connection
                .query_row("SELECT id FROM history", [], |row| row.get::<_, String>(0))
                .unwrap(),
            "preserved"
        );
        assert!(
            connection
                .prepare("SELECT is_torrent FROM history")
                .is_err()
        );
    }

    #[test]
    fn duplicate_and_invalid_time_preserve_old_records() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = HistoryStore::open(&dir.path().join("history.sqlite")).unwrap();
        let original = record(dir.path().join("finished.bin"));
        store.replace(std::slice::from_ref(&original)).unwrap();
        assert!(
            store
                .replace(&[original.clone(), original.clone()])
                .is_err()
        );
        let mut invalid = original.clone();
        invalid.added_at = UNIX_EPOCH - Duration::from_secs(1);
        assert!(store.replace(&[invalid]).is_err());
        assert_eq!(store.load().unwrap(), vec![original]);
    }
    #[test]
    fn active_records_never_resume() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = HistoryStore::open(&dir.path().join("history.sqlite")).unwrap();
        for status in [
            HistoryStatus::Added,
            HistoryStatus::Connecting,
            HistoryStatus::Downloading,
            HistoryStatus::Paused,
        ] {
            let mut r = record(dir.path().join("file"));
            r.status = status;
            store.replace(&[r]).unwrap();
            assert_eq!(store.load().unwrap()[0].status, HistoryStatus::Interrupted);
        }
    }
    #[test]
    fn unsupported_version_and_corruption_are_not_recreated() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("history.sqlite");
        let connection = Connection::open(&path).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE preserved (value TEXT); INSERT INTO preserved VALUES ('old rows');",
            )
            .unwrap();
        connection.pragma_update(None, "user_version", 3).unwrap();
        drop(connection);
        let before = std::fs::read(&path).unwrap();
        assert!(HistoryStore::open(&path).is_err());
        assert_eq!(before, std::fs::read(&path).unwrap());
        let corrupt = dir.path().join("corrupt.sqlite");
        std::fs::write(&corrupt, b"not sqlite").unwrap();
        assert!(HistoryStore::open(&corrupt).is_err());
        assert_eq!(std::fs::read(corrupt).unwrap(), b"not sqlite");
    }
    #[cfg(unix)]
    #[test]
    fn native_non_utf8_path_roundtrip_and_private_permissions() {
        use std::os::unix::{ffi::OsStringExt, fs::PermissionsExt};
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("private/history.sqlite");
        let original = record(
            dir.path()
                .join(std::ffi::OsString::from_vec(vec![b'f', 0xff])),
        );
        let mut store = HistoryStore::open(&path).unwrap();
        store.replace(std::slice::from_ref(&original)).unwrap();
        assert_eq!(store.load().unwrap(), vec![original]);
        assert_eq!(
            std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    #[test]
    fn schema_is_metadata_only_and_over_limit_preserves_rows() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = HistoryStore::open(&dir.path().join("history.sqlite")).unwrap();
        let original = record(dir.path().join("file"));
        store.replace(std::slice::from_ref(&original)).unwrap();
        assert!(store.replace(&vec![original.clone(); 501]).is_err());
        assert_eq!(store.load().unwrap(), vec![original]);
        let sql: String = store
            .connection
            .query_row(
                "SELECT sql FROM sqlite_master WHERE name='history'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        for forbidden in [
            "source",
            "url",
            "cookie",
            "authorization",
            "validator",
            "tracker",
            "hash",
            "metainfo",
        ] {
            assert!(!sql.contains(forbidden));
        }
    }
    #[tokio::test]
    async fn writer_does_not_clear_on_creation_and_flushes_latest() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("history.sqlite");
        let original = record(dir.path().join("file"));
        HistoryStore::open(&path)
            .unwrap()
            .replace(std::slice::from_ref(&original))
            .unwrap();
        let writer = HistoryWriter::new(&Handle::current(), path.clone());
        writer.flush().await.unwrap();
        assert_eq!(
            load_history(path.clone()).await.unwrap(),
            vec![original.clone()]
        );
        for id in 0..20 {
            let mut r = original.clone();
            r.id = id;
            writer.save(vec![r]);
        }
        writer.flush().await.unwrap();
        assert_eq!(load_history(path).await.unwrap()[0].id, 19);
    }
    #[tokio::test]
    async fn deletion_refuses_directories_and_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        assert!(delete_downloaded_file(dir.path().to_owned()).await.is_err());
        // macOS's temporary directory can itself be reached through /var -> /private/var.
        let path = dir.path().canonicalize().unwrap().join("file");
        std::fs::write(&path, b"data").unwrap();
        delete_downloaded_file(path.clone()).await.unwrap();
        delete_downloaded_file(path).await.unwrap();
    }
}
