use std::{
    io::{BufRead, BufReader, Write},
    net::TcpListener,
    sync::mpsc::{self, Receiver, Sender, TryRecvError},
    thread,
    time::{Duration, Instant},
};

use gpui_kit::{AppContext, Entity, TestAppContext, VisualTestContext, component::Root};
use ulos_core::{DownloadProgress, DownloadReceipt};

use super::DownloadManager;
use crate::queue::{DownloadId, Filter};
use ulos_core::tasks::TransferState;

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("test runtime")
}

#[gpui::test]
fn manual_save_path_overrides_a_previously_selected_folder(cx: &mut TestAppContext) {
    let runtime = runtime();
    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("manual.zip");
    let (_root, view, cx) = mount(cx, &runtime);
    cx.update(|window, cx| view.update(cx, |view, cx| view.open_add(window, cx)));
    draw(cx);
    let form = view.read_with(cx, |view, _| view.add_form.clone().unwrap());
    cx.update(|window, cx| {
        form.update(cx, |form, cx| {
            form.prefill("https://example.org/automatic.zip".into(), window, cx);
            form.choose_folder(window, cx);
        })
    });
    cx.simulate_path_prompt_response(|_| Some(vec![directory.path().join("previous")]));
    draw(cx);
    cx.update(|window, cx| {
        form.read(cx)
            .destination
            .clone()
            .update(cx, |input, cx| input.focus(window, cx))
    });
    draw(cx);
    cx.simulate_keystrokes("secondary-a");
    cx.simulate_input(destination.to_str().unwrap());
    draw(cx);
    form.read_with(cx, |form, cx| {
        assert_eq!(form.filename.read(cx).value().as_ref(), "manual.zip")
    });
    cx.update(|window, cx| {
        form.read(cx)
            .filename
            .clone()
            .update(cx, |input, cx| input.focus(window, cx))
    });
    draw(cx);
    cx.simulate_keystrokes("secondary-a");
    cx.simulate_input("final?.zip");
    draw(cx);
    form.read_with(cx, |form, cx| {
        assert_eq!(
            form.destination.read(cx).value().as_ref(),
            directory.path().join("final_.zip").to_str().unwrap()
        )
    });
    form.update(cx, |form, cx| form.submit(false, cx));
    draw(cx);
    view.read_with(cx, |view, _| {
        assert_eq!(
            view.queue.items[0].destination,
            directory.path().join("final_.zip")
        )
    });
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
    cx.simulate_input("sAvEd");
    draw(cx);
    (root, manager.unwrap(), cx)
}

fn draw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
}

// GPUI's deterministic clock does not drive the separate Tokio runtime.
fn poll(cx: &mut VisualTestContext, mut ready: impl FnMut(&mut VisualTestContext) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        cx.executor().advance_clock(Duration::from_millis(100));
        draw(cx);
        if ready(cx) {
            return;
        }
        assert!(Instant::now() < deadline, "manager observation timed out");
        thread::sleep(Duration::from_millis(1));
    }
}

struct Fixture {
    source: String,
    requested: Receiver<()>,
    release: Sender<()>,
    server: thread::JoinHandle<()>,
}

// Every accept, header read, write and gate is bounded. The gate follows one
// body byte so cancellation can be tested after a real temporary file exists.
fn fixture(body: &'static [u8]) -> Fixture {
    assert!(body.len() > 1);
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let source = format!("http://{}/file", listener.local_addr().unwrap());
    listener.set_nonblocking(true).unwrap();
    let (requested_tx, requested) = mpsc::channel();
    let (release, release_rx) = mpsc::channel();
    let server = thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut stream = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(Instant::now() < deadline, "fixture accept timed out");
                    thread::sleep(Duration::from_millis(1));
                }
                Err(error) => panic!("fixture accept: {error}"),
            }
        };
        stream
            .set_read_timeout(Some(Duration::from_millis(100)))
            .unwrap();
        stream
            .set_write_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut headers = Vec::new();
        loop {
            assert!(Instant::now() < deadline, "fixture headers timed out");
            match reader.read_until(b'\n', &mut headers) {
                Ok(0) => panic!("client closed before headers"),
                Ok(_) if headers.ends_with(b"\r\n\r\n") => break,
                Ok(_) => {}
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) => {}
                Err(error) => panic!("fixture headers: {error}"),
            }
            assert!(headers.len() <= 64 * 1024, "fixture headers too large");
        }
        assert!(headers.starts_with(b"GET /file HTTP/1.1\r\n"));
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        )
        .unwrap();
        stream.write_all(&body[..1]).unwrap();
        requested_tx.send(()).unwrap();
        release_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("fixture gate timed out");
        // Cancellation may close the connection before the gate is released.
        let _ = stream.write_all(&body[1..]);
    });
    Fixture {
        source,
        requested,
        release,
        server,
    }
}

fn queued_pair(
    view: &Entity<DownloadManager>,
    cx: &mut VisualTestContext,
    first: String,
    second: String,
    directory: &std::path::Path,
) -> (DownloadId, DownloadId) {
    view.update(cx, |view, cx| {
        let first = view
            .enqueue(first, directory.join("first.bin"), false, cx)
            .unwrap();
        let second = view
            .enqueue(second, directory.join("second.bin"), false, cx)
            .unwrap();
        assert!(view.active.is_none());
        assert!(!view.queue_running);
        assert!(view.queue.items.iter().all(|item| item.is_queued()));
        (first, second)
    })
}

#[gpui::test]
fn start_queue_is_fifo_even_when_search_and_filter_hide_every_row(cx: &mut TestAppContext) {
    let runtime = runtime();
    let directory = tempfile::tempdir().unwrap();
    let first = fixture(b"first payload");
    let second = fixture(b"second payload");
    let (_root, view, cx) = mount(cx, &runtime);
    cx.update(|window, _| window.activate_window());
    draw(cx);
    let (a, b) = queued_pair(
        &view,
        cx,
        first.source.clone(),
        second.source.clone(),
        directory.path(),
    );
    view.update(cx, |view, cx| {
        view.queue.search = "nothing matches".into();
        view.set_filter(Filter::Finished, cx);
        assert!(view.queue.visible().is_empty());
        view.start_queue(cx);
        assert_eq!(view.active, Some(a));
    });
    poll(cx, |_| first.requested.try_recv().is_ok());
    assert!(matches!(
        second.requested.try_recv(),
        Err(TryRecvError::Empty)
    ));
    view.read_with(cx, |view, _| {
        assert_eq!(view.active, Some(a));
        assert!(view.queue.item(b).unwrap().is_queued());
    });
    first.release.send(()).unwrap();
    poll(cx, |_| second.requested.try_recv().is_ok());
    view.read_with(cx, |view, _| {
        assert!(view.queue.item(a).unwrap().is_finished());
        assert_eq!(view.active, Some(b));
    });
    second.release.send(()).unwrap();
    poll(cx, |cx| {
        view.read_with(cx, |view, _| view.active.is_none() && !view.queue_running)
    });
    first.server.join().unwrap();
    second.server.join().unwrap();
    view.read_with(cx, |view, _| {
        assert!(view.queue.items.iter().all(|item| item.is_finished()));
        assert!(view.transfer.is_none());
    });
    assert_eq!(
        std::fs::read(directory.path().join("first.bin")).unwrap(),
        b"first payload"
    );
    assert_eq!(
        std::fs::read(directory.path().join("second.bin")).unwrap(),
        b"second payload"
    );
}

#[gpui::test]
fn stop_queue_finishes_active_but_does_not_request_next(cx: &mut TestAppContext) {
    let runtime = runtime();
    let directory = tempfile::tempdir().unwrap();
    let first = fixture(b"finish me");
    let unrequested = TcpListener::bind("127.0.0.1:0").unwrap();
    unrequested.set_nonblocking(true).unwrap();
    let (_root, view, cx) = mount(cx, &runtime);
    let (a, b) = queued_pair(
        &view,
        cx,
        first.source.clone(),
        format!("http://{}/file", unrequested.local_addr().unwrap()),
        directory.path(),
    );
    view.update(cx, |view, cx| view.start_queue(cx));
    poll(cx, |_| first.requested.try_recv().is_ok());
    view.update(cx, |view, cx| {
        view.stop_queue(cx);
        assert_eq!(view.active, Some(a));
        assert!(!view.queue.item(a).unwrap().cancelling);
    });
    first.release.send(()).unwrap();
    poll(cx, |cx| view.read_with(cx, |view, _| view.active.is_none()));
    first.server.join().unwrap();
    view.read_with(cx, |view, _| {
        assert!(!view.queue_running);
        assert!(view.queue.item(a).unwrap().is_finished());
        assert!(view.queue.item(b).unwrap().is_queued());
    });
    assert_eq!(
        unrequested.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
    assert_eq!(
        std::fs::read(directory.path().join("first.bin")).unwrap(),
        b"finish me"
    );
    assert!(!directory.path().join("second.bin").exists());
}

#[gpui::test]
fn stop_all_cancels_partial_transfer_and_leaves_next_queued(cx: &mut TestAppContext) {
    let runtime = runtime();
    let directory = tempfile::tempdir().unwrap();
    let first = fixture(b"cancel me");
    let unrequested = TcpListener::bind("127.0.0.1:0").unwrap();
    unrequested.set_nonblocking(true).unwrap();
    let (_root, view, cx) = mount(cx, &runtime);
    let (a, b) = queued_pair(
        &view,
        cx,
        first.source.clone(),
        format!("http://{}/file", unrequested.local_addr().unwrap()),
        directory.path(),
    );
    view.update(cx, |view, cx| view.start_queue(cx));
    poll(cx, |_| first.requested.try_recv().is_ok());
    poll(cx, |cx| {
        view.read_with(cx, |view, _| {
            view.queue.item(a).unwrap().downloaded_bytes == 1
        })
    });
    view.update(cx, |view, cx| {
        view.stop_all(cx);
        assert!(!view.queue_running);
        assert!(view.queue.item(a).unwrap().cancelling);
    });
    poll(cx, |cx| view.read_with(cx, |view, _| view.active.is_none()));
    first.release.send(()).unwrap();
    first.server.join().unwrap();
    view.read_with(cx, |view, _| {
        assert_eq!(
            view.queue.item(a).unwrap().state,
            Some(TransferState::Cancelled)
        );
        assert!(!view.queue.item(a).unwrap().cancelling);
        assert!(view.queue.item(b).unwrap().is_queued());
        assert!(view.transfer.is_none());
    });
    assert_eq!(
        unrequested.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
}

#[gpui::test]
fn search_select_visible_and_remove_only_change_session_metadata(cx: &mut TestAppContext) {
    let runtime = runtime();
    let directory = tempfile::tempdir().unwrap();
    let saved = directory.path().join("Saved.txt");
    std::fs::write(&saved, b"keep these bytes").unwrap();
    let (_root, view, cx) = mount(cx, &runtime);
    let (finished, hidden) = view.update(cx, |view, cx| {
        let finished = view
            .enqueue(
                "https://example.invalid/saved".into(),
                saved.clone(),
                false,
                cx,
            )
            .unwrap();
        view.queue.item_mut(finished).unwrap().apply_state(
            TransferState::Completed(DownloadReceipt {
                destination: saved.clone(),
                downloaded_bytes: 16,
            }),
            Instant::now(),
        );
        let hidden = view
            .enqueue(
                "https://example.invalid/hidden".into(),
                directory.path().join("hidden.bin"),
                false,
                cx,
            )
            .unwrap();
        (finished, hidden)
    });
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.search.update(cx, |input, cx| input.focus(window, cx));
        })
    });
    draw(cx);
    cx.simulate_input("sAvEd");
    draw(cx);
    view.update(cx, |view, cx| {
        assert_eq!(view.queue.search, "sAvEd");
        assert_eq!(
            view.queue
                .visible()
                .iter()
                .map(|item| item.id)
                .collect::<Vec<_>>(),
            vec![finished]
        );
        view.queue.select_visible(true);
        assert!(view.queue.all_visible_selected());
        assert!(!view.queue.selected.contains(&hidden));
        view.remove_selected(cx);
        assert!(view.queue.item(finished).is_none());
        assert!(view.queue.item(hidden).unwrap().is_queued());
        assert!(view.queue.selected.is_empty());
        assert!(
            view.notice
                .as_ref()
                .unwrap()
                .contains("Files were not deleted")
        );
    });
    assert_eq!(std::fs::read(saved).unwrap(), b"keep these bytes");
}

#[gpui::test]
fn add_dialog_retains_inputs_handles_save_prompt_and_emits_queue_request(cx: &mut TestAppContext) {
    let runtime = runtime();
    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("selected.bin");
    let (_root, view, cx) = mount(cx, &runtime);
    cx.update(|window, cx| view.update(cx, |view, cx| view.open_add(window, cx)));
    draw(cx);
    let form = view.read_with(cx, |view, _| view.add_form.clone().unwrap());
    let source = form.read_with(cx, |form, _| form.source.clone());
    cx.update(|window, cx| source.update(cx, |input, cx| input.focus(window, cx)));
    draw(cx);
    cx.simulate_input("https://example.invalid/file");
    draw(cx);
    view.update(cx, |_, cx| cx.notify());
    draw(cx);
    cx.simulate_input(".bin");
    draw(cx);
    form.read_with(cx, |form, cx| {
        assert_eq!(form.source, source);
        assert_eq!(
            source.read(cx).value().as_ref(),
            "https://example.invalid/file.bin"
        );
        assert!(!form.can_submit(cx));
    });
    cx.update(|window, cx| form.update(cx, |form, cx| form.choose_path(window, cx)));
    form.read_with(cx, |form, cx| {
        assert!(form.choosing_path);
        assert!(!form.can_submit(cx));
    });
    cx.simulate_new_path_selection(|_| Some(destination.clone()));
    draw(cx);
    cx.update(|window, cx| form.update(cx, |form, cx| form.choose_path(window, cx)));
    cx.simulate_new_path_selection(|_| None);
    draw(cx);
    form.read_with(cx, |form, cx| {
        assert!(!form.choosing_path);
        assert_eq!(
            form.destination.read(cx).value().as_ref(),
            destination.to_str().unwrap()
        );
        assert!(form.can_submit(cx));
    });
    // submit uses the actual AddRequest emitter and manager subscription.
    form.update(cx, |form, cx| form.submit(false, cx));
    draw(cx);
    view.read_with(cx, |view, _| {
        assert!(view.add_form.is_none());
        assert!(view._add_subscription.is_none());
        assert_eq!(view.queue.items.len(), 1);
        assert_eq!(
            view.queue.items[0].source(),
            "https://example.invalid/file.bin"
        );
        assert_eq!(view.queue.items[0].destination, destination);
        assert!(view.queue.items[0].is_queued());
        assert!(view.active.is_none());
    });
    assert!(!destination.exists());
    cx.update(|window, cx| view.update(cx, |view, cx| view.open_add(window, cx)));
    draw(cx);
    view.read_with(cx, |view, _| {
        assert!(view.add_form.as_ref().is_some_and(|new| *new != form))
    });
    let reopened = view.read_with(cx, |view, _| view.add_form.clone().unwrap());
    reopened.update(cx, |_, cx| cx.emit(crate::add_download::DismissAdd));
    draw(cx);
    view.read_with(cx, |view, _| assert!(view.add_form.is_none()));
}

#[gpui::test]
fn unsupported_download_now_fails_and_a_future_queue_still_runs(cx: &mut TestAppContext) {
    let runtime = runtime();
    let directory = tempfile::tempdir().unwrap();
    let (_root, view, cx) = mount(cx, &runtime);
    let failed = view.update(cx, |view, cx| {
        view.enqueue(
            "magnet:?xt=unsupported".into(),
            directory.path().join("invalid.bin"),
            true,
            cx,
        )
        .unwrap()
    });
    poll(cx, |cx| view.read_with(cx, |view, _| view.active.is_none()));
    view.read_with(cx, |view, _| {
        assert!(matches!(
            view.queue.item(failed).unwrap().state,
            Some(TransferState::Failed(_))
        ));
        assert!(view.transfer.is_none());
    });
    let valid = fixture(b"recovered");
    let next = view.update(cx, |view, cx| {
        let id = view
            .enqueue(
                valid.source.clone(),
                directory.path().join("valid.bin"),
                false,
                cx,
            )
            .unwrap();
        assert!(view.queue.item(id).unwrap().is_queued());
        view.start_queue(cx);
        id
    });
    poll(cx, |_| valid.requested.try_recv().is_ok());
    valid.release.send(()).unwrap();
    poll(cx, |cx| {
        view.read_with(cx, |view, _| view.active.is_none() && !view.queue_running)
    });
    valid.server.join().unwrap();
    view.read_with(cx, |view, _| {
        assert!(view.queue.item(next).unwrap().is_finished())
    });
    assert_eq!(
        std::fs::read(directory.path().join("valid.bin")).unwrap(),
        b"recovered"
    );
    assert!(!directory.path().join("invalid.bin").exists());
}

#[gpui::test]
fn stale_id_updates_cannot_mutate_a_newer_active_item(cx: &mut TestAppContext) {
    let runtime = runtime();
    let directory = tempfile::tempdir().unwrap();
    let (_root, view, cx) = mount(cx, &runtime);
    // Synthetic observations isolate ID filtering without worker scheduling.
    view.update(cx, |view, cx| {
        let old = view
            .queue
            .add(
                "https://example.invalid/old".into(),
                directory.path().join("old.bin"),
            )
            .unwrap();
        view.queue
            .item_mut(old)
            .unwrap()
            .apply_state(TransferState::Cancelled, Instant::now());
        let current = view
            .queue
            .add(
                "https://example.invalid/current".into(),
                directory.path().join("current.bin"),
            )
            .unwrap();
        view.queue
            .item_mut(current)
            .unwrap()
            .apply_state(TransferState::Preparing, Instant::now());
        view.active = Some(current);
        view.queue_running = true;
        view.apply_state(
            old,
            TransferState::Downloading(DownloadProgress {
                downloaded_bytes: 99,
                total_bytes: Some(100),
            }),
            cx,
        );
        view.apply_state(
            old,
            TransferState::Completed(DownloadReceipt {
                destination: directory.path().join("old.bin"),
                downloaded_bytes: 100,
            }),
            cx,
        );
        assert_eq!(view.active, Some(current));
        assert!(view.queue_running);
        assert_eq!(
            view.queue.item(old).unwrap().state,
            Some(TransferState::Cancelled)
        );
        assert_eq!(
            view.queue.item(current).unwrap().state,
            Some(TransferState::Preparing)
        );
        assert_eq!(view.queue.item(current).unwrap().downloaded_bytes, 0);
        view.apply_state(current, TransferState::Cancelled, cx);
        assert!(view.active.is_none());
    });
    draw(cx);
    view.read_with(cx, |view, _| assert!(!view.queue_running));
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
}

#[gpui::test]
fn clipboard_offer_is_opt_in_and_dismissed_candidates_do_not_reopen(cx: &mut TestAppContext) {
    let runtime = runtime();
    let (_root, view, cx) = mount(cx, &runtime);
    let source = "https://example.org/files/My%20Photo%3F.png";
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            assert!(!view.clipboard_enabled);
            view.offer_clipboard(Some(source), window, cx);
            assert!(view.add_form.is_none());
            assert!(view.queue.items.is_empty());
            view.clipboard_enabled = true;
            view.offer_clipboard(Some(source), window, cx);
        });
    });
    draw(cx);
    let form = view.read_with(cx, |view, _| {
        assert!(view.queue.items.is_empty());
        assert!(view.active.is_none());
        assert!(view.transfer.is_none());
        view.add_form.clone().unwrap()
    });
    form.read_with(cx, |form, cx| {
        assert_eq!(form.source.read(cx).value().as_ref(), source);
        assert_eq!(form.filename.read(cx).value().as_ref(), "My Photo_.png");
    });
    form.update(cx, |_, cx| cx.emit(crate::add_download::DismissAdd));
    draw(cx);
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            assert!(view.add_form.is_none());
            view.offer_clipboard(Some(source), window, cx);
            assert!(view.add_form.is_none());
            view.offer_clipboard(Some("https://example.org/new.zip"), window, cx);
            assert!(view.add_form.is_some());
            assert!(view.queue.items.is_empty());
            assert!(view.transfer.is_none());
        });
    });
    draw(cx);
    let next = view.read_with(cx, |view, _| view.add_form.clone().unwrap());
    next.read_with(cx, |form, cx| {
        assert_eq!(form.filename.read(cx).value().as_ref(), "new.zip");
    });
    next.update(cx, |_, cx| cx.emit(crate::add_download::DismissAdd));
    draw(cx);
}

#[gpui::test]
fn url_prefill_suggests_safe_names_synchronously_and_preserves_manual_names(
    cx: &mut TestAppContext,
) {
    let runtime = runtime();
    let directory = tempfile::tempdir().unwrap();
    let (_root, view, cx) = mount(cx, &runtime);
    cx.update(|window, cx| view.update(cx, |view, cx| view.open_add(window, cx)));
    draw(cx);
    let form = view.read_with(cx, |view, _| view.add_form.clone().unwrap());
    cx.update(|window, cx| {
        form.update(cx, |form, cx| {
            form.prefill(
                "https://example.org/CON%3F.txt?token=secret".into(),
                window,
                cx,
            );
            assert_eq!(form.filename.read(cx).value().as_ref(), "CON_.txt");
            form.prefill("https://example.org/automatic.zip".into(), window, cx);
            assert_eq!(form.filename.read(cx).value().as_ref(), "automatic.zip");
            form.filename.update(cx, |input, cx| {
                input.set_value("my custom.zip", window, cx);
            });
            form.prefill("https://example.org/another.png".into(), window, cx);
            assert_eq!(form.filename.read(cx).value().as_ref(), "my custom.zip");
            form.destination.update(cx, |input, cx| {
                input.set_value(
                    directory.path().join("CON?.txt").to_str().unwrap(),
                    window,
                    cx,
                );
            });
            assert!(form.can_submit(cx));
        });
    });
    view.read_with(cx, |view, _| assert!(view.queue.items.is_empty()));
    form.update(cx, |form, cx| form.submit(false, cx));
    draw(cx);
    view.read_with(cx, |view, _| {
        assert!(view.add_form.is_none());
        assert_eq!(view.queue.items.len(), 1);
        let item = &view.queue.items[0];
        assert_eq!(item.source(), "https://example.org/another.png");
        assert_eq!(
            item.destination,
            directory
                .path()
                .join(ulos_core::naming::sanitize_filename("CON?.txt"))
        );
        assert!(item.is_queued());
        assert!(view.active.is_none());
        assert!(view.transfer.is_none());
    });
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
}

#[gpui::test]
fn folder_picker_uses_url_name_and_submission_resanitizes_edited_filename(cx: &mut TestAppContext) {
    let runtime = runtime();
    let directory = tempfile::tempdir().unwrap();
    let (_root, view, cx) = mount(cx, &runtime);
    cx.update(|window, cx| view.update(cx, |view, cx| view.open_add(window, cx)));
    draw(cx);
    let form = view.read_with(cx, |view, _| view.add_form.clone().unwrap());
    cx.update(|window, cx| {
        form.update(cx, |form, cx| {
            form.prefill(
                "https://example.org/photos/My%20Photo.png?signature=hidden".into(),
                window,
                cx,
            );
            assert_eq!(form.filename.read(cx).value().as_ref(), "My Photo.png");
            form.choose_folder(window, cx);
            assert!(form.choosing_path);
            assert!(!form.can_submit(cx));
        });
    });
    cx.simulate_path_prompt_response(|_| Some(vec![directory.path().to_path_buf()]));
    draw(cx);
    form.read_with(cx, |form, cx| {
        assert!(!form.choosing_path);
        assert_eq!(form.filename.read(cx).value().as_ref(), "My Photo.png");
        assert_eq!(
            form.destination.read(cx).value().as_ref(),
            directory.path().join("My Photo.png").to_str().unwrap()
        );
        assert!(form.can_submit(cx));
    });
    let unsafe_name = "../AUX.txt";
    cx.update(|window, cx| {
        form.update(cx, |form, cx| {
            form.filename
                .update(cx, |input, cx| input.set_value(unsafe_name, window, cx));
        });
    });
    form.update(cx, |form, cx| form.submit(false, cx));
    draw(cx);
    view.read_with(cx, |view, _| {
        assert!(view.add_form.is_none());
        assert_eq!(view.queue.items.len(), 1);
        let item = &view.queue.items[0];
        assert_eq!(
            item.destination,
            directory
                .path()
                .join(ulos_core::naming::sanitize_filename(unsafe_name))
        );
        assert!(item.is_queued());
        assert!(view.active.is_none());
        assert!(view.transfer.is_none());
    });
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
}

#[gpui::test]
fn selected_details_redact_signed_source_close_reopen_and_refresh_progress(
    cx: &mut TestAppContext,
) {
    let runtime = runtime();
    let directory = tempfile::tempdir().unwrap();
    let (_root, view, cx) = mount(cx, &runtime);
    let id = view.update(cx, |view, cx| {
        let id = view
            .enqueue(
                "https://example.org/file.zip?signature=hidden#private".into(),
                directory.path().join("file.zip"),
                false,
                cx,
            )
            .unwrap();
        view.queue.selected.insert(id);
        id
    });
    cx.update(|window, cx| view.update(cx, |view, cx| view.open_selected_details(window, cx)));
    draw(cx);
    let details = view.read_with(cx, |view, _| {
        assert!(view.active.is_none());
        assert!(view.transfer.is_none());
        view.details.clone().unwrap()
    });
    details.read_with(cx, |details, _| {
        assert_eq!(details.snapshot.id, id);
        assert_eq!(details.snapshot.source, "https://example.org/file.zip");
        assert_eq!(details.snapshot.status, "Added");
        assert!(details.snapshot.progress.is_none());
    });
    details.update(cx, |_, cx| cx.emit(crate::download_details::CloseDetails));
    draw(cx);
    view.read_with(cx, |view, _| assert!(view.details.is_none()));
    cx.update(|window, cx| view.update(cx, |view, cx| view.open_selected_details(window, cx)));
    draw(cx);
    let reopened = view.read_with(cx, |view, _| view.details.clone().unwrap());
    assert_ne!(details, reopened);
    reopened.update(cx, |details, _| details.show_details = false);
    // Synthetic telemetry isolates passive detail updates from any network worker.
    view.update(cx, |view, cx| {
        view.queue.item_mut(id).unwrap().apply_state(
            TransferState::Downloading(DownloadProgress {
                downloaded_bytes: 7,
                total_bytes: Some(20),
            }),
            Instant::now(),
        );
        view.refresh_details(cx);
        assert!(view.active.is_none());
        assert!(view.transfer.is_none());
    });
    draw(cx);
    reopened.read_with(cx, |details, _| {
        assert!(!details.show_details);
        assert_eq!(details.snapshot.progress.unwrap().downloaded_bytes, 7);
        assert_eq!(details.snapshot.progress.unwrap().total_bytes, Some(20));
        assert_eq!(details.snapshot.source, "https://example.org/file.zip");
    });
    reopened.update(cx, |_, cx| cx.emit(crate::download_details::CloseDetails));
    draw(cx);
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
}

#[gpui::test]
fn details_cancel_event_cancels_real_partial_transfer_and_removes_temporary_file(
    cx: &mut TestAppContext,
) {
    let runtime = runtime();
    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("cancelled.bin");
    let server = fixture(b"cancel from details");
    let (_root, view, cx) = mount(cx, &runtime);
    let id = view.update(cx, |view, cx| {
        view.enqueue(server.source.clone(), destination.clone(), true, cx)
            .unwrap()
    });
    poll(cx, |_| server.requested.try_recv().is_ok());
    poll(cx, |cx| {
        view.read_with(cx, |view, _| {
            view.queue.item(id).unwrap().downloaded_bytes == 1
        })
    });
    cx.update(|window, cx| view.update(cx, |view, cx| view.open_details(id, window, cx)));
    draw(cx);
    let details = view.read_with(cx, |view, _| view.details.clone().unwrap());
    details.read_with(cx, |details, _| {
        assert_eq!(details.snapshot.id, id);
        assert!(details.snapshot.active);
        assert_eq!(details.snapshot.progress.unwrap().downloaded_bytes, 1);
    });
    details.update(cx, |_, cx| {
        cx.emit(crate::download_details::CancelDownload(id))
    });
    view.read_with(cx, |view, _| {
        assert!(view.queue.item(id).unwrap().cancelling)
    });
    poll(cx, |cx| view.read_with(cx, |view, _| view.active.is_none()));
    server.release.send(()).unwrap();
    server.server.join().unwrap();
    view.read_with(cx, |view, _| {
        assert_eq!(
            view.queue.item(id).unwrap().state,
            Some(TransferState::Cancelled)
        );
        assert!(!view.queue.item(id).unwrap().cancelling);
        assert!(view.transfer.is_none());
    });
    details.read_with(cx, |details, _| {
        assert_eq!(details.snapshot.status, "Cancelled");
        assert!(!details.snapshot.active);
        assert!(!details.snapshot.cancelling);
    });
    assert!(!destination.exists());
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
    details.update(cx, |_, cx| cx.emit(crate::download_details::CloseDetails));
    draw(cx);
}
