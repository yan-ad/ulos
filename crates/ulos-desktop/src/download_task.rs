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
    pub fn torrent_settings(&self) -> Option<ulos_core::torrent::TorrentSettings> {
        match self {
            Self::Torrent(task) => Some(task.current_settings()),
            Self::Http(_) => None,
        }
    }
    pub fn set_torrent_limits(&self, download: Option<u64>, upload: Option<u64>) -> bool {
        match self {
            Self::Torrent(task) => task.set_limits(download, upload),
            Self::Http(_) => false,
        }
    }
    pub fn set_peer_limit(&self, peers: usize) -> bool {
        match self {
            Self::Torrent(task) => task.set_peer_limit(peers),
            Self::Http(_) => false,
        }
    }
    pub fn set_rate_limit(&self, value: Option<u64>) -> bool {
        match self {
            Self::Http(task) => task.set_rate_limit(value),
            Self::Torrent(task) => task.set_rate_limit(value),
        }
    }
}
