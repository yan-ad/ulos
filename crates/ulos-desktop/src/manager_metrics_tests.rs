use super::DownloadManager;
use crate::{
    manager::tests::{draw, mount},
    queue::Category,
};
use gpui_kit::{TestAppContext, px, size};
use std::time::Instant;
use ulos_core::{
    DownloadProgress,
    tasks::TransferState,
    torrent::{TorrentPhase, TorrentTelemetry},
};

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}

#[gpui::test]
fn torrent_table_metrics_are_observed_fresh_and_protocol_specific(cx: &mut TestAppContext) {
    let runtime = runtime();
    let directory = tempfile::tempdir().unwrap();
    let (_root, view, cx) = mount(cx, &runtime);
    let (http, torrent) = view.update(cx, |manager: &mut DownloadManager, cx| {
        let http = manager
            .queue
            .add(
                "https://example.invalid/a".into(),
                directory.path().join("a.bin"),
            )
            .unwrap();
        let torrent = manager
            .queue
            .add(
                "https://example.invalid/b".into(),
                directory.path().join("b.zip"),
            )
            .unwrap();
        let item = manager.queue.item_mut(torrent).unwrap();
        item.is_torrent = true;
        item.apply_state(
            TransferState::Downloading(DownloadProgress {
                downloaded_bytes: 50,
                total_bytes: Some(100),
            }),
            Instant::now(),
        );
        item.apply_torrent_telemetry(TorrentTelemetry {
            phase: TorrentPhase::Downloading,
            downloaded_bytes: 50,
            total_bytes: 100,
            peers: 4,
            seeds: Some(2),
            availability: Some(1.5),
            payload_downloaded_bytes: Some(80),
            uploaded_bytes: 20,
            upload_bytes_per_second: 200,
            ..Default::default()
        });
        manager.sync_table(cx);
        (http, torrent)
    });
    view.read_with(cx, |manager, cx| {
        let rows = &manager.table.read(cx).delegate().rows;
        let row = rows.iter().find(|row| row.id == http).unwrap();
        assert!(
            row.seeds.is_none()
                && row.peers.is_none()
                && row.upload_speed.is_none()
                && row.ratio.is_none()
                && row.availability.is_none()
        );
        let row = rows.iter().find(|row| row.id == torrent).unwrap();
        assert_eq!(
            (row.seeds, row.peers, row.upload_speed),
            (Some(2), Some(4), Some(200))
        );
        assert_eq!((row.ratio, row.availability), (Some(0.25), Some(1.5)));
        assert_eq!(row.category, Category::Compressed);
    });
    view.update(cx, |manager, cx| {
        let item = manager.queue.item_mut(torrent).unwrap();
        item.apply_state(
            TransferState::Paused(DownloadProgress {
                downloaded_bytes: 50,
                total_bytes: Some(100),
            }),
            Instant::now(),
        );
        manager.sync_table(cx);
    });
    view.read_with(cx, |manager, cx| {
        let row = manager
            .table
            .read(cx)
            .delegate()
            .rows
            .iter()
            .find(|row| row.id == torrent)
            .unwrap();
        assert_eq!(
            (row.seeds, row.peers, row.availability, row.upload_speed),
            (None, None, None, Some(0))
        );
        assert_eq!(row.ratio, Some(0.25));
    });
    view.update(cx, |manager, cx| {
        let item = manager.queue.item_mut(torrent).unwrap();
        item.apply_torrent_telemetry(TorrentTelemetry {
            phase: TorrentPhase::Downloading,
            total_bytes: 100,
            uploaded_bytes: 20,
            ..Default::default()
        });
        manager.sync_table(cx);
    });
    view.read_with(cx, |manager, cx| {
        let row = manager
            .table
            .read(cx)
            .delegate()
            .rows
            .iter()
            .find(|row| row.id == torrent)
            .unwrap();
        assert_eq!(
            row.ratio, None,
            "missing payload denominator is not a zero ratio"
        );
    });
}

#[gpui::test]
fn session_footer_is_always_visible_and_fits_minimum_width(cx: &mut TestAppContext) {
    let runtime = runtime();
    let (_root, view, cx) = mount(cx, &runtime);
    cx.simulate_resize(size(px(880.), px(540.)));
    for large in [false, true] {
        view.update(cx, |manager, cx| {
            if large {
                manager.session_traffic.downloaded = u64::MAX;
                manager.session_traffic.uploaded = u64::MAX;
            }
            cx.notify();
        });
        draw(cx);
        let count = cx.debug_bounds("download-count-controls").unwrap();
        let traffic = cx
            .debug_bounds("session-traffic")
            .expect("always-visible down/up speeds and totals");
        assert!(traffic.left() >= count.right());
        assert!(traffic.right() <= px(880.));
        assert!((traffic.center().y - count.center().y).abs() <= px(1.));
    }
}
