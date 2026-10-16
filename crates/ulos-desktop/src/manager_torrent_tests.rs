//! Mounted Kit integration tests. Torrent fixtures contain no trackers; tests
//! review local metadata and never require peers, public HTTP, or OS registration.
use std::{
    cell::RefCell,
    path::{Path, PathBuf},
    rc::Rc,
    thread,
    time::{Duration, Instant},
};

use gpui_kit::{
    AppContext, Entity, TestAppContext, VisualTestContext, component::Root,
    component::input::InputEvent,
};
use ulos_core::{tasks::TransferState, torrent::TorrentOptions, transfer::CollisionPolicy};

use super::DownloadManager;
use crate::{
    add_torrent::{AddTorrent, TorrentRequest},
    queue::{Category, Filter},
};

struct TestRuntime(Option<tokio::runtime::Runtime>);

impl std::ops::Deref for TestRuntime {
    type Target = tokio::runtime::Runtime;
    fn deref(&self) -> &Self::Target {
        self.0.as_ref().unwrap()
    }
}

impl Drop for TestRuntime {
    fn drop(&mut self) {
        // GPUI's context may still retain the manager on assertion unwind. A
        // blocking torrent worker must not turn a failed fixture into a CI hang.
        if let Some(runtime) = self.0.take() {
            runtime.shutdown_timeout(Duration::from_secs(3));
        }
    }
}

fn runtime() -> TestRuntime {
    TestRuntime(Some(
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("test runtime"),
    ))
}

#[gpui::test]
fn external_file_drop_opens_only_an_offline_review(cx: &mut TestAppContext) {
    let runtime = runtime();
    let directory = tempfile::tempdir().unwrap();
    let path = fixture(directory.path());
    let (_root, view, cx) = mount(cx, &runtime);
    let position = cx.debug_bounds("manager-shell").unwrap().center();
    cx.simulate_event(gpui_kit::FileDropEvent::Entered {
        position,
        paths: gpui_kit::ExternalPaths(vec![path].into()),
    });
    draw(cx);
    cx.simulate_event(gpui_kit::FileDropEvent::Submit { position });
    cx.simulate_event(gpui_kit::FileDropEvent::Ended);
    draw(cx);
    let (form, mut child) = draft(&view, cx);
    poll(&mut child, |cx| {
        form.read_with(cx, |form, _| form.source.is_some())
    });
    view.read_with(&child, |view, _| {
        assert!(view.queue.items.is_empty());
        assert!(view.active.is_none() && view.transfer.is_none());
    });
    assert!(!directory.path().join("file.bin").exists());
}

fn metainfo(name: &[u8; 8]) -> Vec<u8> {
    let mut bytes = b"d4:infod6:lengthi3e4:name8:".to_vec();
    bytes.extend_from_slice(name);
    bytes.extend_from_slice(b"12:piece lengthi16384e6:pieces20:");
    // SHA-1 of the three-byte payload `abc`.
    bytes.extend_from_slice(&[
        0xa9, 0x99, 0x3e, 0x36, 0x47, 0x06, 0x81, 0x6a, 0xba, 0x3e, 0x25, 0x71, 0x78, 0x50, 0xc2,
        0x6c, 0x9c, 0xd0, 0xd8, 0x9d,
    ]);
    bytes.extend_from_slice(b"ee");
    bytes
}

fn fixture(directory: &Path) -> PathBuf {
    let path = directory.join("local.torrent");
    std::fs::write(&path, metainfo(b"file.bin")).unwrap();
    path
}

fn mount<'a>(
    cx: &'a mut TestAppContext,
    runtime: &tokio::runtime::Runtime,
) -> (
    Entity<Root>,
    Entity<DownloadManager>,
    &'a mut VisualTestContext,
) {
    cx.update(gpui_kit::init);
    cx.update(crate::palette::init);
    let mut manager = None;
    let handle = runtime.handle().clone();
    let (root, cx) = cx.add_window_view(|window, cx| {
        let view = cx.new(|cx| DownloadManager::new(handle, window, cx));
        manager = Some(view.clone());
        Root::new(view, window, cx)
    });
    draw(cx);
    (root, manager.unwrap(), cx)
}

fn draw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
}

// GPUI timers are deterministic; the separate Tokio worker still needs real time.
fn poll(cx: &mut VisualTestContext, mut ready: impl FnMut(&mut VisualTestContext) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        cx.executor().advance_clock(Duration::from_millis(100));
        draw(cx);
        if ready(cx) {
            return;
        }
        assert!(Instant::now() < deadline, "torrent observation timed out");
        thread::sleep(Duration::from_millis(1));
    }
}

fn draft(
    view: &Entity<DownloadManager>,
    cx: &VisualTestContext,
) -> (Entity<AddTorrent>, VisualTestContext) {
    view.read_with(cx, |view, _| {
        let child = view.torrent_windows.values().next().unwrap();
        (
            child.upgrade().unwrap(),
            VisualTestContext::from_window(child.handle, &cx.cx),
        )
    })
}

fn destination(form: &Entity<AddTorrent>, path: &Path, cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        form.update(cx, |form, cx| {
            form.destination.update(cx, |input, cx| {
                input.set_value(path.to_str().unwrap(), window, cx);
                cx.emit(InputEvent::Change);
            });
        });
    });
    poll(cx, |cx| form.read_with(cx, |form, cx| form.can_submit(cx)));
}

#[gpui::test]
fn native_torrent_picker_reviews_offline_and_queues_in_shared_table(cx: &mut TestAppContext) {
    let runtime = runtime();
    let directory = tempfile::tempdir().unwrap();
    let source_path = fixture(directory.path());
    let original = std::fs::read(&source_path).unwrap();
    let target = directory.path().join("chosen.bin");
    let (_root, view, cx) = mount(cx, &runtime);
    let _menus = cx.update(|window, cx| {
        crate::native_menu::install(cx, view.downgrade(), window.window_handle())
    });
    cx.dispatch_action(crate::native_menu::OpenTorrentFile);
    draw(cx);
    assert!(cx.did_prompt_for_paths());
    assert!(
        !cx.simulate_close(),
        "main window must retain its outstanding picker"
    );
    cx.simulate_path_prompt_response(|options| {
        assert!(options.files && options.multiple && !options.directories);
        Some(vec![source_path.clone()])
    });
    draw(cx);
    let (form, mut child) = draft(&view, cx);
    poll(&mut child, |cx| {
        form.read_with(cx, |form, _| form.source.is_some())
    });
    form.read_with(&child, |form, _| {
        let source = form.source.as_ref().unwrap();
        assert_eq!(source.name(), "file.bin");
        assert_eq!(source.total_bytes(), 3);
        assert_eq!(source.tracker_count(), 0);
    });
    destination(&form, &target, &mut child);
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.clipboard_enabled = true;
            view.offer_clipboard(Some("https://example.invalid/copied"), window, cx);
            assert!(
                view.add_windows.is_empty(),
                "torrent reviews suppress clipboard drafts"
            );
            assert!(view.active.is_none());
            assert!(view.transfer.is_none());
        })
    });
    form.update(&mut child, |form, cx| form.submit(false, cx));
    draw(cx);
    view.read_with(cx, |view, cx| {
        assert!(view.torrent_windows.is_empty());
        assert_eq!(view.queue.items.len(), 1);
        let item = &view.queue.items[0];
        assert!(item.is_torrent && item.is_queued());
        assert_eq!(item.category, Category::Other);
        assert_eq!(item.total_bytes, Some(3));
        assert_eq!(item.destination, target);
        assert!(view.active.is_none() && view.transfer.is_none());
        let row = &view.table.read(cx).delegate().rows[0];
        assert_eq!(row.id, item.id);
        assert!(row.is_torrent);
        let snapshot = view.snapshot(item.id).unwrap();
        assert!(snapshot.is_torrent);
        assert_eq!(snapshot.source, "Local torrent metadata");
        assert!(snapshot.telemetry.connections.is_empty());
    });
    assert!(!target.exists(), "review/queue must not create payloads");
    assert_eq!(std::fs::read(source_path).unwrap(), original);
}

#[gpui::test]
fn torrent_parent_rejects_forged_source_destination_and_collision(cx: &mut TestAppContext) {
    let runtime = runtime();
    let directory = tempfile::tempdir().unwrap();
    let path = fixture(directory.path());
    let (_root, view, cx) = mount(cx, &runtime);
    view.update(cx, |view, cx| view.offer_torrent_paths(vec![path], cx));
    let (form, mut child) = draft(&view, cx);
    poll(&mut child, |cx| {
        form.read_with(cx, |form, _| form.source.is_some())
    });
    let target = directory.path().join("reviewed.bin");
    destination(&form, &target, &mut child);
    for mismatch in 0..3 {
        form.update(&mut child, |form, cx| {
            let mut request = TorrentRequest {
                source: form.source.clone().unwrap(),
                destination: target.clone(),
                start_now: true,
                options: form.options.clone(),
            };
            match mismatch {
                0 => request.source = ulos_core::torrent::parse(metainfo(b"else.bin")).unwrap(),
                1 => request.destination = directory.path().join("unreviewed.bin"),
                _ => request.options.collision = CollisionPolicy::Overwrite,
            }
            cx.emit(request);
        });
        draw(&mut child);
        view.read_with(&child, |view, _| {
            assert!(view.queue.items.is_empty());
            assert_eq!(view.torrent_windows.len(), 1);
            assert!(view.active.is_none() && view.transfer.is_none());
        });
    }
    form.update(&mut child, |form, cx| form.submit(false, cx));
    draw(cx);
    view.read_with(cx, |view, _| assert_eq!(view.queue.items.len(), 1));
    assert!(!target.exists());
    assert!(!directory.path().join("unreviewed.bin").exists());
}

#[gpui::test]
fn torrent_native_draft_retains_owner_after_main_close_without_cycle(cx: &mut TestAppContext) {
    let runtime = runtime();
    let directory = tempfile::tempdir().unwrap();
    let path = fixture(directory.path());
    let (root, view, cx) = mount(cx, &runtime);
    view.update(cx, |view, cx| view.offer_torrent_paths(vec![path], cx));
    let (form, mut child) = draft(&view, cx);
    poll(&mut child, |cx| {
        form.read_with(cx, |form, _| form.source.is_some())
    });
    let weak = view.downgrade();
    cx.update(|window, _| window.remove_window());
    drop(root);
    drop(view);
    draw(&mut child);
    assert!(weak.upgrade().is_some());
    destination(&form, &directory.path().join("surviving.bin"), &mut child);
    let observer = weak.upgrade().unwrap();
    form.update(&mut child, |form, cx| form.submit(false, cx));
    child.run_until_parked();
    observer.read_with(&child, |view, _| {
        assert_eq!(view.queue.items.len(), 1);
        assert!(view.queue.items[0].is_torrent && view.queue.items[0].is_queued());
        assert!(view.torrent_windows.is_empty());
    });
    drop(observer);
    drop(form);
    child.run_until_parked();
    assert!(
        weak.upgrade().is_none(),
        "closed torrent roots must release their owner"
    );
    assert_eq!(child.cx.update(|cx| cx.windows().len()), 0);
    assert!(!directory.path().join("surviving.bin").exists());
}

#[gpui::test]
fn torrent_intake_is_atomic_bounded_and_drains_callback_inbox(cx: &mut TestAppContext) {
    let runtime = runtime();
    let directory = tempfile::tempdir().unwrap();
    let path = fixture(directory.path());
    let (_root, view, cx) = mount(cx, &runtime);
    view.update(cx, |view, cx| {
        view.offer_torrent_paths(vec![path.clone(), directory.path().join("wrong.txt")], cx);
        assert!(
            view.torrent_windows.is_empty(),
            "reject the entire mixed batch"
        );
        assert!(view.notice.is_some());
    });
    let inbox = Rc::new(RefCell::new(vec![path.clone()]));
    let error = Rc::new(RefCell::new(None));
    view.update(cx, |view, cx| {
        view.watch_torrent_intake(inbox.clone(), error.clone(), cx)
    });
    poll(cx, |cx| {
        view.read_with(cx, |view, _| view.torrent_windows.len() == 1)
    });
    assert!(inbox.borrow().is_empty());
    view.update(cx, |view, cx| {
        // Test the shared HTTP/torrent draft limit, not merely a torrent-only cap.
        for _ in 0..14 {
            view.open_torrent(None, cx);
        }
        assert_eq!(view.torrent_windows.len(), 15);
        view.offer_torrent_paths(
            vec![path.clone(), directory.path().join("second.torrent")],
            cx,
        );
        assert_eq!(
            view.torrent_windows.len(),
            15,
            "oversized batches are atomic"
        );
    });
    cx.update(|window, cx| view.update(cx, |view, cx| view.open_add(window, cx)));
    view.update(cx, |view, cx| {
        assert_eq!(view.torrent_windows.len() + view.add_windows.len(), 16);
        view.open_torrent(None, cx);
        assert_eq!(view.torrent_windows.len(), 15);
    });
    *error.borrow_mut() = Some("Torrent intake unavailable.".into());
    poll(cx, |cx| {
        view.read_with(cx, |view, _| {
            view.notice.as_deref() == Some("Torrent intake unavailable.")
        })
    });
    assert!(error.borrow().is_none());
    view.read_with(cx, |view, _| {
        assert!(view.queue.items.is_empty() && view.active.is_none())
    });
}

#[gpui::test]
fn mixed_fifo_retains_torrent_slot_through_pause_and_cancellation(cx: &mut TestAppContext) {
    let runtime = runtime();
    let directory = tempfile::tempdir().unwrap();
    let source = ulos_core::torrent::parse(metainfo(b"file.bin")).unwrap();
    let (_root, view, cx) = mount(cx, &runtime);
    let original = directory.path().join("original.bin");
    std::fs::write(&original, b"preserve original").unwrap();
    let target = directory.path().join("pending.bin");
    let (first, torrent, last) = view.update(cx, |view, cx| {
        // Invalid schemes fail locally in the HTTP engine; no network fixture needed.
        let first = view
            .enqueue("file:///invalid-first".into(), original.clone(), false, cx)
            .unwrap();
        let torrent = view
            .enqueue_torrent(
                &TorrentRequest {
                    source,
                    destination: target.clone(),
                    start_now: false,
                    options: TorrentOptions::default(),
                },
                cx,
            )
            .unwrap();
        let last = view
            .enqueue(
                "file:///invalid-last".into(),
                directory.path().join("last.bin"),
                false,
                cx,
            )
            .unwrap();
        // Presentation filters must not change dispatch order.
        view.queue.filter = Filter::Category(Category::Image);
        view.start_queue(cx);
        assert_eq!(view.active, Some(first));
        (first, torrent, last)
    });
    poll(cx, |cx| {
        view.read_with(cx, |view, _| view.active == Some(torrent))
    });
    view.read_with(cx, |view, _| {
        assert!(matches!(
            view.queue.item(first).unwrap().state,
            Some(TransferState::Failed(_))
        ));
        assert_eq!(view.queue.active_count(), 1);
        assert!(view.queue.item(last).unwrap().is_queued());
    });
    poll(cx, |cx| {
        view.read_with(cx, |view, _| {
            let item = view.queue.item(torrent).unwrap();
            matches!(item.state, Some(TransferState::Downloading(_)))
                && item.torrent_telemetry.as_ref().is_some_and(|telemetry| {
                    telemetry.phase == ulos_core::torrent::TorrentPhase::Downloading
                })
        })
    });
    view.update(cx, |view, cx| view.toggle_pause(torrent, cx));
    poll(cx, |cx| {
        view.read_with(cx, |view, _| {
            matches!(
                view.queue.item(torrent).unwrap().state,
                Some(TransferState::Paused(_))
            )
        })
    });
    view.read_with(cx, |view, _| {
        assert_eq!(view.active, Some(torrent));
        assert_eq!(view.queue.active_count(), 1);
        assert!(view.queue.item(last).unwrap().is_queued());
        let snapshot = view.snapshot(torrent).unwrap();
        assert!(snapshot.is_torrent && snapshot.paused);
        assert!(snapshot.telemetry.connections.is_empty());
    });
    view.update(cx, |view, cx| {
        view.cancel_id(torrent, cx);
        assert_eq!(
            view.active,
            Some(torrent),
            "cancel retains slot until worker teardown"
        );
        assert!(view.queue.item(last).unwrap().is_queued());
    });
    poll(cx, |cx| {
        view.read_with(cx, |view, _| {
            view.active.is_none()
                && matches!(
                    view.queue.item(last).unwrap().state,
                    Some(TransferState::Failed(_))
                )
        })
    });
    view.read_with(cx, |view, _| {
        assert!(matches!(
            view.queue.item(torrent).unwrap().state,
            Some(TransferState::Cancelled)
        ));
        assert_eq!(view.queue.active_count(), 0);
    });
    assert_eq!(std::fs::read(original).unwrap(), b"preserve original");
    assert!(!target.exists());
    assert!(!directory.path().join("last.bin").exists());
}

#[gpui::test]
fn add_new_dropdown_opens_independent_torrent_review(cx: &mut TestAppContext) {
    let runtime = runtime();
    let (_root, view, cx) = mount(cx, &runtime);
    let add = cx.debug_bounds("add-url").expect("stock Add New button");
    cx.simulate_click(add.center(), gpui_kit::Modifiers::default());
    draw(cx);
    // Stock PopupMenu starts without selection. Navigate the two authored items.
    cx.simulate_keystrokes("down down enter");
    draw(cx);
    view.read_with(cx, |view, cx| {
        assert_eq!(view.torrent_windows.len(), 1);
        assert!(view.add_windows.is_empty());
        assert!(view.queue.items.is_empty() && view.active.is_none());
        let form = view
            .torrent_windows
            .values()
            .next()
            .unwrap()
            .upgrade()
            .unwrap();
        assert!(form.read(cx).source.is_none());
        assert!(!form.read(cx).can_submit(cx));
    });
}

#[gpui::test]
fn torrent_callback_inbox_survives_main_close_while_review_owns_manager(cx: &mut TestAppContext) {
    let runtime = runtime();
    let directory = tempfile::tempdir().unwrap();
    let path = fixture(directory.path());
    let (root, view, cx) = mount(cx, &runtime);
    let inbox = Rc::new(RefCell::new(Vec::new()));
    let error = Rc::new(RefCell::new(None));
    view.update(cx, |view, cx| {
        view.open_torrent(None, cx);
        view.watch_torrent_intake(inbox.clone(), error, cx);
    });
    let (form, mut child) = draft(&view, cx);
    let weak = view.downgrade();
    cx.update(|window, _| window.remove_window());
    drop(root);
    drop(view);
    child.run_until_parked();
    inbox.borrow_mut().push(path);
    poll(&mut child, |cx| {
        weak.upgrade()
            .unwrap()
            .read_with(cx, |view, _| view.torrent_windows.len() == 2)
    });
    assert!(inbox.borrow().is_empty());
    let observer = weak.upgrade().unwrap();
    observer.read_with(&child, |view, _| {
        assert!(view.active.is_none() && view.transfer.is_none());
        assert!(view.queue.items.is_empty());
    });
    let forms = observer.read_with(&child, |view, _| {
        view.torrent_windows
            .values()
            .map(|child| child.upgrade().unwrap())
            .collect::<Vec<_>>()
    });
    for form in forms {
        form.update(&mut child, |form, cx| form.dismiss(cx));
    }
    child.run_until_parked();
    drop(form);
    drop(observer);
    child.run_until_parked();
    assert!(weak.upgrade().is_none());
}
