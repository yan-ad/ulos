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
    let form = view.read_with(cx, |view, _| {
        view.add_windows.values().next().unwrap().upgrade().unwrap()
    });
    let mut child = add_context(&view, cx);
    child.update(|window, cx| {
        form.update(cx, |form, cx| {
            form.prefill("https://example.org/automatic.zip".into(), window, cx);
            form.choose_folder(window, cx);
        })
    });
    child.simulate_path_prompt_response(|_| Some(vec![directory.path().join("previous")]));
    draw(&mut child);
    child.update(|window, cx| {
        form.read(cx)
            .destination
            .clone()
            .update(cx, |input, cx| input.focus(window, cx))
    });
    draw(&mut child);
    child.simulate_keystrokes("secondary-a");
    child.simulate_input(destination.to_str().unwrap());
    draw(&mut child);
    form.read_with(cx, |form, cx| {
        assert_eq!(form.filename.read(cx).value().as_ref(), "manual.zip")
    });
    child.update(|window, cx| {
        form.read(cx)
            .filename
            .clone()
            .update(cx, |input, cx| input.focus(window, cx))
    });
    draw(&mut child);
    child.simulate_keystrokes("secondary-a");
    child.simulate_input("final?.zip");
    draw(&mut child);
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
    (root, manager.unwrap(), cx)
}

fn add_context(view: &Entity<DownloadManager>, cx: &VisualTestContext) -> VisualTestContext {
    let handle = view.read_with(cx, |view, _| {
        view.add_windows.values().next().unwrap().handle
    });
    VisualTestContext::from_window(handle, &cx.cx)
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
    let form = view.read_with(cx, |view, _| {
        view.add_windows.values().next().unwrap().upgrade().unwrap()
    });
    let mut child = add_context(&view, cx);
    let source = form.read_with(cx, |form, _| form.source.clone());
    child.update(|window, cx| source.update(cx, |input, cx| input.focus(window, cx)));
    draw(&mut child);
    child.simulate_input("https://example.invalid/file");
    draw(&mut child);
    view.update(cx, |_, cx| cx.notify());
    draw(&mut child);
    child.simulate_input(".bin");
    draw(&mut child);
    form.read_with(cx, |form, cx| {
        assert_eq!(form.source, source);
        assert_eq!(
            source.read(cx).value().as_ref(),
            "https://example.invalid/file.bin"
        );
        assert!(!form.can_submit(cx));
    });
    child.update(|window, cx| form.update(cx, |form, cx| form.choose_path(window, cx)));
    form.read_with(cx, |form, cx| {
        assert!(form.choosing_path);
        assert!(!form.can_submit(cx));
    });
    child.simulate_new_path_selection(|_| Some(destination.clone()));
    draw(&mut child);
    child.update(|window, cx| form.update(cx, |form, cx| form.choose_path(window, cx)));
    child.simulate_new_path_selection(|_| None);
    draw(&mut child);
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
        assert!(view.add_windows.is_empty());

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
        assert!(
            view.add_windows
                .values()
                .any(|child| child.upgrade().is_some_and(|new| new != form))
        )
    });
    let reopened = view.read_with(cx, |view, _| {
        view.add_windows.values().next().unwrap().upgrade().unwrap()
    });
    reopened.update(cx, |_, cx| cx.emit(crate::add_download::DismissAdd));
    draw(cx);
    view.read_with(cx, |view, _| assert!(view.add_windows.is_empty()));
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
            assert!(view.add_windows.is_empty());
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
        view.add_windows.values().next().unwrap().upgrade().unwrap()
    });
    form.read_with(cx, |form, cx| {
        assert_eq!(form.source.read(cx).value().as_ref(), source);
        assert_eq!(form.filename.read(cx).value().as_ref(), "My Photo_.png");
    });
    form.update(cx, |_, cx| cx.emit(crate::add_download::DismissAdd));
    draw(cx);
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            assert!(view.add_windows.is_empty());
            view.offer_clipboard(Some(source), window, cx);
            assert!(view.add_windows.is_empty());
            view.offer_clipboard(Some("https://example.org/new.zip"), window, cx);
            assert!(!view.add_windows.is_empty());
            assert!(view.queue.items.is_empty());
            assert!(view.transfer.is_none());
        });
    });
    draw(cx);
    let next = view.read_with(cx, |view, _| {
        view.add_windows.values().next().unwrap().upgrade().unwrap()
    });
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
    let form = view.read_with(cx, |view, _| {
        view.add_windows.values().next().unwrap().upgrade().unwrap()
    });
    let mut child = add_context(&view, cx);
    child.update(|window, cx| {
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
        assert!(view.add_windows.is_empty());
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
    let form = view.read_with(cx, |view, _| {
        view.add_windows.values().next().unwrap().upgrade().unwrap()
    });
    let mut child = add_context(&view, cx);
    child.update(|window, cx| {
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
    child.simulate_path_prompt_response(|_| Some(vec![directory.path().to_path_buf()]));
    draw(&mut child);
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
    child.update(|window, cx| {
        form.update(cx, |form, cx| {
            form.filename
                .update(cx, |input, cx| input.set_value(unsafe_name, window, cx));
        });
    });
    form.update(cx, |form, cx| form.submit(false, cx));
    draw(cx);
    view.read_with(cx, |view, _| {
        assert!(view.add_windows.is_empty());
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
        view.detail_windows.get(&id).unwrap().upgrade().unwrap()
    });
    details.read_with(cx, |details, _| {
        assert_eq!(details.snapshot.id, id);
        assert_eq!(details.snapshot.source, "https://example.org/file.zip");
        assert_eq!(details.snapshot.status, "Added");
        assert!(details.snapshot.progress.is_none());
    });
    details.update(cx, |_, cx| cx.emit(crate::download_details::CloseDetails));
    draw(cx);
    view.read_with(cx, |view, _| assert!(view.detail_windows.is_empty()));
    cx.update(|window, cx| view.update(cx, |view, cx| view.open_selected_details(window, cx)));
    draw(cx);
    let reopened = view.read_with(cx, |view, _| {
        view.detail_windows.get(&id).unwrap().upgrade().unwrap()
    });
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
    let details = view.read_with(cx, |view, _| {
        view.detail_windows.get(&id).unwrap().upgrade().unwrap()
    });
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

#[gpui::test]
fn native_add_windows_keep_independent_drafts_and_close_independently(cx: &mut TestAppContext) {
    let runtime = runtime();
    let directory = tempfile::tempdir().unwrap();
    let (_root, view, cx) = mount(cx, &runtime);
    cx.update(|window, cx| view.update(cx, |view, cx| view.open_add(window, cx)));
    let mut first = add_context(&view, cx);
    let first_handle = view.read_with(cx, |view, _| {
        view.add_windows.values().next().unwrap().handle
    });
    let first_form = view.read_with(cx, |view, _| {
        view.add_windows.values().next().unwrap().upgrade().unwrap()
    });
    cx.update(|window, cx| view.update(cx, |view, cx| view.open_add(window, cx)));
    let (second_handle, second_form) = view.read_with(cx, |view, _| {
        assert_eq!(view.add_windows.len(), 2);
        let child = view
            .add_windows
            .values()
            .find(|child| child.handle != first_handle)
            .unwrap();
        (child.handle, child.upgrade().unwrap())
    });
    let mut second = VisualTestContext::from_window(second_handle, &cx.cx);
    assert_eq!(cx.update(|_, cx| cx.windows().len()), 3);
    first.update(|window, cx| {
        first_form.update(cx, |form, cx| {
            form.prefill("https://example.org/first.zip".into(), window, cx);
            form.destination.update(cx, |input, cx| {
                input.set_value(
                    directory.path().join("first.zip").to_str().unwrap(),
                    window,
                    cx,
                )
            });
        })
    });
    second.update(|window, cx| {
        second_form.update(cx, |form, cx| {
            form.prefill("https://example.org/second.zip".into(), window, cx);
            form.destination.update(cx, |input, cx| {
                input.set_value(
                    directory.path().join("second.zip").to_str().unwrap(),
                    window,
                    cx,
                )
            });
        })
    });
    draw(&mut first);
    draw(&mut second);
    first_form.read_with(cx, |form, cx| {
        assert_eq!(form.filename.read(cx).value().as_ref(), "first.zip")
    });
    second_form.read_with(cx, |form, cx| {
        assert_eq!(form.filename.read(cx).value().as_ref(), "second.zip")
    });
    first.update(|window, _| window.remove_window());
    draw(cx);
    view.read_with(cx, |view, _| {
        assert_eq!(view.add_windows.len(), 1);
        assert!(view.add_windows.contains_key(&second_handle.window_id()));
        assert!(view.queue.items.is_empty());
    });
    assert_eq!(cx.update(|_, cx| cx.windows().len()), 2);
    // Real input is routed to the surviving child, not to the manager's search.
    second.update(|window, cx| {
        second_form
            .read(cx)
            .filename
            .clone()
            .update(cx, |input, cx| input.focus(window, cx))
    });
    draw(&mut second);
    second.simulate_keystrokes("secondary-a");
    second.simulate_input("surviving.zip");
    draw(&mut second);
    second_form.read_with(cx, |form, cx| {
        assert_eq!(form.filename.read(cx).value().as_ref(), "surviving.zip")
    });
    second_form.update(cx, |form, cx| form.submit(false, cx));
    draw(cx);
    view.read_with(cx, |view, _| {
        assert!(view.add_windows.is_empty());
        assert_eq!(view.queue.items.len(), 1);
        assert_eq!(
            view.queue.items[0].source(),
            "https://example.org/second.zip"
        );
    });
    assert_eq!(cx.update(|_, cx| cx.windows().len()), 1);
}

#[gpui::test]
fn native_details_are_per_download_and_reopening_reuses_the_window(cx: &mut TestAppContext) {
    let runtime = runtime();
    let directory = tempfile::tempdir().unwrap();
    let (_root, view, cx) = mount(cx, &runtime);
    let (first, second) = queued_pair(
        &view,
        cx,
        "https://example.org/first".into(),
        "https://example.org/second".into(),
        directory.path(),
    );
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.open_details(first, window, cx);
            view.open_details(second, window, cx);
        })
    });
    draw(cx);
    let (first_handle, first_details, second_handle, second_details) =
        view.read_with(cx, |view, _| {
            let a = &view.detail_windows[&first];
            let b = &view.detail_windows[&second];
            (
                a.handle,
                a.upgrade().unwrap(),
                b.handle,
                b.upgrade().unwrap(),
            )
        });
    assert_ne!(first_handle, second_handle);
    assert_eq!(cx.update(|_, cx| cx.windows().len()), 3);
    cx.update(|window, cx| view.update(cx, |view, cx| view.open_details(first, window, cx)));
    view.read_with(cx, |view, _| {
        assert_eq!(view.detail_windows[&first].handle, first_handle)
    });
    assert_eq!(cx.update(|_, cx| cx.windows().len()), 3);
    first_details.update(cx, |_, cx| cx.emit(crate::download_details::CloseDetails));
    draw(cx);
    view.read_with(cx, |view, _| {
        assert_eq!(view.detail_windows.len(), 1);
        assert_eq!(view.detail_windows[&second].handle, second_handle);
        assert!(view.queue.items.iter().all(|item| item.is_queued()));
        assert!(view.active.is_none());
    });
    second_details.read_with(cx, |details, _| assert_eq!(details.snapshot.id, second));
    second_details.update(cx, |_, cx| cx.emit(crate::download_details::CloseDetails));
    draw(cx);
    view.read_with(cx, |view, _| assert!(view.detail_windows.is_empty()));
    assert_eq!(cx.update(|_, cx| cx.windows().len()), 1);
}

#[gpui::test]
fn native_add_owns_manager_after_main_close_and_can_submit(cx: &mut TestAppContext) {
    let runtime = runtime();
    let directory = tempfile::tempdir().unwrap();
    let (root, view, cx) = mount(cx, &runtime);
    cx.update(|window, cx| view.update(cx, |view, cx| view.open_add(window, cx)));
    let mut child = add_context(&view, cx);
    let form = view.read_with(cx, |view, _| {
        view.add_windows.values().next().unwrap().upgrade().unwrap()
    });
    let weak = view.downgrade();
    cx.update(|window, _| window.remove_window());
    drop(root);
    drop(view);
    child.run_until_parked();
    assert!(
        weak.upgrade().is_some(),
        "native root must retain the manager"
    );
    assert_eq!(child.update(|_, cx| cx.windows().len()), 1);
    child.update(|window, cx| {
        form.update(cx, |form, cx| {
            form.prefill("https://example.org/survives.zip".into(), window, cx);
            form.destination.update(cx, |input, cx| {
                input.set_value(
                    directory.path().join("survives.zip").to_str().unwrap(),
                    window,
                    cx,
                )
            });
        })
    });
    draw(&mut child);
    // Holding an observation only across submission allows us to inspect the queue;
    // it is dropped before checking that no root/registry ownership cycle remains.
    let observer = weak.upgrade().unwrap();
    form.update(&mut child, |form, cx| form.submit(false, cx));
    child.run_until_parked();
    observer.read_with(&child, |view, _| {
        assert_eq!(view.queue.items.len(), 1);
        assert_eq!(
            view.queue.items[0].source(),
            "https://example.org/survives.zip"
        );
        assert!(view.queue.items[0].is_queued());
        assert!(view.add_windows.is_empty());
    });
    drop(observer);
    drop(form);
    child.run_until_parked();
    assert!(
        weak.upgrade().is_none(),
        "closed roots must not leak their owner"
    );
    assert_eq!(child.cx.update(|cx| cx.windows().len()), 0);
}

#[gpui::test]
fn native_picker_blocks_dismiss_and_submit_until_cancelled(cx: &mut TestAppContext) {
    let runtime = runtime();
    let (_root, view, cx) = mount(cx, &runtime);
    cx.update(|window, cx| view.update(cx, |view, cx| view.open_add(window, cx)));
    let mut child = add_context(&view, cx);
    let form = view.read_with(cx, |view, _| {
        view.add_windows.values().next().unwrap().upgrade().unwrap()
    });
    child.update(|window, cx| {
        form.update(cx, |form, cx| {
            form.prefill("https://example.org/file.zip".into(), window, cx);
            form.choose_path(window, cx);
        })
    });
    assert!(!child.simulate_close(), "picker must veto native close");
    form.update(cx, |form, cx| {
        cx.emit(crate::add_download::DismissAdd);
        form.submit(false, cx);
    });
    draw(&mut child);
    view.read_with(cx, |view, _| {
        assert_eq!(view.add_windows.len(), 1);
        assert!(view.queue.items.is_empty());
    });
    child.simulate_new_path_selection(|_| None);
    draw(&mut child);
    form.read_with(cx, |form, _| assert!(!form.choosing_path));
    form.update(cx, |_, cx| cx.emit(crate::add_download::DismissAdd));
    draw(cx);
    view.read_with(cx, |view, _| assert!(view.add_windows.is_empty()));
}

#[gpui::test]
fn closing_other_details_does_not_cancel_transfer_and_surviving_child_cancels_after_main_close(
    cx: &mut TestAppContext,
) {
    let runtime = runtime();
    let directory = tempfile::tempdir().unwrap();
    let server = fixture(b"native status cancellation");
    let (root, view, cx) = mount(cx, &runtime);
    let (active, queued) = queued_pair(
        &view,
        cx,
        server.source.clone(),
        "https://example.org/queued".into(),
        directory.path(),
    );
    view.update(cx, |view, cx| view.start_queue(cx));
    poll(cx, |_| server.requested.try_recv().is_ok());
    poll(cx, |cx| {
        view.read_with(cx, |view, _| {
            view.queue.item(active).unwrap().downloaded_bytes == 1
        })
    });
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.open_details(queued, window, cx);
            view.open_details(active, window, cx);
        })
    });
    draw(cx);
    let (passive, telemetry, handle) = view.read_with(cx, |view, _| {
        (
            view.detail_windows[&queued].upgrade().unwrap(),
            view.detail_windows[&active].upgrade().unwrap(),
            view.detail_windows[&active].handle,
        )
    });
    passive.update(cx, |_, cx| cx.emit(crate::download_details::CloseDetails));
    draw(cx);
    view.read_with(cx, |view, _| {
        assert_eq!(view.active, Some(active));
        assert!(!view.queue.item(active).unwrap().cancelling);
        assert!(view.queue.item(queued).unwrap().is_queued());
        assert_eq!(view.detail_windows.len(), 1);
    });
    // Do not advance the queue after cancelling the active transfer.
    view.update(cx, |view, cx| view.stop_queue(cx));
    let mut child = VisualTestContext::from_window(handle, &cx.cx);
    let weak = view.downgrade();
    cx.update(|window, _| window.remove_window());
    drop(root);
    drop(view);
    child.run_until_parked();
    assert!(weak.upgrade().is_some());
    telemetry.update(&mut child, |_, cx| {
        cx.emit(crate::download_details::CancelDownload(active))
    });
    poll(&mut child, |cx| {
        weak.upgrade()
            .unwrap()
            .read_with(cx, |view, _| view.active.is_none())
    });
    server.release.send(()).unwrap();
    server.server.join().unwrap();
    weak.upgrade().unwrap().read_with(&child, |view, _| {
        assert_eq!(
            view.queue.item(active).unwrap().state,
            Some(TransferState::Cancelled)
        );
        assert!(view.queue.item(queued).unwrap().is_queued());
        assert!(view.transfer.is_none());
    });
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
    telemetry.update(&mut child, |_, cx| {
        cx.emit(crate::download_details::CloseDetails)
    });
    child.run_until_parked();
    drop(telemetry);
    drop(passive);
    child.run_until_parked();
    assert!(weak.upgrade().is_none());
}

#[gpui::test]
fn viewport_table_and_native_add_do_not_rebuild_the_entire_queue(cx: &mut TestAppContext) {
    let runtime = runtime();
    let (_root, view, cx) = mount(cx, &runtime);
    view.update(cx, |view, cx| {
        for index in 0..crate::queue::MAX_ITEMS {
            view.queue
                .add(
                    "https://example.invalid/file".into(),
                    format!("test-{index}.bin").into(),
                )
                .unwrap();
        }
        view.sync_table(cx);
        cx.notify();
    });
    draw(cx);
    let (manager_renders, rendered_cells) = view.read_with(cx, |view, cx| {
        (
            view.render_count.clone(),
            view.table.read(cx).delegate().rendered_cells.clone(),
        )
    });
    let cells = rendered_cells.get();
    assert!(
        cells > 0,
        "The mounted DataTable must render viewport cells"
    );
    assert!(
        cells < crate::queue::MAX_ITEMS * 5,
        "Virtualized DataTable must not materialize all rows"
    );
    let renders = manager_renders.get();
    let notifications = std::rc::Rc::new(std::cell::Cell::new(0));
    let count = notifications.clone();
    let _subscription =
        cx.update(|_, cx| cx.observe(&view, move |_, _| count.set(count.get() + 1)));
    cx.update(|window, cx| view.update(cx, |view, cx| view.open_add(window, cx)));
    let mut child = add_context(&view, cx);
    draw(&mut child);
    draw(cx);
    assert!(
        manager_renders.get() <= renders + 1,
        "Opening a native window may redraw focus once, not invalidate continuously"
    );
    assert!(
        rendered_cells.get() - cells < crate::queue::MAX_ITEMS * 5,
        "Native focus changes must not materialize the entire queue"
    );
    assert_eq!(
        notifications.get(),
        0,
        "Opening Add must not notify the manager"
    );
    let form = view.read_with(cx, |view, _| {
        view.add_windows.values().next().unwrap().upgrade().unwrap()
    });
    child.update(|window, cx| {
        form.read(cx)
            .source
            .clone()
            .update(cx, |input, cx| input.focus(window, cx))
    });
    draw(&mut child);
    draw(cx);
    let cells = rendered_cells.get();
    child.simulate_input("https://example.org/independent.zip");
    draw(&mut child);
    draw(cx);
    assert_eq!(
        notifications.get(),
        0,
        "Child input must not notify the manager"
    );
    assert!(
        rendered_cells.get() - cells < crate::queue::MAX_ITEMS * 5,
        "Forced test frames must remain viewport-bounded during child input"
    );
}

#[gpui::test]
fn removing_and_clearing_finished_records_closes_only_matching_native_status_roots(
    cx: &mut TestAppContext,
) {
    let runtime = runtime();
    let directory = tempfile::tempdir().unwrap();
    let (_root, view, cx) = mount(cx, &runtime);
    let (first, second) = queued_pair(
        &view,
        cx,
        "https://example.invalid/first".into(),
        "https://example.invalid/second".into(),
        directory.path(),
    );
    let first_path = directory.path().join("first.bin");
    let third_path = directory.path().join("third.bin");
    std::fs::write(&first_path, b"first").unwrap();
    std::fs::write(&third_path, b"third").unwrap();
    let third = view.update(cx, |view, cx| {
        let third = view
            .enqueue(
                "https://example.invalid/third".into(),
                third_path.clone(),
                false,
                cx,
            )
            .unwrap();
        for (id, destination) in [(first, first_path.clone()), (third, third_path.clone())] {
            view.queue.item_mut(id).unwrap().apply_state(
                TransferState::Completed(DownloadReceipt {
                    destination,
                    downloaded_bytes: 5,
                }),
                Instant::now(),
            );
        }
        third
    });
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            for id in [first, second, third] {
                view.open_details(id, window, cx);
            }
        })
    });
    draw(cx);
    let (second_handle, second_details) = view.read_with(cx, |view, _| {
        assert_eq!(view.detail_windows.len(), 3);
        let child = &view.detail_windows[&second];
        (child.handle, child.upgrade().unwrap())
    });
    assert_eq!(cx.update(|_, cx| cx.windows().len()), 4);
    view.update(cx, |view, cx| {
        view.queue.selected.insert(first);
        view.remove_selected(cx);
    });
    draw(cx);
    view.read_with(cx, |view, _| {
        assert!(view.queue.item(first).is_none());
        assert!(view.queue.item(second).unwrap().is_queued());
        assert!(view.queue.item(third).is_some());
        assert_eq!(view.detail_windows.len(), 2);
        assert!(!view.detail_windows.contains_key(&first));
        assert!(view.detail_windows.contains_key(&third));
        assert_eq!(view.detail_windows[&second].handle, second_handle);
    });
    assert_eq!(cx.update(|_, cx| cx.windows().len()), 3);
    assert_eq!(std::fs::read(&first_path).unwrap(), b"first");
    assert_eq!(std::fs::read(&third_path).unwrap(), b"third");
    view.update(cx, |view, cx| view.clear_finished(cx));
    draw(cx);
    view.read_with(cx, |view, _| {
        assert!(view.queue.item(third).is_none());
        assert_eq!(view.queue.items.len(), 1);
        assert!(view.queue.item(second).unwrap().is_queued());
        assert_eq!(view.detail_windows.len(), 1);
        assert_eq!(view.detail_windows[&second].handle, second_handle);
        assert!(view.active.is_none());
    });
    assert_eq!(cx.update(|_, cx| cx.windows().len()), 2);
    assert_eq!(std::fs::read(&first_path).unwrap(), b"first");
    assert_eq!(std::fs::read(&third_path).unwrap(), b"third");
    second_details.read_with(cx, |details, _| {
        assert_eq!(details.snapshot.id, second);
        assert!(details.show_details);
    });
    second_details.update(cx, |_, cx| cx.emit(crate::download_details::CloseDetails));
    draw(cx);
    view.read_with(cx, |view, _| assert!(view.detail_windows.is_empty()));
    assert_eq!(cx.update(|_, cx| cx.windows().len()), 1);
}

#[gpui::test]
fn native_add_hands_manager_ownership_to_status_when_starting_after_main_close(
    cx: &mut TestAppContext,
) {
    let runtime = runtime();
    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("survives.bin");
    let body = b"native add ownership handoff";
    let server = fixture(body);
    let (root, view, cx) = mount(cx, &runtime);
    cx.update(|window, cx| view.update(cx, |view, cx| view.open_add(window, cx)));
    let mut add = add_context(&view, cx);
    let form = view.read_with(cx, |view, _| {
        view.add_windows.values().next().unwrap().upgrade().unwrap()
    });
    add.update(|window, cx| {
        form.update(cx, |form, cx| {
            form.prefill(server.source.clone(), window, cx);
            form.destination.update(cx, |input, cx| {
                input.set_value(destination.to_str().unwrap(), window, cx)
            });
        })
    });
    draw(&mut add);
    let weak = view.downgrade();
    cx.update(|window, _| window.remove_window());
    drop(root);
    drop(view);
    add.run_until_parked();
    assert!(weak.upgrade().is_some());
    assert_eq!(add.update(|_, cx| cx.windows().len()), 1);
    form.update(&mut add, |form, cx| form.submit(true, cx));
    add.run_until_parked();
    let observer = weak.upgrade().expect("status root must retain the manager");
    let (id, status_handle, details) = observer.read_with(&add, |view, _| {
        assert!(view.add_windows.is_empty());
        assert_eq!(view.detail_windows.len(), 1);
        let id = view.active.expect("submitted transfer must be active");
        let child = &view.detail_windows[&id];
        (id, child.handle, child.upgrade().unwrap())
    });
    assert_eq!(add.cx.update(|cx| cx.windows().len()), 1);
    let mut status = VisualTestContext::from_window(status_handle, &add.cx);
    poll(&mut status, |_| server.requested.try_recv().is_ok());
    server.release.send(()).unwrap();
    poll(&mut status, |cx| {
        observer.read_with(cx, |view, _| {
            matches!(
                view.queue.item(id).unwrap().state,
                Some(TransferState::Completed(_))
            ) && view.active.is_none()
        })
    });
    server.server.join().unwrap();
    assert_eq!(std::fs::read(&destination).unwrap(), body);
    observer.read_with(&status, |view, _| {
        assert!(view.transfer.is_none());
        assert_eq!(view.detail_windows.len(), 1);
    });
    details.read_with(&status, |details, _| {
        assert_eq!(details.snapshot.status, "Finished");
        assert!(!details.snapshot.active);
        assert!(details.show_details);
    });
    drop(form);
    drop(observer);
    status.run_until_parked();
    assert!(weak.upgrade().is_some(), "status root owns the manager");
    details.update(&mut status, |_, cx| {
        cx.emit(crate::download_details::CloseDetails)
    });
    status.run_until_parked();
    drop(details);
    status.run_until_parked();
    assert_eq!(status.cx.update(|cx| cx.windows().len()), 0);
    assert!(weak.upgrade().is_none(), "last root must release its owner");
    assert_eq!(std::fs::read(&destination).unwrap(), body);
}

#[gpui::test]
fn native_menu_toolbar_uses_stock_sizes_and_one_row_at_minimum_width(cx: &mut TestAppContext) {
    let runtime = runtime();
    let (_root, view, cx) = mount(cx, &runtime);
    let _menus = cx.update(|window, cx| {
        crate::native_menu::install(cx, view.downgrade(), window.window_handle())
    });
    cx.simulate_resize(gpui_kit::size(gpui_kit::px(880.), gpui_kit::px(640.)));
    draw(cx);
    #[cfg(target_os = "macos")]
    assert!(cx.debug_bounds("manager-menubar").is_none());
    let add = cx.debug_bounds("add-url").expect("Add URL toolbar button");
    let search = cx.debug_bounds("download-search").expect("search wrapper");
    let options = cx.debug_bounds("options").expect("Options toolbar button");
    #[cfg(target_os = "macos")]
    {
        assert!(
            add.left() >= gpui_kit::px(80.),
            "Keep native traffic lights clear"
        );
        assert!(
            add.top() < gpui_kit::px(18.),
            "No separate titlebar strip above actions"
        );
    }
    assert!((add.center().y - search.center().y).abs() <= gpui_kit::px(1.));
    assert!((add.size.height - options.size.height).abs() <= gpui_kit::px(1.));
    assert!(options.size.height > gpui_kit::px(24.));
    for selector in [
        "start-queue",
        "stop-selected",
        "stop-all",
        "remove-selected",
        "clear-finished",
        "scheduler",
        "grabber",
        "selected-details",
    ] {
        let button = cx.debug_bounds(selector).expect("toolbar control");
        assert!((button.center().y - add.center().y).abs() <= gpui_kit::px(1.));
        assert!(button.origin.x >= add.origin.x);
        assert!(button.origin.x + button.size.width <= search.origin.x);
    }
}

#[gpui::test]
fn native_menu_add_bubbles_from_search_and_options_waits_for_children(cx: &mut TestAppContext) {
    let runtime = runtime();
    let (_root, view, cx) = mount(cx, &runtime);
    let _menus = cx.update(|window, cx| {
        crate::native_menu::install(cx, view.downgrade(), window.window_handle())
    });
    cx.update(|window, cx| {
        window.activate_window();
        view.read(cx)
            .search
            .clone()
            .update(cx, |input, cx| input.focus(window, cx));
    });
    draw(cx);
    cx.dispatch_action(crate::native_menu::NewDownload);
    draw(cx);
    cx.dispatch_action(crate::native_menu::NewDownload);
    draw(cx);
    let handles = view.read_with(cx, |view, _| {
        assert_eq!(view.add_windows.len(), 2);
        view.add_windows
            .values()
            .map(|child| child.handle)
            .collect::<Vec<_>>()
    });
    assert_ne!(handles[0].window_id(), handles[1].window_id());
    cx.dispatch_action(crate::native_menu::OpenOptions);
    draw(cx);
    view.read_with(cx, |view, _| assert!(!view.options_open));
    for handle in handles {
        let mut child = VisualTestContext::from_window(handle, &cx.cx);
        child.update(|window, _| window.remove_window());
    }
    draw(cx);
    cx.update(|window, _| window.activate_window());
    cx.dispatch_action(crate::native_menu::OpenOptions);
    draw(cx);
    view.read_with(cx, |view, cx| {
        assert!(view.options_open);
        assert!(!view.menu_state(cx).can_options);
    });
}

#[gpui::test]
fn native_menu_add_routes_to_surviving_child_after_main_closes(cx: &mut TestAppContext) {
    let runtime = runtime();
    let (root, view, cx) = mount(cx, &runtime);
    let _menus = cx.update(|window, cx| {
        crate::native_menu::install(cx, view.downgrade(), window.window_handle())
    });
    cx.dispatch_action(crate::native_menu::NewDownload);
    draw(cx);
    let mut child = add_context(&view, cx);
    let first_id = child.update(|window, _| window.window_handle().window_id());
    let owner = view.downgrade();
    cx.update(|window, _| window.remove_window());
    drop(root);
    drop(view);
    child.run_until_parked();
    child.update(|window, _| window.activate_window());
    draw(&mut child);
    child.dispatch_action(crate::native_menu::NewDownload);
    draw(&mut child);
    owner.upgrade().unwrap().read_with(&child, |view, cx| {
        assert_eq!(view.add_windows.len(), 2);
        assert!(view.add_windows.contains_key(&first_id));
        assert_eq!(cx.windows().len(), 2);
        assert!(!view.menu_state(cx).can_options);
    });
    child.dispatch_action(crate::native_menu::OpenOptions);
    draw(&mut child);
    owner
        .upgrade()
        .unwrap()
        .read_with(&child, |view, _| assert!(!view.options_open));
}

#[gpui::test]
fn native_menu_start_guard_and_real_worker_stop_use_registered_actions(cx: &mut TestAppContext) {
    let runtime = runtime();
    let directory = tempfile::tempdir().unwrap();
    let (_root, view, cx) = mount(cx, &runtime);
    let _menus = cx.update(|window, cx| {
        crate::native_menu::install(cx, view.downgrade(), window.window_handle())
    });
    cx.dispatch_action(crate::native_menu::StartDownloads);
    draw(cx);
    view.read_with(cx, |view, cx| {
        assert!(!view.menu_state(cx).can_start);
        assert!(!view.queue_running);
        assert!(view.active.is_none());
        assert!(view.transfer.is_none());
    });
    let server = fixture(b"native menu worker payload");
    let id = view.update(cx, |view, cx| {
        view.enqueue(
            server.source.clone(),
            directory.path().join("native.bin"),
            false,
            cx,
        )
        .unwrap()
    });
    cx.update(|window, cx| {
        window.activate_window();
        view.read(cx)
            .search
            .clone()
            .update(cx, |input, cx| input.focus(window, cx));
    });
    draw(cx);
    cx.dispatch_action(crate::native_menu::StartDownloads);
    poll(cx, |_| server.requested.try_recv().is_ok());
    view.read_with(cx, |view, _| assert_eq!(view.active, Some(id)));
    cx.dispatch_action(crate::native_menu::StartDownloads);
    cx.dispatch_action(crate::native_menu::StopDownloads);
    server.release.send(()).unwrap();
    poll(cx, |cx| {
        view.read_with(cx, |view, _| view.active.is_none() && !view.queue_running)
    });
    server.server.join().unwrap();
    view.read_with(cx, |view, _| assert_eq!(view.queue.items.len(), 1));
}

#[gpui::test]
fn native_menu_filter_clear_and_disabled_remove_preserve_downloaded_files(cx: &mut TestAppContext) {
    let runtime = runtime();
    let directory = tempfile::tempdir().unwrap();
    let saved = directory.path().join("saved.txt");
    std::fs::write(&saved, b"keep these bytes").unwrap();
    let (_root, view, cx) = mount(cx, &runtime);
    let _menus = cx.update(|window, cx| {
        crate::native_menu::install(cx, view.downgrade(), window.window_handle())
    });
    let (finished, active) = view.update(cx, |view, cx| {
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
        let active = view
            .enqueue(
                "https://example.invalid/active".into(),
                directory.path().join("active.bin"),
                false,
                cx,
            )
            .unwrap();
        view.queue
            .item_mut(active)
            .unwrap()
            .apply_state(TransferState::Preparing, Instant::now());
        view.queue.selected.insert(active);
        cx.notify();
        (finished, active)
    });
    cx.update(|window, cx| {
        view.read(cx)
            .search
            .clone()
            .update(cx, |input, cx| input.focus(window, cx))
    });
    draw(cx);
    cx.dispatch_action(crate::native_menu::RemoveDownloads);
    draw(cx);
    view.read_with(cx, |view, cx| {
        assert!(!view.menu_state(cx).removable);
        assert!(view.queue.item(active).is_some());
        assert_eq!(view.queue.items.len(), 2);
    });
    cx.dispatch_action(crate::native_menu::ShowFinishedDownloads);
    draw(cx);
    view.read_with(cx, |view, cx| {
        assert_eq!(view.menu_state(cx).filter, Filter::Finished);
        assert_eq!(
            view.queue
                .visible()
                .iter()
                .map(|item| item.id)
                .collect::<Vec<_>>(),
            vec![finished]
        );
    });
    cx.dispatch_action(crate::native_menu::ClearFinishedDownloads);
    cx.dispatch_action(crate::native_menu::ShowAllDownloads);
    draw(cx);
    view.read_with(cx, |view, cx| {
        assert_eq!(view.menu_state(cx).filter, Filter::All);
        assert!(view.queue.item(finished).is_none());
        assert!(view.queue.item(active).is_some());
        assert_eq!(view.queue.visible().len(), 1);
    });
    assert_eq!(std::fs::read(saved).unwrap(), b"keep these bytes");
}

#[gpui::test]
fn native_menu_theme_actions_apply_stock_theme_and_checked_state(cx: &mut TestAppContext) {
    use gpui_kit::component::{Theme, ThemeColor};
    let runtime = runtime();
    let (_root, view, cx) = mount(cx, &runtime);
    let _menus = cx.update(|window, cx| {
        crate::native_menu::install(cx, view.downgrade(), window.window_handle())
    });
    cx.dispatch_action(crate::native_menu::UseDarkTheme);
    draw(cx);
    view.read_with(cx, |view, cx| {
        assert!(view.menu_state(cx).dark);
        assert!(Theme::global(cx).is_dark());
        assert_eq!(
            Theme::global(cx).colors.background,
            ThemeColor::dark().background
        );
    });
    cx.dispatch_action(crate::native_menu::UseLightTheme);
    draw(cx);
    view.read_with(cx, |view, cx| {
        assert!(!view.menu_state(cx).dark);
        assert!(!Theme::global(cx).is_dark());
        assert_eq!(
            Theme::global(cx).colors.background,
            ThemeColor::light().background
        );
    });
}

#[gpui::test]
fn native_menu_new_download_shortcut_bubbles_from_focused_inputs(cx: &mut TestAppContext) {
    let runtime = runtime();
    let (_root, view, cx) = mount(cx, &runtime);
    let _menus = cx.update(|window, cx| {
        crate::native_menu::install(cx, view.downgrade(), window.window_handle())
    });
    cx.update(|window, cx| {
        window.activate_window();
        view.read(cx)
            .search
            .clone()
            .update(cx, |input, cx| input.focus(window, cx));
    });
    draw(cx);
    cx.simulate_keystrokes("secondary-n");
    draw(cx);
    let form = view.read_with(cx, |view, _| {
        assert_eq!(view.add_windows.len(), 1);
        view.add_windows.values().next().unwrap().upgrade().unwrap()
    });
    let mut child = add_context(&view, cx);
    child.update(|window, cx| {
        window.activate_window();
        form.read(cx)
            .source
            .clone()
            .update(cx, |input, cx| input.focus(window, cx));
    });
    draw(&mut child);
    child.simulate_input("https://example.org/native-shortcut.zip");
    draw(&mut child);
    child.simulate_keystrokes("secondary-n");
    draw(&mut child);
    let handles = view.read_with(&child, |view, cx| {
        assert_eq!(view.add_windows.len(), 2);
        assert_eq!(view.search.read(cx).value().as_ref(), "");
        assert!(view.queue.items.is_empty());
        view.add_windows
            .values()
            .map(|child| child.handle)
            .collect::<Vec<_>>()
    });
    form.read_with(&child, |form, cx| {
        assert_eq!(
            form.source.read(cx).value().as_ref(),
            "https://example.org/native-shortcut.zip"
        );
    });
    assert_ne!(handles[0].window_id(), handles[1].window_id());
    child.dispatch_action(crate::native_menu::OpenOptions);
    draw(&mut child);
    view.read_with(&child, |view, _| assert!(!view.options_open));
    for handle in handles {
        let mut child = VisualTestContext::from_window(handle, &cx.cx);
        child.update(|window, _| window.remove_window());
    }
    draw(cx);
    view.read_with(cx, |view, _| assert!(view.add_windows.is_empty()));
}

#[gpui::test]
fn native_menu_edit_actions_use_the_focused_search_input(cx: &mut TestAppContext) {
    let runtime = runtime();
    let (_root, view, cx) = mount(cx, &runtime);
    let _menus = cx.update(|window, cx| {
        crate::native_menu::install(cx, view.downgrade(), window.window_handle())
    });
    cx.update(|window, cx| {
        window.activate_window();
        view.read(cx)
            .search
            .clone()
            .update(cx, |input, cx| input.focus(window, cx));
    });
    draw(cx);
    cx.simulate_input("alpha");
    draw(cx);
    view.read_with(cx, |view, cx| {
        assert_eq!(view.search.read(cx).value().as_ref(), "alpha");
        assert_eq!(view.queue.search, "alpha");
    });
    cx.dispatch_action(gpui_kit::component::input::SelectAll);
    draw(cx);
    cx.dispatch_action(gpui_kit::component::input::Cut);
    draw(cx);
    view.read_with(cx, |view, cx| {
        assert_eq!(view.search.read(cx).value().as_ref(), "");
        assert_eq!(view.queue.search, "");
        assert_eq!(
            cx.read_from_clipboard().unwrap().text().as_deref(),
            Some("alpha")
        );
    });
    cx.dispatch_action(gpui_kit::component::input::Paste);
    draw(cx);
    view.read_with(cx, |view, cx| {
        assert_eq!(view.search.read(cx).value().as_ref(), "alpha");
        assert_eq!(view.queue.search, "alpha");
        assert!(view.queue.items.is_empty());
        assert!(view.add_windows.is_empty());
    });
}

#[gpui::test]
fn native_menu_state_events_do_not_notify_the_manager_view(cx: &mut TestAppContext) {
    let runtime = runtime();
    let (_root, view, cx) = mount(cx, &runtime);
    let _menus = cx.update(|window, cx| {
        crate::native_menu::install(cx, view.downgrade(), window.window_handle())
    });
    let states = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let observed_states = states.clone();
    let _events = cx.update(|_, cx| {
        cx.subscribe(
            &view,
            move |manager, _: &crate::manager::MenuStateChanged, cx| {
                observed_states
                    .borrow_mut()
                    .push(manager.read(cx).menu_state(cx));
            },
        )
    });
    let notifications = std::rc::Rc::new(std::cell::Cell::new(0));
    let count = notifications.clone();
    let _observer = cx.update(|_, cx| cx.observe(&view, move |_, _| count.set(count.get() + 1)));
    view.read_with(cx, |view, cx| assert!(view.menu_state(cx).can_options));
    cx.dispatch_action(crate::native_menu::NewDownload);
    draw(cx);
    assert_eq!(states.borrow().len(), 1);
    assert!(!states.borrow()[0].can_options);
    assert!(states.borrow()[0].can_add);
    cx.dispatch_action(crate::native_menu::NewDownload);
    draw(cx);
    assert_eq!(states.borrow().len(), 2);
    assert!(!states.borrow()[1].can_options);
    assert_eq!(notifications.get(), 0);
    let handles = view.read_with(cx, |view, _| {
        assert_eq!(view.add_windows.len(), 2);
        view.add_windows
            .values()
            .map(|child| child.handle)
            .collect::<Vec<_>>()
    });
    let mut first = VisualTestContext::from_window(handles[0], &cx.cx);
    first.update(|window, _| window.remove_window());
    draw(cx);
    view.read_with(cx, |view, cx| {
        assert_eq!(view.add_windows.len(), 1);
        assert!(!view.menu_state(cx).can_options);
    });
    let mut second = VisualTestContext::from_window(handles[1], &cx.cx);
    second.update(|window, _| window.remove_window());
    draw(cx);
    view.read_with(cx, |view, cx| {
        assert!(view.add_windows.is_empty());
        assert!(view.menu_state(cx).can_options);
    });
    assert_eq!(states.borrow().len(), 2);
    assert_eq!(notifications.get(), 0);
}
