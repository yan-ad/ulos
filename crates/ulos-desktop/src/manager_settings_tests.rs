//! Mounted settings regressions. All persistence is confined to fresh tempdirs;
//! no OS preference paths, public downloads, or application-singleton shutdown.
use super::super::tests::{draw, mount};
use super::{DownloadManager, NativeSettingsWindow, SettingsSaved, SettingsWindow};
use crate::config::{self, ConfigSnapshot};
use gpui_kit::Entity;
use gpui_kit::{TestAppContext, VisualTestContext};
use std::time::{Duration, Instant};

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap()
}

fn settings(
    owner: &Entity<DownloadManager>,
    cx: &mut VisualTestContext,
) -> (Entity<SettingsWindow>, VisualTestContext) {
    cx.update(|window, cx| owner.update(cx, |view, cx| view.open_settings(window, cx)));
    let (content, handle) = owner.read_with(cx, |view, _| {
        let child = view
            .settings_window
            .as_ref()
            .expect("native settings child");
        (child.upgrade().unwrap(), child.handle)
    });
    let mut child = VisualTestContext::from_window(handle, &cx.cx);
    draw(&mut child);
    (content, child)
}

fn click(cx: &mut VisualTestContext, selector: &'static str) {
    draw(cx);
    let bounds = cx.debug_bounds(selector).expect("stock settings button");
    cx.simulate_click(bounds.center(), Default::default());
    cx.run_until_parked();
}

fn wait_closed(owner: &Entity<DownloadManager>, cx: &mut VisualTestContext) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        draw(cx);
        if owner.read_with(cx, |view, _| view.settings_window.is_none()) {
            break;
        }
        assert!(Instant::now() < deadline, "settings worker timed out");
        std::thread::sleep(Duration::from_millis(2));
    }
    owner.read_with(cx, |view, _| assert!(!view.options_open));
}

#[gpui::test]
fn settings_native_root_reuses_content_and_cancel_resets_options(cx: &mut TestAppContext) {
    let runtime = runtime();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("ulos.conf");
    let (_root, owner, cx) = mount(cx, &runtime);
    owner.update(cx, |view, cx| {
        view.initialize_settings(config::load(path.clone()), cx)
    });
    let (content, mut child) = settings(&owner, cx);
    let handle = owner.read_with(cx, |view, _| view.settings_window.as_ref().unwrap().handle);
    // The native Kit Root owns a NativeSettingsWindow, not the manager's main root.
    handle
        .update(cx, |root, _, cx| {
            let root = root.downcast::<gpui_kit::component::Root>().unwrap();
            let native = root
                .read(cx)
                .view()
                .clone()
                .downcast::<NativeSettingsWindow>()
                .unwrap();
            assert_eq!(native.read(cx)._owner, owner);
            assert_eq!(native.read(cx).content, content);
        })
        .unwrap();
    let (again, _) = settings(&owner, cx);
    assert_eq!(again, content);
    assert_eq!(cx.update(|_, cx| cx.windows().len()), 2);
    cx.update(|window, cx| {
        owner.update(cx, |view, cx| {
            assert!(view.options_open);
            view.open_add(window, cx);
            view.open_torrent(None, cx);
            view.open_scheduler(window, cx);
            assert!(view.add_windows.is_empty());
            assert!(view.torrent_windows.is_empty());
            assert!(view.scheduler_window.is_none());
        })
    });
    content.update(cx, |view, _| assert!(!view.busy));
    click(&mut child, "settings-cancel");
    wait_closed(&owner, cx);
    assert!(!path.exists(), "Cancel must not create preferences");
    let (reopened, mut child) = settings(&owner, cx);
    assert_ne!(reopened, content);
    child.update(|window, _| window.remove_window());
    wait_closed(&owner, cx);
    assert!(!path.exists(), "native close must not save preferences");
}

#[gpui::test]
fn settings_stock_save_publishes_pristine_config_from_native_child(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let runtime = runtime();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("preferences/ulos.conf");
    let (_root, owner, cx) = mount(cx, &runtime);
    owner.update(cx, |view, cx| {
        view.initialize_settings(config::load(path.clone()), cx)
    });
    let (_content, mut child) = settings(&owner, cx);
    assert!(!path.exists(), "opening Settings must not write");
    click(&mut child, "settings-save");
    wait_closed(&owner, cx);
    let loaded = config::load(path.clone());
    assert!(loaded.error.is_none());
    assert_eq!(loaded.snapshot, ConfigSnapshot::default());
    owner.read_with(cx, |view, _| {
        assert_eq!(view.settings.path.as_ref(), Some(&path));
        assert_eq!(view.settings.snapshot, loaded.snapshot);
        assert!(!view.clipboard_enabled);
    });
    let json = std::fs::read_to_string(path).unwrap();
    assert!(!json.contains("clipboard"));
    assert!(!json.contains("allow_upload"));
}

#[gpui::test]
fn settings_parent_rejects_forged_snapshot_path_error_and_busy_events(cx: &mut TestAppContext) {
    let runtime = runtime();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("ulos.conf");
    let baseline = config::load(path.clone());
    let (_root, owner, cx) = mount(cx, &runtime);
    owner.update(cx, |view, cx| {
        view.initialize_settings(baseline.clone(), cx)
    });
    let (content, mut child) = settings(&owner, cx);
    for kind in 0..5 {
        let mut loaded = baseline.clone();
        match kind {
            0 => loaded.snapshot.http_connections = 12,
            1 => loaded.path = Some(directory.path().join("forged.conf")),
            2 => loaded.error = Some("forged failure".into()),
            _ => {}
        }
        content.update(cx, |view, cx| {
            view.busy = kind == 4;
            cx.emit(SettingsSaved {
                loaded,
                clipboard_enabled: kind == 3,
            });
        });
        draw(&mut child);
        owner.read_with(cx, |view, _| {
            assert!(view.options_open && view.settings_window.is_some());
            assert_eq!(view.settings.snapshot, baseline.snapshot);
            assert_eq!(view.settings.path, baseline.path);
            assert!(!view.clipboard_enabled);
        });
        assert!(!path.exists());
    }
    content.update(cx, |view, _| view.busy = false);
    click(&mut child, "settings-cancel");
    wait_closed(&owner, cx);
}

#[gpui::test]
fn settings_cannot_overlap_add_status_or_scheduler(cx: &mut TestAppContext) {
    let runtime = runtime();
    let directory = tempfile::tempdir().unwrap();
    let (_root, owner, cx) = mount(cx, &runtime);
    cx.update(|window, cx| owner.update(cx, |view, cx| view.open_add(window, cx)));
    cx.update(|window, cx| {
        owner.update(cx, |view, cx| {
            view.open_settings(window, cx);
            assert!(view.settings_window.is_none() && !view.options_open);
        })
    });
    let handle = owner.read_with(cx, |view, _| {
        view.add_windows.values().next().unwrap().handle
    });
    handle
        .update(cx, |_, window, _| window.remove_window())
        .unwrap();
    draw(cx);
    let id = owner.update(cx, |view, cx| {
        view.enqueue(
            "https://example.invalid/settings-test".into(),
            directory.path().join("later.bin"),
            false,
            cx,
        )
        .unwrap()
    });
    cx.update(|window, cx| {
        owner.update(cx, |view, cx| {
            view.open_details(id, window, cx);
            view.open_settings(window, cx);
            assert!(view.settings_window.is_none() && !view.options_open);
        })
    });
    let handle = owner.read_with(cx, |view, _| view.detail_windows[&id].handle);
    handle
        .update(cx, |_, window, _| window.remove_window())
        .unwrap();
    draw(cx);
    cx.update(|window, cx| {
        owner.update(cx, |view, cx| {
            view.open_scheduler(window, cx);
            view.open_settings(window, cx);
            assert!(view.settings_window.is_none() && !view.options_open);
        })
    });
    let handle = owner.read_with(cx, |view, _| view.scheduler_window.as_ref().unwrap().handle);
    handle
        .update(cx, |_, window, _| window.remove_window())
        .unwrap();
    draw(cx);
    let (_, mut child) = settings(&owner, cx);
    click(&mut child, "settings-cancel");
    wait_closed(&owner, cx);
}

#[gpui::test]
fn settings_external_conflict_keeps_native_draft_open_without_changing_owner(
    cx: &mut TestAppContext,
) {
    cx.executor().allow_parking();
    let runtime = runtime();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("ulos.conf");
    let (_root, owner, cx) = mount(cx, &runtime);
    owner.update(cx, |view, cx| {
        view.initialize_settings(config::load(path.clone()), cx)
    });
    let (content, mut child) = settings(&owner, cx);
    std::fs::write(&path, b"external bytes\n").unwrap();
    click(&mut child, "settings-save");
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        draw(&mut child);
        if content.read_with(&child, |view, _| !view.busy) {
            break;
        }
        assert!(Instant::now() < deadline, "conflict worker timed out");
        std::thread::sleep(Duration::from_millis(2));
    }
    owner.read_with(cx, |view, _| {
        assert!(view.settings_window.is_some() && view.options_open);
        assert_eq!(view.settings.snapshot, ConfigSnapshot::default());
    });
    assert_eq!(std::fs::read(&path).unwrap(), b"external bytes\n");
    click(&mut child, "settings-cancel");
    wait_closed(&owner, cx);
}

#[gpui::test]
fn settings_saved_defaults_apply_only_to_new_native_http_drafts(cx: &mut TestAppContext) {
    let runtime = runtime();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("ulos.conf");
    let initial = config::load(path)
        .save(ConfigSnapshot {
            default_directory: Some(directory.path().to_owned()),
            http_connections: 7,
            http_rate_limit: Some(8192),
            torrent_peer_limit: 321,
            torrent_download_limit: Some(2049),
            torrent_upload_limit: Some(1025),
            dht_enabled: true,
            ..Default::default()
        })
        .unwrap();
    let (_root, owner, cx) = mount(cx, &runtime);
    owner.update(cx, |view, cx| view.initialize_settings(initial.clone(), cx));
    cx.update(|window, cx| owner.update(cx, |view, cx| view.open_add(window, cx)));
    let first = owner.read_with(cx, |view, _| {
        view.add_windows.values().next().unwrap().upgrade().unwrap()
    });
    first.read_with(cx, |form, cx| {
        assert_eq!(form.options.connections, 7);
        assert_eq!(form.options.bytes_per_second, Some(8192));
        // Saved DHT is a preference, never persisted network/upload consent.
        let review = form.magnet_review.read(cx);
        assert!(!review.options.settings.allow_upload);
        assert!(!review.options.settings.dht);
    });
    let mut next = initial.clone();
    next.snapshot.http_connections = 2;
    next.snapshot.http_rate_limit = None;
    owner.update(cx, |view, cx| view.initialize_settings(next, cx));
    cx.update(|window, cx| owner.update(cx, |view, cx| view.open_add(window, cx)));
    owner.read_with(cx, |view, cx| {
        assert_eq!(view.add_windows.len(), 2);
        let fresh = view
            .add_windows
            .values()
            .map(|child| child.upgrade().unwrap())
            .find(|form| *form != first)
            .unwrap();
        assert_eq!(fresh.read(cx).options.connections, 2);
        assert_eq!(fresh.read(cx).options.bytes_per_second, None);
        assert_eq!(first.read(cx).options.connections, 7);
        assert_eq!(first.read(cx).options.bytes_per_second, Some(8192));
    });
}

#[gpui::test]
fn settings_saved_torrent_defaults_never_grant_native_draft_consent(cx: &mut TestAppContext) {
    let runtime = runtime();
    let directory = tempfile::tempdir().unwrap();
    let loaded = config::load(directory.path().join("ulos.conf"))
        .save(ConfigSnapshot {
            default_directory: Some(directory.path().to_owned()),
            torrent_download_limit: Some(2049),
            torrent_upload_limit: Some(1025),
            torrent_peer_limit: 321,
            dht_enabled: true,
            ..Default::default()
        })
        .unwrap();
    let (_root, owner, cx) = mount(cx, &runtime);
    owner.update(cx, |view, cx| view.initialize_settings(loaded, cx));
    owner.update(cx, |view, cx| view.open_torrent(None, cx));
    owner.read_with(cx, |view, cx| {
        let form = view
            .torrent_windows
            .values()
            .next()
            .unwrap()
            .upgrade()
            .unwrap();
        assert_eq!(
            form.read(cx).options.settings,
            ulos_core::torrent::TorrentSettings::default()
        );
        assert!(form.read(cx).source.is_none());
        assert!(!form.read(cx).can_submit(cx));
    });
}
