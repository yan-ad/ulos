//! Session-only observed payload/HTTP received bytes, not wire/protocol traffic.
//! Kept outside queue rows so removal, filtering and history do not change totals.
use crate::queue::{DownloadId, QueueItem};
use ulos_core::tasks::TransferState;

#[derive(Default)]
pub(super) struct SessionTraffic {
    pub downloaded: u64,
    pub uploaded: u64,
    current: Option<(u64, DownloadId)>,
    last_downloaded: u64,
    last_uploaded: u64,
}

impl SessionTraffic {
    pub fn begin(&mut self, attempt: u64, id: DownloadId) {
        self.current = Some((attempt, id));
        self.last_downloaded = 0;
        self.last_uploaded = 0;
    }

    pub fn observe(&mut self, attempt: u64, id: DownloadId, down: Option<u64>, up: u64) {
        if self.current != Some((attempt, id)) {
            return;
        }
        if let Some(down) = down {
            self.downloaded = self
                .downloaded
                .saturating_add(down.saturating_sub(self.last_downloaded));
            self.last_downloaded = self.last_downloaded.max(down);
        }
        self.uploaded = self
            .uploaded
            .saturating_add(up.saturating_sub(self.last_uploaded));
        self.last_uploaded = self.last_uploaded.max(up);
    }

    pub fn observe_item(&mut self, attempt: u64, item: &QueueItem) {
        if item.is_torrent {
            if let Some(sample) = &item.torrent_telemetry {
                self.observe(
                    attempt,
                    item.id,
                    sample.payload_downloaded_bytes,
                    sample.uploaded_bytes,
                );
            }
        } else {
            self.observe(
                attempt,
                item.id,
                Some(
                    item.downloaded_bytes
                        .max(item.telemetry.progress.downloaded_bytes),
                ),
                0,
            );
        }
    }
}

/// Unknown fresh activity remains unknown; paused/terminal/idle activity is zero.
pub(super) fn rates<'a>(items: impl Iterator<Item = &'a QueueItem>) -> (Option<f64>, Option<f64>) {
    let mut down = Some(0.0);
    let mut up = Some(0.0);
    for item in items.filter(|item| item.is_active()) {
        if matches!(item.state, Some(TransferState::Paused(_))) {
            continue;
        }
        down = down
            .zip(item.speed_bytes_per_second())
            .map(|(sum, rate)| sum + rate);
        if item.is_torrent && item.torrent_settings.allow_upload {
            up = up
                .zip(item.live_torrent())
                .map(|(sum, sample)| sum + sample.upload_bytes_per_second as f64);
        }
    }
    (down, up)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn session_traffic_manager_observation_retry_and_removal(cx: &mut gpui::TestAppContext) {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let (_root, manager, cx) = super::super::tests::mount(cx, &runtime);
        manager.update(cx, |manager, cx| {
            let id = manager
                .queue
                .add("https://example.invalid/file".into(), "/unused/file".into())
                .unwrap();
            manager.attempt = 1;
            manager.active = Some(id);
            manager.session_traffic.begin(1, id);
            let progress = |bytes| {
                TransferState::Downloading(ulos_core::DownloadProgress {
                    downloaded_bytes: bytes,
                    total_bytes: Some(100),
                })
            };
            for bytes in [10, 70, 70, 30, 80] {
                manager.apply_state(id, progress(bytes), cx);
            }
            assert_eq!(manager.session_traffic.downloaded, 80);
            manager.apply_state(id, TransferState::Failed("fixture".into()), cx);
            assert!(
                manager
                    .queue
                    .retry(id, ulos_core::transfer::CollisionPolicy::Reject)
            );
            manager.attempt = 2;
            manager.active = Some(id);
            manager.session_traffic.begin(2, id);
            manager.apply_state(id, progress(20), cx);
            manager.apply_state(id, TransferState::Cancelled, cx);
            assert_eq!(manager.session_traffic.downloaded, 100);
            assert_eq!(manager.queue.remove_records(&[id]), 1);
            assert_eq!(manager.session_traffic.downloaded, 100);
            assert_eq!(rates(manager.queue.items.iter()), (Some(0.0), Some(0.0)));

            // A coalesced HTTP completion may be the only byte-bearing observation.
            let completed = manager
                .queue
                .add(
                    "https://example.invalid/other".into(),
                    "/unused/other".into(),
                )
                .unwrap();
            manager.attempt = 3;
            manager.active = Some(completed);
            manager.session_traffic.begin(3, completed);
            manager.apply_state(
                completed,
                TransferState::Completed(ulos_core::DownloadReceipt {
                    destination: "/unused/other".into(),
                    downloaded_bytes: 55,
                }),
                cx,
            );
            assert_eq!(manager.session_traffic.downloaded, 155);
            manager.queue.items.clear();
            assert_eq!(manager.session_traffic.downloaded, 155);
        });
    }

    #[test]
    fn session_traffic_payload_only_not_verified_progress_or_terminal_size() {
        let mut queue = crate::queue::SessionQueue::default();
        let id = queue
            .add("https://example.invalid/file".into(), "/unused/file".into())
            .unwrap();
        let item = queue.item_mut(id).unwrap();
        item.is_torrent = true;
        item.downloaded_bytes = 999;
        item.torrent_telemetry = Some(ulos_core::torrent::TorrentTelemetry {
            downloaded_bytes: 999,
            payload_downloaded_bytes: None,
            uploaded_bytes: 40,
            ..Default::default()
        });
        let mut traffic = SessionTraffic::default();
        traffic.begin(1, id);
        traffic.observe_item(1, item);
        assert_eq!((traffic.downloaded, traffic.uploaded), (0, 40));
        item.torrent_telemetry
            .as_mut()
            .unwrap()
            .payload_downloaded_bytes = Some(123);
        traffic.observe_item(1, item);
        traffic.observe_item(1, item);
        assert_eq!((traffic.downloaded, traffic.uploaded), (123, 40));
    }

    #[test]
    fn session_traffic_idle_stale_and_terminal_rates() {
        let mut queue = crate::queue::SessionQueue::default();
        assert_eq!(rates(queue.items.iter()), (Some(0.0), Some(0.0)));
        let id = queue
            .add("https://example.invalid/file".into(), "/unused/file".into())
            .unwrap();
        let item = queue.item_mut(id).unwrap();
        item.is_torrent = true;
        item.torrent_settings.allow_upload = true;
        item.apply_state(TransferState::Preparing, std::time::Instant::now());
        assert_eq!(rates(queue.items.iter()), (None, None));
        let item = queue.item_mut(id).unwrap();
        item.apply_state(
            TransferState::Paused(ulos_core::DownloadProgress {
                downloaded_bytes: 0,
                total_bytes: None,
            }),
            std::time::Instant::now(),
        );
        assert_eq!(rates(queue.items.iter()), (Some(0.0), Some(0.0)));
        queue
            .item_mut(id)
            .unwrap()
            .apply_state(TransferState::Cancelled, std::time::Instant::now());
        assert_eq!(rates(queue.items.iter()), (Some(0.0), Some(0.0)));
    }

    #[test]
    fn session_traffic_coalesced_regressions_and_retry() {
        let mut traffic = SessionTraffic::default();
        traffic.begin(1, 7);
        for down in [100, 100, 50, 120] {
            traffic.observe(1, 7, Some(down), down / 2);
        }
        assert_eq!((traffic.downloaded, traffic.uploaded), (120, 60));
        traffic.begin(2, 7);
        traffic.observe(1, 7, Some(900), 900);
        traffic.observe(2, 8, Some(900), 900);
        traffic.observe(2, 7, Some(30), 10);
        assert_eq!((traffic.downloaded, traffic.uploaded), (150, 70));
    }

    #[test]
    fn session_traffic_unknown_download_and_saturation() {
        let mut traffic = SessionTraffic::default();
        traffic.begin(1, 1);
        traffic.observe(1, 1, None, 30);
        assert_eq!((traffic.downloaded, traffic.uploaded), (0, 30));
        traffic.observe(1, 1, Some(u64::MAX), u64::MAX);
        traffic.begin(2, 1);
        traffic.observe(2, 1, Some(1), 1);
        assert_eq!((traffic.downloaded, traffic.uploaded), (u64::MAX, u64::MAX));
    }
}
