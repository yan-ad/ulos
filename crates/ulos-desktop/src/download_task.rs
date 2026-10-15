use tokio::sync::watch;
use ulos_core::{
    tasks::{HttpDownloadTask, TransferState},
    torrent::{TorrentDownloadTask, TorrentTelemetry},
    transfer::TransferTelemetry,
};

pub enum DownloadTask {
    Http(HttpDownloadTask),
    Torrent(TorrentDownloadTask),
}
impl DownloadTask {
    pub fn subscribe(&self) -> watch::Receiver<TransferState> {
        match self {
            Self::Http(task) => task.subscribe(),
            Self::Torrent(task) => task.subscribe(),
        }
    }
    pub fn telemetry(&self) -> Option<watch::Receiver<TransferTelemetry>> {
        match self {
            Self::Http(task) => task.telemetry(),
            Self::Torrent(_) => None,
        }
    }
    pub fn torrent_telemetry(&self) -> Option<watch::Receiver<TorrentTelemetry>> {
        match self {
            Self::Http(_) => None,
            Self::Torrent(task) => Some(task.telemetry()),
        }
    }
    pub fn cancel(&mut self) {
        match self {
            Self::Http(task) => task.cancel(),
            Self::Torrent(task) => task.cancel(),
        }
    }
    pub fn pause(&self) -> bool {
        match self {
            Self::Http(task) => task.pause(),
            Self::Torrent(task) => task.pause(),
        }
    }
    pub fn resume(&self) -> bool {
        match self {
            Self::Http(task) => task.resume(),
            Self::Torrent(task) => task.resume(),
        }
    }
    pub fn set_rate_limit(&self, value: Option<u64>) -> bool {
        match self {
            Self::Http(task) => task.set_rate_limit(value),
            Self::Torrent(_) => false,
        }
    }
}
