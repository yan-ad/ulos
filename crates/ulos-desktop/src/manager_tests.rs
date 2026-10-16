use std::{
    io::{BufRead, BufReader, Write},
    net::TcpListener,
    sync::mpsc::{self, Receiver, Sender, TryRecvError},
    thread,
    time::{Duration, Instant},
};

use gpui_kit::{
    AppContext, Entity, Focusable, TestAppContext, VisualTestContext, component::Root, px,
};
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
fn running_named_queues_share_one_slot_and_exhausted_state_refreshes(cx: &mut TestAppContext) {
    let runtime = runtime();
    let directory = tempfile::tempdir().unwrap();
    let first = fixture(b"first queue bytes");
    let second = fixture(b"second queue bytes");
    let (_root, view, cx) = mount(cx, &runtime);
    let (a, b) = queued_pair(
        &view,
        cx,
        first.source.clone(),
        second.source.clone(),
        directory.path(),
    );
    let (one, two) = view.update(cx, |view, cx| {
        let one = view.scheduler.create("First queue").unwrap();
        let two = view.scheduler.create("Second queue").unwrap();
        view.queue.assign_queue(&[a], one).unwrap();
        view.queue.assign_queue(&[b], two).unwrap();
        view.scheduler.selected = two;
        view.scheduler.queue_mut(one).unwrap().running = true;
        view.scheduler.queue_mut(two).unwrap().running = true;
        view.advance(cx);
        assert_eq!(view.active, Some(a));
        (one, two)
    });
    cx.update(|window, cx| view.update(cx, |view, cx| view.open_scheduler(window, cx)));
    draw(cx);
    let content = view.read_with(cx, |view, _| {
        view.scheduler_window.as_ref().unwrap().upgrade().unwrap()
    });
    poll(cx, |_| first.requested.try_recv().is_ok());
    assert!(second.requested.try_recv().is_err());
    view.read_with(cx, |view, _| assert_eq!(view.queue.active_count(), 1));
    first.release.send(()).unwrap();
    poll(cx, |_| second.requested.try_recv().is_ok());
    view.read_with(cx, |view, _| {
        assert_eq!(view.active, Some(b));
        assert_eq!(view.queue.active_count(), 1);
        assert!(!view.scheduler.queue(one).unwrap().running);
    });
    second.release.send(()).unwrap();
    poll(cx, |cx| {
        view.read_with(cx, |view, _| {
            view.active.is_none() && !view.scheduler.any_running()
        })
    });
    draw(cx);
    content.read_with(cx, |content, _| {
        assert!(
            content
                .snapshot
                .queues
                .iter()
                .filter(|queue| queue.id == one || queue.id == two)
                .all(|queue| !queue.running)
        );
    });
    first.server.join().unwrap();
    second.server.join().unwrap();
    assert_eq!(
        std::fs::read(directory.path().join("first.bin")).unwrap(),
        b"first queue bytes"
    );
    assert_eq!(
        std::fs::read(directory.path().join("second.bin")).unwrap(),
        b"second queue bytes"
    );
}

#[gpui::test]
fn scheduler_parent_rechecks_membership_and_expired_schedules_refresh_independently(
    cx: &mut TestAppContext,
) {
    use crate::{
        scheduler::{DEFAULT_QUEUE, Schedule, ScheduleSpec},
        scheduler_window::SchedulerRequest,
    };
    use chrono::{Local, NaiveDate, TimeZone};
    let runtime = runtime();
    let directory = tempfile::tempdir().unwrap();
    let (_root, view, cx) = mount(cx, &runtime);
    let id = view.update(cx, |view, cx| {
        view.enqueue(
            "https://example.invalid/file".into(),
            directory.path().join("file.bin"),
            false,
            cx,
        )
        .unwrap()
    });
    let _menus = cx.update(|window, cx| {
        crate::native_menu::install(cx, view.downgrade(), window.window_handle())
    });
    let button = cx
        .debug_bounds("scheduler")
        .expect("enabled Scheduler toolbar action");
    cx.simulate_click(button.center(), gpui_kit::Modifiers::default());
    draw(cx);
    let content = view.read_with(cx, |view, _| {
        view.scheduler_window.as_ref().unwrap().upgrade().unwrap()
    });
    let at = |hour| {
        Local
            .from_local_datetime(
                &NaiveDate::from_ymd_opt(2030, 6, 3)
                    .unwrap()
                    .and_hms_opt(hour, 0, 0)
                    .unwrap(),
            )
            .earliest()
            .unwrap()
    };
    view.update(cx, |view, cx| {
        let second = view.scheduler.create("Second").unwrap();
        let third = view.scheduler.create("Third").unwrap();
        for (queue, start, stop) in [(DEFAULT_QUEUE, 10, 11), (second, 10, 11), (third, 15, 16)] {
            let schedule = Schedule::new(
                ScheduleSpec::Once {
                    date: at(9).date_naive(),
                    start: at(start).time(),
                    stop: Some(at(stop).time()),
                },
                at(9),
            )
            .unwrap();
            view.scheduler
                .set_schedule(queue, Some(schedule), at(9))
                .unwrap();
        }
        view.scheduler_tick(at(12), cx);
        assert!(
            !view
                .scheduler
                .queue(DEFAULT_QUEUE)
                .unwrap()
                .schedule
                .as_ref()
                .unwrap()
                .enabled
        );
        assert!(view.scheduler.has_enabled_schedule());
        view.arm_shutdown(id, cx);
        assert!(!view.queue.item(id).unwrap().completion.is_armed());
        let old_draft = view.scheduler_snapshot();
        view.scheduler.selected = second;
        view.scheduler_request(&SchedulerRequest::DeleteQueue(third), &old_draft, cx);
        assert!(
            view.scheduler.queue(third).is_some(),
            "stale drafts cannot delete another queue"
        );
        view.scheduler.selected = DEFAULT_QUEUE;
        view.scheduler_error = None;
        view.refresh_scheduler(cx);
    });
    draw(cx);
    content.read_with(cx, |content, _| {
        assert!(
            content
                .snapshot
                .queues
                .iter()
                .find(|queue| queue.id == DEFAULT_QUEUE)
                .unwrap()
                .schedule
                .is_none()
        );
        assert!(
            content
                .snapshot
                .queues
                .iter()
                .any(|queue| queue.schedule.is_some())
        );
    });
    view.update(cx, |view, cx| {
        let target = view.scheduler.create("Delete me").unwrap();
        view.scheduler.selected = target;
        view.queue.assign_queue(&[id], target).unwrap();
        let draft = view.scheduler_snapshot();
        view.scheduler_request(&SchedulerRequest::DeleteQueue(target), &draft, cx);
        assert!(view.scheduler.queue(target).is_none());
        assert_eq!(view.queue.item(id).unwrap().queue_id, DEFAULT_QUEUE);
        assert!(view.queue.item(id).unwrap().is_queued());
        let draft = view.scheduler_snapshot();
        view.scheduler_request(&SchedulerRequest::DeleteQueue(DEFAULT_QUEUE), &draft, cx);
        assert!(view.scheduler.queue(DEFAULT_QUEUE).is_some());
        assert!(view.scheduler_error.is_some());
    });
    assert!(!directory.path().join("file.bin").exists());
}

#[gpui::test]
fn compact_table_checkboxes_toggle_both_ways_and_links_do_not_select(cx: &mut TestAppContext) {
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
    draw(cx);
    assert_eq!(first, 1);
    for selected in [true, false, true, false] {
        let checkbox = cx.debug_bounds("select-download-1").unwrap();
        cx.simulate_click(checkbox.center(), gpui_kit::Modifiers::default());
        draw(cx);
        view.read_with(cx, |view, cx| {
            assert_eq!(view.queue.selected.contains(&first), selected);
            assert!(!view.queue.selected.contains(&second));
            assert_eq!(
                view.table
                    .read(cx)
                    .delegate()
                    .rows
                    .iter()
                    .find(|row| row.id == first)
                    .unwrap()
                    .selected,
                selected
            );
        });
    }
    for selected in [true, false] {
        let checkbox = cx.debug_bounds("select-visible").unwrap();
        cx.simulate_click(checkbox.center(), gpui_kit::Modifiers::default());
        draw(cx);
        view.read_with(cx, |view, _| {
            assert_eq!(view.queue.selected.len(), if selected { 2 } else { 0 })
        });
    }
    let table_bounds = cx.debug_bounds("download-table").unwrap();
    let first_checkbox = cx.debug_bounds("select-download-1").unwrap();
    assert!(first_checkbox.center().y - table_bounds.top() < px(60.));
    let name = cx.debug_bounds("download-name-1").unwrap();
    assert!(
        name.size.width < px(300.),
        "filename is content, not a full-width Button"
    );
    cx.simulate_click(name.center(), gpui_kit::Modifiers::default());
    draw(cx);
    view.read_with(cx, |view, _| {
        assert!(view.detail_windows.contains_key(&first));
        assert!(view.queue.selected.is_empty());
        assert!(view.active.is_none());
    });
    cx.update(|window, cx| {
        view.read(cx).table.clone().update(cx, |table, cx| {
            table.set_selected_row(1, cx);
            table.focus_handle(cx).focus(window, cx);
        });
    });
    draw(cx);
    cx.simulate_keystrokes("enter");
    draw(cx);
    view.read_with(cx, |view, _| {
        assert!(view.detail_windows.contains_key(&second));
        assert!(view.queue.selected.is_empty());
    });
}

#[gpui::test]
fn named_queue_schedule_runs_reordered_work_and_stop_preserves_active_transfer(
    cx: &mut TestAppContext,
) {
    use crate::{
        scheduler::{Schedule, ScheduleSpec},
        scheduler_window::SchedulerRequest,
    };
    use chrono::{Local, NaiveDate, TimeZone};
    let runtime = runtime();
    let directory = tempfile::tempdir().unwrap();
    let first = fixture(b"first scheduled payload");
    let second = fixture(b"second scheduled payload");
    let (_root, view, cx) = mount(cx, &runtime);
    let (a, b) = queued_pair(
        &view,
        cx,
        first.source.clone(),
        second.source.clone(),
        directory.path(),
    );
    let at = |hour, minute| {
        Local
            .from_local_datetime(
                &NaiveDate::from_ymd_opt(2030, 6, 3)
                    .unwrap()
                    .and_hms_opt(hour, minute, 0)
                    .unwrap(),
            )
            .earliest()
            .unwrap()
    };
    let queue_id = view.update(cx, |view, cx| {
        let queue_id = view.scheduler.create("Night downloads").unwrap();
        view.scheduler.selected = queue_id;
        view.queue.assign_queue(&[a, b], queue_id).unwrap();
        assert!(view.queue.move_queued(b, true));
        view.queue.set_description(b, "Nightly archive").unwrap();
        let schedule = Schedule::new(
            ScheduleSpec::Once {
                date: at(9, 0).date_naive(),
                start: at(10, 0).time(),
                stop: Some(at(11, 0).time()),
            },
            at(9, 0),
        )
        .unwrap();
        view.scheduler
            .set_schedule(queue_id, Some(schedule), at(9, 0))
            .unwrap();
        view.scheduler_tick(at(9, 30), cx);
        assert!(view.active.is_none());
        view.scheduler_tick(at(10, 0), cx);
        assert_eq!(view.active, Some(b));
        assert!(view.queue.item(b).unwrap().last_try.is_some());
        view.sync_table(cx);
        let row = view
            .table
            .read(cx)
            .delegate()
            .rows
            .iter()
            .find(|row| row.id == b)
            .unwrap();
        assert_eq!(row.queue_name, "Night downloads");
        assert_eq!(row.description, "Nightly archive");
        queue_id
    });
    poll(cx, |_| second.requested.try_recv().is_ok());
    view.update(cx, |view, cx| {
        let snapshot = view.scheduler_snapshot();
        view.scheduler_request(&SchedulerRequest::AssignSelected(queue_id), &snapshot, cx);
        assert!(view.scheduler_error.is_some());
        view.scheduler_tick(at(11, 0), cx);
        assert_eq!(view.active, Some(b));
        assert!(!view.queue.item(b).unwrap().cancelling);
        assert!(!view.scheduler.queue(queue_id).unwrap().running);
    });
    second.release.send(()).unwrap();
    poll(cx, |cx| view.read_with(cx, |view, _| view.active.is_none()));
    assert!(first.requested.try_recv().is_err());
    view.read_with(cx, |view, _| {
        assert!(view.queue.item(b).unwrap().is_finished());
        assert!(view.queue.item(a).unwrap().is_queued());
        assert_eq!(view.queue.item(a).unwrap().last_try, None);
    });
    view.update(cx, |view, cx| {
        let snapshot = view.scheduler_snapshot();
        view.scheduler_request(&SchedulerRequest::StartQueue(queue_id), &snapshot, cx);
    });
    poll(cx, |_| first.requested.try_recv().is_ok());
    first.release.send(()).unwrap();
    poll(cx, |cx| view.read_with(cx, |view, _| view.active.is_none()));
    first.server.join().unwrap();
    second.server.join().unwrap();
    assert_eq!(
        std::fs::read(directory.path().join("second.bin")).unwrap(),
        b"second scheduled payload"
    );
}

#[gpui::test]
fn scheduler_window_is_reused_blocks_clipboard_and_survives_main_close(cx: &mut TestAppContext) {
    use crate::scheduler_window::SchedulerRequest;
    let runtime = runtime();
    let (root, view, cx) = mount(cx, &runtime);
    cx.update(|window, cx| view.update(cx, |view, cx| view.open_scheduler(window, cx)));
    draw(cx);
    let (handle, content) = view.read_with(cx, |view, _| {
        let child = view.scheduler_window.as_ref().unwrap();
        (child.handle, child.upgrade().unwrap())
    });
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.open_scheduler(window, cx);
            view.clipboard_enabled = true;
            view.offer_clipboard(Some("https://example.invalid/copied"), window, cx);
            assert!(view.add_windows.is_empty());
            assert_eq!(view.scheduler_window.as_ref().unwrap().handle, handle);
        })
    });
    let mut child = VisualTestContext::from_window(handle, &cx.cx);
    child.dispatch_action(crate::native_menu::OpenScheduler);
    draw(&mut child);
    view.read_with(&child, |view, _| {
        assert_eq!(view.scheduler_window.as_ref().unwrap().handle, handle)
    });
    child.update(|_, cx| {
        content.update(cx, |_, cx| {
            cx.emit(SchedulerRequest::CreateQueue("Night".into()))
        })
    });
    draw(&mut child);
    view.read_with(&child, |view, _| assert_eq!(view.scheduler.queues.len(), 2));
    cx.update(|window, _| window.remove_window());
    draw(&mut child);
    child.update(|_, cx| {
        content.update(cx, |_, cx| {
            cx.emit(SchedulerRequest::CreateQueue("Weekend".into()))
        })
    });
    draw(&mut child);
    view.read_with(&child, |view, _| assert_eq!(view.scheduler.queues.len(), 3));
    child.update(|_, cx| content.update(cx, |_, cx| cx.emit(SchedulerRequest::Close)));
    child.run_until_parked();
    drop(root);
}

#[gpui::test]
fn manual_save_path_overrides_a_previously_selected_path(cx: &mut TestAppContext) {
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
            form.choose_path(window, cx);
        })
    });
    child.simulate_new_path_selection(|_| Some(directory.path().join("previous.zip")));
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
    child.update(|window, cx| {
        form.update(cx, |form, cx| {
            form.prefill("https://example.org/another.png".into(), window, cx);
            assert_eq!(
                form.destination.read(cx).value().as_ref(),
                destination.to_str().unwrap()
            );
        })
    });
    poll(&mut child, |cx| {
        form.read_with(cx, |form, cx| form.can_submit(cx))
    });
    form.update(cx, |form, cx| form.submit(false, cx));
    draw(cx);
    view.read_with(cx, |view, _| {
        assert_eq!(view.queue.items[0].destination, destination);
        assert_eq!(
            view.queue.items[0].source(),
            "https://example.org/another.png"
        );
    });
    assert!(!destination.exists());
}

pub(super) fn mount<'a>(
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

pub(super) fn draw(cx: &mut VisualTestContext) {
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
        let ids = view.queue.selected.iter().copied().collect::<Vec<_>>();
        view.finish_record_removal(&ids, cx);
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
fn cancelling_record_removal_and_clear_finished_preserves_records_and_files(
    cx: &mut TestAppContext,
) {
    let runtime = runtime();
    let directory = tempfile::tempdir().unwrap();
    let saved = directory.path().join("saved.bin");
    std::fs::write(&saved, b"preserve me").unwrap();
    let (_root, view, cx) = mount(cx, &runtime);
    let id = view.update(cx, |view, cx| {
        let id = view
            .enqueue(
                "https://example.invalid/saved.bin".into(),
                saved.clone(),
                false,
                cx,
            )
            .unwrap();
        view.queue.item_mut(id).unwrap().apply_state(
            TransferState::Completed(DownloadReceipt {
                destination: saved.clone(),
                downloaded_bytes: 11,
            }),
            Instant::now(),
        );
        view.queue.selected.insert(id);
        id
    });
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.open_details(id, window, cx);
            view.remove_selected(window, cx);
        })
    });
    draw(cx);
    view.read_with(cx, |view, _| assert!(view.queue.item(id).is_some()));
    let cancel = cx.debug_bounds("cancel-removal").unwrap().center();
    cx.simulate_click(cancel, gpui::Modifiers::default());
    draw(cx);
    view.read_with(cx, |view, _| {
        assert!(view.queue.item(id).unwrap().is_finished());
        assert!(view.queue.selected.contains(&id));
        assert!(view.detail_windows.contains_key(&id));
    });
    cx.update(|window, cx| view.update(cx, |view, cx| view.clear_finished(window, cx)));
    draw(cx);
    let cancel = cx.debug_bounds("cancel-removal").unwrap().center();
    cx.simulate_click(cancel, gpui::Modifiers::default());
    draw(cx);
    view.read_with(cx, |view, _| {
        assert_eq!(view.queue.items.len(), 1);
        assert!(view.queue.item(id).unwrap().is_finished());
        assert!(view.detail_windows.contains_key(&id));
    });
    assert_eq!(std::fs::read(&saved).unwrap(), b"preserve me");
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
    poll(&mut child, |cx| {
        form.read_with(cx, |form, cx| form.can_submit(cx))
    });
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
        assert!(form.destination.read(cx).value().is_empty());
        assert!(!form.can_submit(cx));
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
        assert_eq!(
            form.source.read(cx).value().as_ref(),
            "https://example.org/new.zip"
        );
        assert!(form.destination.read(cx).value().is_empty());
    });
    next.update(cx, |_, cx| cx.emit(crate::add_download::DismissAdd));
    draw(cx);
}

#[gpui::test]
fn url_prefill_preserves_manual_full_path_and_submission_sanitizes_only_basename(
    cx: &mut TestAppContext,
) {
    let runtime = runtime();
    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("CON?.txt");
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
            // A manager without platform paths cannot invent a save directory.
            assert!(form.destination.read(cx).value().is_empty());
            assert!(!form.can_submit(cx));
            form.destination.update(cx, |input, cx| {
                input.set_value(destination.to_str().unwrap(), window, cx);
                cx.emit(gpui_kit::component::input::InputEvent::Change);
            });
        });
    });
    draw(&mut child);
    child.update(|window, cx| {
        form.update(cx, |form, cx| {
            form.prefill("https://example.org/another.png".into(), window, cx);
            assert_eq!(
                form.destination.read(cx).value().as_ref(),
                destination.to_str().unwrap()
            );
        });
    });
    poll(&mut child, |cx| {
        form.read_with(cx, |form, cx| form.can_submit(cx))
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
fn save_as_picker_full_path_is_preserved_when_source_changes(cx: &mut TestAppContext) {
    let runtime = runtime();
    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("My Photo.png");
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
            form.choose_path(window, cx);
            assert!(form.choosing_path);
            assert!(!form.can_submit(cx));
        });
    });
    child.simulate_new_path_selection(|_| Some(destination.clone()));
    draw(&mut child);
    child.update(|window, cx| {
        form.update(cx, |form, cx| {
            assert!(!form.choosing_path);
            form.prefill("https://example.org/replacement.zip".into(), window, cx);
            assert_eq!(
                form.destination.read(cx).value().as_ref(),
                destination.to_str().unwrap()
            );
        });
    });
    poll(&mut child, |cx| {
        form.read_with(cx, |form, cx| form.can_submit(cx))
    });
    form.update(cx, |form, cx| form.submit(false, cx));
    draw(cx);
    view.read_with(cx, |view, _| {
        assert!(view.add_windows.is_empty());
        assert_eq!(view.queue.items.len(), 1);
        assert_eq!(view.queue.items[0].destination, destination);
        assert_eq!(
            view.queue.items[0].source(),
            "https://example.org/replacement.zip"
        );
        assert!(view.queue.items[0].is_queued());
        assert!(view.active.is_none());
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
                );
                cx.emit(gpui_kit::component::input::InputEvent::Change);
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
                );
                cx.emit(gpui_kit::component::input::InputEvent::Change);
            });
        })
    });
    draw(&mut first);
    draw(&mut second);
    first_form.read_with(cx, |form, cx| {
        assert_eq!(
            form.destination.read(cx).value().as_ref(),
            directory.path().join("first.zip").to_str().unwrap()
        )
    });
    second_form.read_with(cx, |form, cx| {
        assert_eq!(
            form.destination.read(cx).value().as_ref(),
            directory.path().join("second.zip").to_str().unwrap()
        )
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
            .destination
            .clone()
            .update(cx, |input, cx| input.focus(window, cx))
    });
    draw(&mut second);
    second.simulate_keystrokes("secondary-a");
    second.simulate_input(directory.path().join("surviving.zip").to_str().unwrap());
    draw(&mut second);
    second_form.read_with(cx, |form, cx| {
        assert_eq!(
            form.destination.read(cx).value().as_ref(),
            directory.path().join("surviving.zip").to_str().unwrap()
        )
    });
    poll(&mut second, |cx| {
        second_form.read_with(cx, |form, cx| form.can_submit(cx))
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
                );
                cx.emit(gpui_kit::component::input::InputEvent::Change);
            });
        })
    });
    draw(&mut child);
    poll(&mut child, |cx| {
        form.read_with(cx, |form, cx| form.can_submit(cx))
    });
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
        let ids = view.queue.selected.iter().copied().collect::<Vec<_>>();
        view.finish_record_removal(&ids, cx);
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
    view.update(cx, |view, cx| {
        let ids = view
            .queue
            .items
            .iter()
            .filter(|item| item.is_finished())
            .map(|item| item.id)
            .collect::<Vec<_>>();
        view.finish_record_removal(&ids, cx);
    });
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
        assert!(!details.show_details);
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
                input.set_value(destination.to_str().unwrap(), window, cx);
                cx.emit(gpui_kit::component::input::InputEvent::Change);
            });
        })
    });
    draw(&mut add);
    poll(&mut add, |cx| {
        form.read_with(cx, |form, cx| form.can_submit(cx))
    });
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
        assert!(!details.show_details);
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
    cx.update(|_, cx| cx.set_reduce_motion(true));
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
    draw(cx);
    view.read_with(cx, |view, _| assert!(view.queue.item(finished).is_some()));
    let confirm = cx
        .debug_bounds("remove-records")
        .expect("clear requires confirmation");
    cx.simulate_click(confirm.center(), gpui_kit::Modifiers::default());
    draw(cx);
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

// Sidebar widths are Kit's public defaults, not application-specific test sizing.
fn settle_sidebar(cx: &mut VisualTestContext) {
    cx.update(|_, cx| cx.set_reduce_motion(true));
    draw(cx);
    cx.executor().advance_clock(Duration::from_millis(250));
    draw(cx);
}

fn click_sidebar_toggle(cx: &mut VisualTestContext) {
    let bounds = cx.debug_bounds("toggle-sidebar").expect("sidebar toggle");
    cx.simulate_click(bounds.center(), gpui_kit::Modifiers::default());
    settle_sidebar(cx);
}

fn assert_sidebar_width(cx: &mut VisualTestContext, width: f32) {
    let bounds = cx.debug_bounds("manager-sidebar").expect("sidebar wrapper");
    assert!(
        (bounds.size.width - gpui_kit::px(width)).abs() <= gpui_kit::px(1.),
        "Expected sidebar width {width}, got {}",
        bounds.size.width.as_f32()
    );
}

fn assert_sidebar_menu_label(
    view: &Entity<DownloadManager>,
    cx: &VisualTestContext,
    expected: &str,
) {
    view.read_with(cx, |view, cx| {
        let menus = crate::native_menu::menus(&view.menu_state(cx));
        let menu = menus
            .iter()
            .find(|menu| menu.name.as_ref() == "View")
            .unwrap();
        let gpui_kit::MenuItem::Action { name, disabled, .. } = &menu.items[0] else {
            panic!("View starts with the sidebar action")
        };
        assert_eq!(name.as_ref(), expected);
        assert!(!disabled);
    });
}

#[gpui::test]
fn sidebar_mouse_toggle_restores_geometry_without_changing_queue_state(cx: &mut TestAppContext) {
    let runtime = runtime();
    let directory = tempfile::tempdir().unwrap();
    let (_root, view, cx) = mount(cx, &runtime);
    // Queue-only entries intentionally have invalid URLs: no HTTP task is started.
    let (first, second) = queued_pair(
        &view,
        cx,
        "invalid first URL".into(),
        "invalid second URL".into(),
        directory.path(),
    );
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.queue.select(second, true);
            view.search.update(cx, |input, cx| input.focus(window, cx));
            view.set_filter(Filter::Queued, cx);
        });
    });
    draw(cx);
    cx.simulate_input("first");
    settle_sidebar(cx);
    assert_sidebar_width(cx, 255.);
    for collapsed in [true, false] {
        click_sidebar_toggle(cx);
        assert_sidebar_width(cx, if collapsed { 48. } else { 255. });
        view.read_with(cx, |view, cx| {
            assert_eq!(view.sidebar_collapsed, collapsed);
            assert_eq!(
                view.queue.selected,
                std::collections::BTreeSet::from([second])
            );
            assert_eq!(view.queue.search, "first");
            assert_eq!(view.search.read(cx).value().as_ref(), "first");
            assert_eq!(view.queue.filter, Filter::Queued);
            assert_eq!(view.queue.next_queued(), Some(first));
            assert_eq!(view.queue.items.len(), 2);
            assert!(view.queue.items.iter().all(|item| item.is_queued()));
            assert!(view.active.is_none());
            assert!(!view.queue_running);
        });
    }
}

#[gpui::test]
fn sidebar_collapsed_filter_survives_expansion_without_resetting_selection(
    cx: &mut TestAppContext,
) {
    let runtime = runtime();
    let directory = tempfile::tempdir().unwrap();
    let (_root, view, cx) = mount(cx, &runtime);
    let (image, document) = view.update(cx, |view, cx| {
        let image = view
            .enqueue(
                "invalid image URL".into(),
                directory.path().join("photo.png"),
                false,
                cx,
            )
            .unwrap();
        let document = view
            .enqueue(
                "invalid document URL".into(),
                directory.path().join("notes.pdf"),
                false,
                cx,
            )
            .unwrap();
        view.queue.select(document, true);
        view.sync_table(cx);
        (image, document)
    });
    draw(cx);
    click_sidebar_toggle(cx);
    assert_sidebar_width(cx, 48.);
    // This application enables GPUI's test support, but not Kit/Base's
    // optional observation registry. SidebarMenuItem's generated IDs therefore
    // have no public bounds hook here. Exercise the real filter callback while
    // collapsed rather than guessing rail coordinates or adding UI selectors.
    view.update(cx, |view, cx| view.set_filter(Filter::Queued, cx));
    draw(cx);
    view.read_with(cx, |view, _| assert_eq!(view.queue.filter, Filter::Queued));
    view.update(cx, |view, cx| {
        view.set_filter(Filter::Category(crate::queue::Category::Image), cx)
    });
    draw(cx);
    for collapsed in [true, false] {
        view.read_with(cx, |view, _| {
            assert_eq!(view.sidebar_collapsed, collapsed);
            assert_eq!(
                view.queue.filter,
                Filter::Category(crate::queue::Category::Image)
            );
            assert_eq!(
                view.queue
                    .visible()
                    .iter()
                    .map(|item| item.id)
                    .collect::<Vec<_>>(),
                vec![image]
            );
            assert_eq!(
                view.queue.selected,
                std::collections::BTreeSet::from([document])
            );
            assert_eq!(view.queue.next_queued(), Some(image));
            assert_eq!(view.queue.items.len(), 2);
            assert!(view.active.is_none());
        });
        if collapsed {
            click_sidebar_toggle(cx);
            assert_sidebar_width(cx, 255.);
        }
    }
}

#[gpui::test]
fn sidebar_keyboard_from_search_and_native_action_from_child_share_state(cx: &mut TestAppContext) {
    let runtime = runtime();
    let (_root, view, cx) = mount(cx, &runtime);
    let _menus = cx.update(|window, cx| {
        // Test windows do not execute main's application binding setup.
        cx.bind_keys([gpui_kit::KeyBinding::new(
            "secondary-b",
            super::ToggleSidebar,
            Some("DownloadManager"),
        )]);
        crate::native_menu::install(cx, view.downgrade(), window.window_handle())
    });
    cx.update(|window, cx| {
        window.activate_window();
        view.read(cx).search.clone().update(cx, |input, cx| {
            input.focus(window, cx);
        });
    });
    draw(cx);
    cx.simulate_input("alpha");
    draw(cx);
    assert_sidebar_menu_label(&view, cx, "Collapse Sidebar");
    for collapsed in [true, false] {
        cx.simulate_keystrokes("secondary-b");
        settle_sidebar(cx);
        assert_sidebar_width(cx, if collapsed { 48. } else { 255. });
        view.read_with(cx, |view, cx| {
            assert_eq!(view.sidebar_collapsed, collapsed);
            assert_eq!(view.search.read(cx).value().as_ref(), "alpha");
            assert_eq!(view.queue.search, "alpha");
        });
        assert_sidebar_menu_label(
            &view,
            cx,
            if collapsed {
                "Expand Sidebar"
            } else {
                "Collapse Sidebar"
            },
        );
    }
    cx.update(|window, cx| view.update(cx, |view, cx| view.open_add(window, cx)));
    draw(cx);
    let mut child = add_context(&view, cx);
    child.update(|window, _| window.activate_window());
    draw(&mut child);
    for collapsed in [true, false] {
        child.dispatch_action(crate::native_menu::ToggleDownloadsSidebar);
        draw(&mut child);
        settle_sidebar(cx);
        assert_sidebar_width(cx, if collapsed { 48. } else { 255. });
        assert_sidebar_menu_label(
            &view,
            cx,
            if collapsed {
                "Expand Sidebar"
            } else {
                "Collapse Sidebar"
            },
        );
        view.read_with(cx, |view, cx| {
            assert_eq!(view.sidebar_collapsed, collapsed);
            assert_eq!(view.search.read(cx).value().as_ref(), "alpha");
            assert_eq!(view.add_windows.len(), 1);
            assert!(view.queue.items.is_empty());
            assert!(view.active.is_none());
        });
    }
}

#[gpui::test]
fn sidebar_empty_light_and_dark_layout_stays_in_one_row_at_minimum_size(cx: &mut TestAppContext) {
    let runtime = runtime();
    let (_root, view, cx) = mount(cx, &runtime);
    cx.simulate_resize(gpui_kit::size(gpui_kit::px(880.), gpui_kit::px(540.)));
    for mode in [
        gpui_kit::component::ThemeMode::Light,
        gpui_kit::component::ThemeMode::Dark,
    ] {
        cx.update(|_, cx| crate::palette::apply(mode, cx));
        settle_sidebar(cx);
        for collapsed in [false, true, false] {
            if view.read_with(cx, |view, _| view.sidebar_collapsed) != collapsed {
                click_sidebar_toggle(cx);
            }
            assert_sidebar_width(cx, if collapsed { 48. } else { 255. });
            let search = cx.debug_bounds("download-search").expect("search wrapper");
            let toggle = cx.debug_bounds("toggle-sidebar").expect("sidebar toggle");
            let options = cx.debug_bounds("options").expect("options button");
            assert!((toggle.center().y - search.center().y).abs() <= gpui_kit::px(1.));
            assert!((options.center().y - search.center().y).abs() <= gpui_kit::px(1.));
            assert!(toggle.right() <= search.left());
            assert!(options.right() <= search.left());
            assert!(options.right() <= gpui_kit::px(880.));
            #[cfg(target_os = "macos")]
            assert!(
                toggle.left() >= gpui_kit::px(80.),
                "native traffic lights remain clear"
            );
            let sidebar = cx.debug_bounds("manager-sidebar").expect("sidebar wrapper");
            assert!(sidebar.bottom() <= gpui_kit::px(540.));
            view.read_with(cx, |view, _| {
                assert!(view.queue.items.is_empty());
                assert_eq!(view.queue.filter, Filter::All);
                assert!(!view.queue_running);
            });
        }
    }
}

// These tests call the application's appearance callback directly. TestWindow
// does not expose OS appearance-event injection or Kit's private popup state.
fn assert_theme_menu_preference(
    state: &crate::native_menu::MenuState,
    expected: crate::palette::ThemePreference,
) {
    use crate::palette::ThemePreference;
    let menus = crate::native_menu::menus(state);
    let menu = menus
        .iter()
        .find(|menu| menu.name.as_ref() == "View")
        .unwrap();
    for (label, preference) in [
        ("Follow System", ThemePreference::System),
        ("Light Theme", ThemePreference::Light),
        ("Dark Theme", ThemePreference::Dark),
    ] {
        let item = menu
            .items
            .iter()
            .find(|item| matches!(item, gpui_kit::MenuItem::Action { name, .. } if name.as_ref() == label))
            .expect("named theme menu action");
        assert_eq!(item.is_checked(), preference == expected);
        assert!(!item.is_disabled());
    }
}

fn assert_stock_theme(cx: &gpui_kit::App, mode: gpui_kit::component::ThemeMode) {
    use gpui_kit::component::{Theme, ThemeColor};
    assert_eq!(Theme::global(cx).mode, mode);
    let colors = if mode.is_dark() {
        ThemeColor::dark()
    } else {
        ThemeColor::light()
    };
    assert_eq!(Theme::global(cx).colors.background, colors.background);
    assert_eq!(Theme::global(cx).colors.foreground, colors.foreground);
}

#[gpui::test]
fn theme_preference_defaults_to_system_and_native_checks_follow_preference(
    cx: &mut TestAppContext,
) {
    use crate::palette::{self, ThemePreference};
    use gpui_kit::component::ThemeMode;
    let runtime = runtime();
    let (_root, view, cx) = mount(cx, &runtime);
    let _menus = cx.update(|window, cx| {
        crate::native_menu::install(cx, view.downgrade(), window.window_handle())
    });
    cx.update(|_, cx| {
        assert_eq!(palette::preference(cx), ThemePreference::System);
        assert_stock_theme(cx, ThemeMode::from(cx.window_appearance()));
    });
    assert_eq!(ThemePreference::System.label(), "Follow system");
    assert!(cx.debug_bounds("theme-selector").is_some());
    cx.update(|_, cx| palette::appearance_changed(gpui_kit::WindowAppearance::Dark, cx));
    draw(cx);
    view.read_with(cx, |view, cx| {
        assert_stock_theme(cx, ThemeMode::Dark);
        assert!(view.menu_state(cx).dark);
        assert_theme_menu_preference(&view.menu_state(cx), ThemePreference::System);
    });
    cx.dispatch_action(crate::native_menu::UseLightTheme);
    draw(cx);
    view.read_with(cx, |view, cx| {
        assert_eq!(palette::preference(cx), ThemePreference::Light);
        assert_stock_theme(cx, ThemeMode::Light);
        assert_theme_menu_preference(&view.menu_state(cx), ThemePreference::Light);
    });
    cx.dispatch_action(crate::native_menu::UseDarkTheme);
    draw(cx);
    view.read_with(cx, |view, cx| {
        assert_eq!(palette::preference(cx), ThemePreference::Dark);
        assert_stock_theme(cx, ThemeMode::Dark);
        assert_theme_menu_preference(&view.menu_state(cx), ThemePreference::Dark);
    });
    cx.dispatch_action(crate::native_menu::UseSystemTheme);
    draw(cx);
    view.read_with(cx, |view, cx| {
        assert_eq!(palette::preference(cx), ThemePreference::System);
        assert_stock_theme(cx, ThemeMode::from(cx.window_appearance()));
        assert_theme_menu_preference(&view.menu_state(cx), ThemePreference::System);
    });
}

#[gpui::test]
fn system_theme_callback_updates_shared_native_roots_without_losing_add_drafts(
    cx: &mut TestAppContext,
) {
    use crate::palette::{self, ThemePreference};
    use gpui_kit::{WindowAppearance, component::ThemeMode};
    let runtime = runtime();
    let directory = tempfile::tempdir().unwrap();
    let (_root, view, cx) = mount(cx, &runtime);
    let id = view.update(cx, |view, cx| {
        view.enqueue(
            "https://example.org/queued-theme.zip".into(),
            directory.path().join("queued-theme.zip"),
            false,
            cx,
        )
        .unwrap()
    });
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.open_add(window, cx);
            view.open_add(window, cx);
            view.open_details(id, window, cx);
        })
    });
    draw(cx);
    let forms = view.read_with(cx, |view, _| {
        assert_eq!(view.add_windows.len(), 2);
        view.add_windows
            .values()
            .map(|child| (child.handle, child.upgrade().unwrap()))
            .collect::<Vec<_>>()
    });
    for (index, (handle, form)) in forms.iter().enumerate() {
        let mut child = VisualTestContext::from_window(*handle, &cx.cx);
        child.update(|window, cx| {
            form.update(cx, |form, cx| {
                form.prefill(format!("https://example.org/draft-{index}.zip"), window, cx)
            })
        });
        draw(&mut child);
    }
    let drafts = forms
        .iter()
        .map(|(_, form)| {
            form.read_with(cx, |form, cx| {
                (
                    form.source.read(cx).value().to_string(),
                    form.destination.read(cx).value().to_string(),
                )
            })
        })
        .collect::<Vec<_>>();
    let details_handle = view.read_with(cx, |view, _| view.detail_windows[&id].handle);
    for (appearance, mode) in [
        (WindowAppearance::Dark, ThemeMode::Dark),
        (WindowAppearance::Light, ThemeMode::Light),
    ] {
        cx.update(|_, cx| palette::appearance_changed(appearance, cx));
        draw(cx);
        view.read_with(cx, |view, cx| {
            assert_eq!(palette::preference(cx), ThemePreference::System);
            assert_stock_theme(cx, mode);
            assert_eq!(view.menu_state(cx).dark, mode.is_dark());
            assert!(view.queue.item(id).unwrap().is_queued());
            assert!(view.active.is_none());
            assert!(view.transfer.is_none());
        });
        for ((handle, form), draft) in forms.iter().zip(&drafts) {
            let mut child = VisualTestContext::from_window(*handle, &cx.cx);
            draw(&mut child);
            child.update(|_, cx| {
                assert_eq!(palette::preference(cx), ThemePreference::System);
                assert_stock_theme(cx, mode);
            });
            form.read_with(&child, |form, cx| {
                assert_eq!(form.source.read(cx).value().as_ref(), draft.0);
                assert_eq!(form.destination.read(cx).value().as_ref(), draft.1);
            });
        }
        let mut details = VisualTestContext::from_window(details_handle, &cx.cx);
        draw(&mut details);
        details.update(|_, cx| {
            assert_eq!(palette::preference(cx), ThemePreference::System);
            assert_stock_theme(cx, mode);
        });
    }
    for handle in forms
        .iter()
        .map(|(handle, _)| *handle)
        .chain([details_handle])
    {
        let mut child = VisualTestContext::from_window(handle, &cx.cx);
        child.update(|window, _| window.remove_window());
    }
    draw(cx);
    view.read_with(cx, |view, _| {
        assert!(view.add_windows.is_empty());
        assert!(view.detail_windows.is_empty());
    });
    assert_eq!(cx.update(|_, cx| cx.windows().len()), 1);
    assert!(!directory.path().join("queued-theme.zip").exists());
}

#[gpui::test]
fn manual_theme_native_actions_ignore_appearance_and_system_survives_main_close(
    cx: &mut TestAppContext,
) {
    use crate::palette::{self, ThemePreference};
    use gpui_kit::{WindowAppearance, component::ThemeMode};
    let runtime = runtime();
    let (root, view, cx) = mount(cx, &runtime);
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
    cx.dispatch_action(crate::native_menu::UseLightTheme);
    cx.update(|_, cx| palette::appearance_changed(WindowAppearance::Dark, cx));
    draw(cx);
    cx.update(|_, cx| {
        assert_eq!(palette::preference(cx), ThemePreference::Light);
        assert_stock_theme(cx, ThemeMode::Light);
    });
    cx.dispatch_action(crate::native_menu::UseDarkTheme);
    cx.update(|_, cx| palette::appearance_changed(WindowAppearance::Light, cx));
    draw(cx);
    cx.update(|_, cx| {
        assert_eq!(palette::preference(cx), ThemePreference::Dark);
        assert_stock_theme(cx, ThemeMode::Dark);
    });
    cx.dispatch_action(crate::native_menu::NewDownload);
    draw(cx);
    let mut child = add_context(&view, cx);
    let owner = view.downgrade();
    cx.update(|window, _| window.remove_window());
    drop(root);
    drop(view);
    child.run_until_parked();
    child.update(|window, _| window.activate_window());
    draw(&mut child);
    child.dispatch_action(crate::native_menu::UseSystemTheme);
    draw(&mut child);
    child.update(|_, cx| {
        assert_eq!(palette::preference(cx), ThemePreference::System);
        palette::appearance_changed(WindowAppearance::Dark, cx);
    });
    draw(&mut child);
    owner.upgrade().unwrap().read_with(&child, |view, cx| {
        assert_stock_theme(cx, ThemeMode::Dark);
        assert_eq!(palette::preference(cx), ThemePreference::System);
        assert_theme_menu_preference(&view.menu_state(cx), ThemePreference::System);
        assert_eq!(view.add_windows.len(), 1);
        assert_eq!(cx.windows().len(), 1);
        assert!(view.queue.items.is_empty());
        assert!(view.active.is_none());
    });
    child.update(|window, _| window.remove_window());
    child.run_until_parked();
    assert_eq!(child.cx.update(|cx| cx.windows().len()), 0);
    assert!(
        owner.upgrade().is_none(),
        "last child root releases manager"
    );
}

#[gpui::test]
fn theme_selector_stock_geometry_and_click_preserve_preference_until_selection(
    cx: &mut TestAppContext,
) {
    use crate::palette::{self, ThemePreference};
    let runtime = runtime();
    let (_root, _view, cx) = mount(cx, &runtime);
    cx.simulate_resize(gpui_kit::size(gpui_kit::px(880.), gpui_kit::px(540.)));
    draw(cx);
    let theme = cx
        .debug_bounds("theme-selector")
        .expect("stock theme dropdown button");
    let add = cx.debug_bounds("add-url").expect("Add URL button");
    let search = cx.debug_bounds("download-search").expect("search wrapper");
    let toggle = cx.debug_bounds("toggle-sidebar").expect("sidebar toggle");
    for control in [add, search, toggle] {
        assert!((theme.center().y - control.center().y).abs() <= gpui_kit::px(1.));
    }
    for control in [add, toggle] {
        assert!((theme.size.height - control.size.height).abs() <= gpui_kit::px(1.));
    }
    assert!(theme.size.height > gpui_kit::px(24.));
    assert!(theme.right() <= gpui_kit::px(880.));
    cx.update(|_, cx| assert_eq!(palette::preference(cx), ThemePreference::System));
    cx.simulate_click(theme.center(), gpui_kit::Modifiers::default());
    draw(cx);
    // Clicking the stock dropdown is exercised, not a native popup visibility
    // assertion. Its private Kit popup state is intentionally not inspected.
    cx.update(|_, cx| assert_eq!(palette::preference(cx), ThemePreference::System));
}

// Advanced controls use a real, reusable HTTP peer. Unlike the one-request
// fixture above, probe, parallel parts and resumed parts each own a connection.
#[derive(Clone)]
struct AdvancedRequest {
    range: Option<(usize, usize)>,
    if_range: Option<String>,
}

struct AdvancedFixture {
    source: String,
    body: std::sync::Arc<Vec<u8>>,
    requests: Receiver<AdvancedRequest>,
    release: std::sync::Arc<std::sync::atomic::AtomicBool>,
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    server: Option<thread::JoinHandle<()>>,
}

impl AdvancedFixture {
    fn release_all(&self) {
        self.release
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }
}

impl Drop for AdvancedFixture {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::SeqCst);
        self.release_all();
        if let Some(server) = self.server.take() {
            server.join().expect("advanced fixture server panicked");
        }
    }
}

fn advanced_fixture(ranges: bool) -> AdvancedFixture {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    let body = Arc::new((0..65536).map(|i| (i % 251) as u8).collect::<Vec<_>>());
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let source = format!("http://{}/file", listener.local_addr().unwrap());
    let release = Arc::new(AtomicBool::new(false));
    let stop = Arc::new(AtomicBool::new(false));
    let (tx, requests) = mpsc::channel();
    let worker_body = body.clone();
    let worker_release = release.clone();
    let worker_stop = stop.clone();
    let server = thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut workers = Vec::new();
        while !worker_stop.load(Ordering::SeqCst) && Instant::now() < deadline {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    let body = worker_body.clone();
                    let release = worker_release.clone();
                    let stop = worker_stop.clone();
                    let tx = tx.clone();
                    workers.push(thread::spawn(move || {
                        stream.set_read_timeout(Some(Duration::from_millis(200))).unwrap();
                        stream.set_write_timeout(Some(Duration::from_millis(200))).unwrap();
                        let mut reader = BufReader::new(stream.try_clone().unwrap());
                        let mut headers = Vec::new();
                        let read_deadline = Instant::now() + Duration::from_secs(2);
                        while !headers.ends_with(b"\r\n\r\n") {
                            if stop.load(Ordering::SeqCst) || Instant::now() >= read_deadline {
                                return;
                            }
                            match reader.read_until(b'\n', &mut headers) {
                                Ok(0) => return,
                                Ok(_) => {},
                                Err(e) if matches!(e.kind(), std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock) => {},
                                Err(_) => return,
                            }
                            assert!(headers.len() <= 16 * 1024);
                        }
                        let headers = String::from_utf8(headers).unwrap();
                        assert!(headers.starts_with("GET /file HTTP/1.1\r\n"));
                        let mut range = None;
                        let mut if_range = None;
                        for line in headers.lines() {
                            if let Some((name, value)) = line.split_once(':') {
                                if name.eq_ignore_ascii_case("range") {
                                    let (start, end) = value.trim().strip_prefix("bytes=").unwrap().split_once('-').unwrap();
                                    range = Some((start.parse::<usize>().unwrap(), end.parse::<usize>().unwrap()));
                                } else if name.eq_ignore_ascii_case("if-range") {
                                    if_range = Some(value.trim().to_string());
                                }
                            }
                        }
                        let _ = tx.send(AdvancedRequest { range, if_range });
                        let (start, end) = if ranges { range.unwrap_or((0, body.len() - 1)) } else { (0, body.len() - 1) };
                        assert!(start <= end && end < body.len());
                        let response = if ranges && range.is_some() {
                            format!("HTTP/1.1 206 Partial Content\r\nContent-Range: bytes {start}-{end}/{}\r\nContent-Length: {}\r\nETag: \"advanced-v1\"\r\nAccept-Ranges: bytes\r\nConnection: close\r\n\r\n", body.len(), end - start + 1)
                        } else {
                            format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len())
                        };
                        if stream.write_all(response.as_bytes()).is_err() || stream.write_all(&body[start..=start]).is_err() { return; }
                        // Only the validated 0-0 probe is never gated.
                        if ranges && range == Some((0, 0)) { return; }
                        let gate_deadline = Instant::now() + Duration::from_secs(5);
                        while !release.load(Ordering::SeqCst) {
                            if stop.load(Ordering::SeqCst) || Instant::now() >= gate_deadline { return; }
                            thread::sleep(Duration::from_millis(1));
                        }
                        if !stop.load(Ordering::SeqCst) {
                            let _ = stream.write_all(&body[start + 1..=end]);
                        }
                    }));
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(1))
                }
                Err(e) => panic!("advanced fixture accept: {e}"),
            }
        }
        for worker in workers {
            worker.join().expect("advanced fixture worker panicked");
        }
    });
    AdvancedFixture {
        source,
        body,
        requests,
        release,
        stop,
        server: Some(server),
    }
}

fn advanced_add(
    view: &Entity<DownloadManager>,
    cx: &mut VisualTestContext,
    fixture: &AdvancedFixture,
    path: std::path::PathBuf,
    start: bool,
) -> DownloadId {
    view.update(cx, |view, cx| {
        view.enqueue_with_options(
            fixture.source.clone(),
            path,
            start,
            ulos_core::transfer::TransferOptions {
                connections: 2,
                ..Default::default()
            },
            cx,
        )
        .unwrap()
    })
}

fn advanced_details(
    view: &Entity<DownloadManager>,
    id: DownloadId,
    cx: &mut VisualTestContext,
) -> (
    Entity<crate::download_details::DownloadDetails>,
    VisualTestContext,
) {
    cx.update(|window, cx| view.update(cx, |view, cx| view.open_details(id, window, cx)));
    draw(cx);
    let (details, handle) = view.read_with(cx, |view, _| {
        let child = &view.detail_windows[&id];
        (child.upgrade().unwrap(), child.handle)
    });
    let mut child = VisualTestContext::from_window(handle, &cx.cx);
    child.update(|window, cx| {
        window.activate_window();
        details.update(cx, |details, cx| {
            details.show_details = true;
            cx.notify();
        });
    });
    draw(&mut child);
    (details, child)
}

fn advanced_finished(view: &Entity<DownloadManager>, id: DownloadId, cx: &mut VisualTestContext) {
    poll(cx, |cx| {
        view.read_with(cx, |view, _| view.queue.item(id).unwrap().is_finished())
    });
}

#[gpui::test]
fn advanced_parallel_pause_retains_slot_and_resumes_validated_ranges(cx: &mut TestAppContext) {
    use crate::download_details::{SetConnections, TogglePause};
    use ulos_core::transfer::{ConnectionStatus, ResumeCapability};
    let runtime = runtime();
    let directory = tempfile::tempdir().unwrap();
    let fixture = advanced_fixture(true);
    let (_root, view, cx) = mount(cx, &runtime);
    let id = advanced_add(
        &view,
        cx,
        &fixture,
        directory.path().join("parallel.bin"),
        true,
    );
    let (details, mut child) = advanced_details(&view, id, cx);
    let next = view.update(cx, |view, cx| {
        view.enqueue(
            "https://example.org/next".into(),
            directory.path().join("next.bin"),
            false,
            cx,
        )
        .unwrap()
    });
    poll(cx, |cx| {
        details.read_with(cx, |d, _| {
            d.snapshot.telemetry.connections.len() == 2
                && d.snapshot.telemetry.progress.downloaded_bytes >= 2
        })
    });
    details.update(&mut child, |_, cx| {
        cx.emit(SetConnections { id, connections: 8 })
    });
    details.update(&mut child, |_, cx| cx.emit(TogglePause(id)));
    poll(cx, |cx| details.read_with(cx, |d, _| d.snapshot.paused));
    view.read_with(cx, |view, _| {
        assert_eq!(view.active, Some(id));
        assert!(view.queue.item(next).unwrap().is_queued());
        assert_eq!(view.queue.item(id).unwrap().options.connections, 2);
    });
    details.read_with(cx, |d, _| {
        assert_eq!(d.snapshot.telemetry.resume, ResumeCapability::Supported);
        assert!(
            d.snapshot
                .telemetry
                .connections
                .iter()
                .all(|c| c.status == ConnectionStatus::Paused)
        );
    });
    details.update(&mut child, |_, cx| cx.emit(TogglePause(id)));
    let mut observed = Vec::new();
    poll(cx, |_| {
        observed.extend(fixture.requests.try_iter());
        observed
            .iter()
            .filter(|r| r.if_range.as_deref() == Some("\"advanced-v1\""))
            .count()
            >= 4
    });
    assert!(observed.iter().any(|r| r.range == Some((0, 0))));
    assert!(
        observed
            .iter()
            .any(|r| r.range.is_some_and(|(start, _)| start > 32768))
    );
    fixture.release_all();
    advanced_finished(&view, id, cx);
    assert_eq!(
        std::fs::read(directory.path().join("parallel.bin")).unwrap(),
        *fixture.body
    );
    details.read_with(cx, |d, _| {
        assert_eq!(d.snapshot.telemetry.connections.len(), 2);
        assert!(
            d.snapshot
                .telemetry
                .connections
                .iter()
                .all(|c| c.status == ConnectionStatus::Complete)
        );
    });
}

#[gpui::test]
fn advanced_live_rate_changes_reach_task_and_zero_is_rejected(cx: &mut TestAppContext) {
    use crate::download_details::SetRateLimit;
    let runtime = runtime();
    let directory = tempfile::tempdir().unwrap();
    let fixture = advanced_fixture(true);
    let (_root, view, cx) = mount(cx, &runtime);
    let id = advanced_add(&view, cx, &fixture, directory.path().join("rate.bin"), true);
    let (details, mut child) = advanced_details(&view, id, cx);
    poll(cx, |cx| {
        details.read_with(cx, |d, _| !d.snapshot.telemetry.connections.is_empty())
    });
    details.update(&mut child, |_, cx| {
        cx.emit(SetRateLimit {
            id,
            bytes_per_second: Some(512),
        })
    });
    poll(cx, |cx| {
        details.read_with(cx, |d, _| {
            d.snapshot.telemetry.bytes_per_second == Some(512)
        })
    });
    details.update(&mut child, |_, cx| {
        cx.emit(SetRateLimit {
            id,
            bytes_per_second: Some(0),
        })
    });
    draw(&mut child);
    view.read_with(cx, |view, _| {
        assert_eq!(
            view.queue.item(id).unwrap().options.bytes_per_second,
            Some(512)
        )
    });
    details.update(&mut child, |_, cx| {
        cx.emit(SetRateLimit {
            id,
            bytes_per_second: None,
        })
    });
    poll(cx, |cx| {
        details.read_with(cx, |d, _| d.snapshot.telemetry.bytes_per_second.is_none())
    });
    fixture.release_all();
    advanced_finished(&view, id, cx);
    assert_eq!(
        std::fs::read(directory.path().join("rate.bin")).unwrap(),
        *fixture.body
    );
    details.read_with(cx, |d, _| {
        assert_eq!(
            d.snapshot.telemetry.resume,
            ulos_core::transfer::ResumeCapability::Supported
        )
    });
}

#[gpui::test]
fn advanced_queued_connection_input_is_retained_and_bounded(cx: &mut TestAppContext) {
    let runtime = runtime();
    let directory = tempfile::tempdir().unwrap();
    let fixture = advanced_fixture(true);
    let (_root, view, cx) = mount(cx, &runtime);
    let id = view.update(cx, |view, cx| {
        view.enqueue(
            fixture.source.clone(),
            directory.path().join("queued.bin"),
            false,
            cx,
        )
        .unwrap()
    });
    let (details, mut child) = advanced_details(&view, id, cx);
    let input = details.read_with(cx, |d, _| d.connections_input.clone().unwrap());
    child.update(|window, cx| input.update(cx, |input, cx| input.set_value("2", window, cx)));
    details.update(&mut child, |d, cx| d.apply_connections(cx));
    draw(&mut child);
    assert_eq!(
        details.read_with(cx, |d, _| d.snapshot.options.connections),
        2
    );
    child.update(|window, cx| input.update(cx, |input, cx| input.set_value("17", window, cx)));
    details.update(&mut child, |d, cx| d.apply_connections(cx));
    draw(&mut child);
    assert_eq!(
        details.read_with(cx, |d, _| d.snapshot.options.connections),
        2
    );
    assert_eq!(
        details.read_with(cx, |d, _| d.connections_input.clone().unwrap()),
        input
    );
    view.update(cx, |view, cx| view.start_item(id, cx));
    poll(cx, |cx| {
        details.read_with(cx, |d, _| d.snapshot.telemetry.connections.len() == 2)
    });
    fixture.release_all();
    advanced_finished(&view, id, cx);
    assert_eq!(
        std::fs::read(directory.path().join("queued.bin")).unwrap(),
        *fixture.body
    );
}

#[gpui::test]
fn advanced_collision_rename_retry_preserves_original_without_initial_http(
    cx: &mut TestAppContext,
) {
    let runtime = runtime();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("original.bin");
    std::fs::write(&path, b"original content").unwrap();
    let fixture = advanced_fixture(false);
    let (_root, view, cx) = mount(cx, &runtime);
    cx.update(|_, cx| cx.set_reduce_motion(true));
    let id = advanced_add(&view, cx, &fixture, path.clone(), true);
    let (details, mut child) = advanced_details(&view, id, cx);
    poll(cx, |cx| {
        details.read_with(cx, |d, _| d.snapshot.status == "Failed")
    });
    assert!(fixture.requests.try_recv().is_err());
    details.update(&mut child, |_, cx| {
        cx.emit(crate::download_details::RetryDownload {
            id,
            collision: ulos_core::transfer::CollisionPolicy::Rename,
        })
    });
    fixture.release_all();
    advanced_finished(&view, id, cx);
    let renamed = directory.path().join("original (1).bin");
    assert_eq!(std::fs::read(&path).unwrap(), b"original content");
    assert_eq!(std::fs::read(&renamed).unwrap(), *fixture.body);
    view.read_with(cx, |view, _| {
        assert_eq!(view.queue.item(id).unwrap().destination, renamed)
    });
}

#[gpui::test]
fn advanced_overwrite_retry_requires_dialog_confirmation(cx: &mut TestAppContext) {
    let runtime = runtime();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("overwrite.bin");
    std::fs::write(&path, b"keep until confirmed").unwrap();
    let fixture = advanced_fixture(false);
    let (_root, view, cx) = mount(cx, &runtime);
    cx.update(|_, cx| cx.set_reduce_motion(true));
    let id = advanced_add(&view, cx, &fixture, path.clone(), true);
    let (details, mut child) = advanced_details(&view, id, cx);
    poll(cx, |cx| {
        details.read_with(cx, |d, _| d.snapshot.status == "Failed")
    });
    details.update(&mut child, |_, cx| {
        cx.emit(crate::download_details::RetryDownload {
            id,
            collision: ulos_core::transfer::CollisionPolicy::Overwrite,
        })
    });
    draw(&mut child);
    assert!(fixture.requests.try_recv().is_err());
    assert_eq!(std::fs::read(&path).unwrap(), b"keep until confirmed");
    let confirm = child
        .debug_bounds("confirm-retry")
        .expect("overwrite consent control");
    child.simulate_click(confirm.center(), gpui_kit::Modifiers::default());
    fixture.release_all();
    advanced_finished(&view, id, cx);
    assert_eq!(std::fs::read(&path).unwrap(), *fixture.body);
}

#[gpui::test]
fn advanced_add_overwrite_consent_resets_on_destination_edit(cx: &mut TestAppContext) {
    let runtime = runtime();
    let directory = tempfile::tempdir().unwrap();
    let fixture = advanced_fixture(false);
    std::fs::write(directory.path().join("consent.bin"), b"original").unwrap();
    std::fs::write(directory.path().join("edited.bin"), b"other original").unwrap();
    let (_root, view, cx) = mount(cx, &runtime);
    cx.update(|window, cx| view.update(cx, |view, cx| view.open_add(window, cx)));
    draw(cx);
    let form = view.read_with(cx, |view, _| {
        view.add_windows.values().next().unwrap().upgrade().unwrap()
    });
    let mut child = add_context(&view, cx);
    child.update(|window, cx| {
        form.update(cx, |form, cx| {
            form.prefill(fixture.source.clone(), window, cx);
            form.destination.update(cx, |input, cx| {
                input.set_value(
                    directory.path().join("consent.bin").to_str().unwrap(),
                    window,
                    cx,
                );
                cx.emit(gpui_kit::component::input::InputEvent::Change);
            });
        })
    });
    poll(&mut child, |cx| {
        form.read_with(cx, |form, cx| form.has_file_target(cx))
    });
    form.update(&mut child, |form, cx| {
        form.set_collision(ulos_core::transfer::CollisionPolicy::Overwrite, cx)
    });
    draw(&mut child);
    form.read_with(cx, |form, cx| assert!(!form.can_submit(cx)));
    form.update(&mut child, |form, cx| form.submit(false, cx));
    view.read_with(cx, |view, _| assert!(view.queue.items.is_empty()));
    form.update(&mut child, |form, cx| form.authorize_overwrite(true, cx));
    form.read_with(cx, |form, cx| assert!(form.can_submit(cx)));
    child.update(|window, cx| {
        form.read(cx)
            .destination
            .clone()
            .update(cx, |input, cx| input.focus(window, cx))
    });
    draw(&mut child);
    child.simulate_keystrokes("secondary-a");
    child.simulate_input(directory.path().join("edited.bin").to_str().unwrap());
    draw(&mut child);
    form.read_with(cx, |form, _| assert!(!form.overwrite_confirmed));
    poll(&mut child, |cx| {
        form.read_with(cx, |form, cx| form.has_file_target(cx))
    });
    form.update(&mut child, |form, cx| {
        form.set_collision(ulos_core::transfer::CollisionPolicy::Overwrite, cx);
        assert!(!form.can_submit(cx));
        form.authorize_overwrite(true, cx);
        form.submit(false, cx);
    });
    draw(cx);
    view.read_with(cx, |view, _| {
        assert_eq!(view.queue.items.len(), 1);
        assert_eq!(
            view.queue.items[0].options.collision,
            ulos_core::transfer::CollisionPolicy::Overwrite
        );
        assert_eq!(
            view.queue.items[0].destination,
            directory.path().join("edited.bin")
        );
    });
    assert!(fixture.requests.try_recv().is_err());
}

#[gpui::test]
fn advanced_progress_footer_is_shared_by_all_status_tabs(cx: &mut TestAppContext) {
    let runtime = runtime();
    let directory = tempfile::tempdir().unwrap();
    let (_root, view, cx) = mount(cx, &runtime);
    let id = view.update(cx, |view, cx| {
        view.enqueue(
            "https://example.org/progress".into(),
            directory.path().join("progress.bin"),
            false,
            cx,
        )
        .unwrap()
    });
    let (details, mut child) = advanced_details(&view, id, cx);
    // Synthetic progress deliberately tests presentation, not engine behaviour.
    view.update(cx, |view, cx| {
        view.queue
            .item_mut(id)
            .unwrap()
            .completion
            .arm(crate::completion::CompletionAction::Shutdown);
        view.queue.item_mut(id).unwrap().apply_state(
            TransferState::Downloading(DownloadProgress {
                downloaded_bytes: 50,
                total_bytes: Some(100),
            }),
            Instant::now(),
        );
        view.refresh_details(cx);
    });
    for tab in 0..3 {
        details.update(&mut child, |d, cx| d.select_tab(tab, cx));
        draw(&mut child);
        assert!(child.debug_bounds("status-progress").is_some(), "tab {tab}");
        assert!(
            child.debug_bounds("shared-cancel-shutdown").is_some(),
            "Shutdown cancellation must remain visible on tab {tab}"
        );
        assert!(
            child.debug_bounds("status-percentage").is_some(),
            "tab {tab}"
        );
        details.read_with(cx, |d, _| {
            assert_eq!(
                d.snapshot.progress,
                Some(DownloadProgress {
                    downloaded_bytes: 50,
                    total_bytes: Some(100)
                })
            )
        });
    }
    details.update(&mut child, |_, cx| {
        cx.emit(crate::download_details::CancelShutdown(id))
    });
    draw(&mut child);
    view.read_with(cx, |view, _| {
        assert!(!view.queue.item(id).unwrap().completion.is_armed())
    });
}

#[gpui::test]
fn advanced_shutdown_confirmation_countdown_and_new_work_cancellation(cx: &mut TestAppContext) {
    if !crate::completion::shutdown_supported() {
        return;
    }
    let runtime = runtime();
    let directory = tempfile::tempdir().unwrap();
    let fixture = advanced_fixture(false);
    let (_root, view, cx) = mount(cx, &runtime);
    let id = advanced_add(
        &view,
        cx,
        &fixture,
        directory.path().join("shutdown.bin"),
        true,
    );
    let (details, mut child) = advanced_details(&view, id, cx);
    details.update(&mut child, |_, cx| {
        cx.emit(crate::download_details::RequestShutdown(id))
    });
    draw(&mut child);
    assert!(!details.read_with(cx, |d, _| d.snapshot.shutdown_armed));
    let confirm = child
        .debug_bounds("confirm-poweroff")
        .expect("shutdown consent control");
    child.simulate_click(confirm.center(), gpui_kit::Modifiers::default());
    poll(cx, |cx| {
        details.read_with(cx, |d, _| d.snapshot.shutdown_armed)
    });
    fixture.release_all();
    advanced_finished(&view, id, cx);
    details.read_with(cx, |d, _| {
        assert_eq!(d.snapshot.shutdown_remaining, Some(30))
    });
    view.update(cx, |view, cx| {
        view.enqueue(
            "https://example.org/new".into(),
            directory.path().join("new.bin"),
            false,
            cx,
        )
        .unwrap();
        assert!(!view.completion_tick(Instant::now() + Duration::from_secs(35), cx));
        assert!(!view.completion_tick(Instant::now() + Duration::from_secs(36), cx));
    });
    details.read_with(cx, |d, _| {
        assert!(!d.snapshot.shutdown_armed);
        assert!(d.snapshot.shutdown_remaining.is_none());
    });
    // No provider call: new work cancels the actual countdown before its deadline.
}

#[gpui::test]
fn advanced_native_status_close_disarms_shutdown_without_cancelling_transfer(
    cx: &mut TestAppContext,
) {
    use ulos_core::transfer::ResumeCapability;
    if !crate::completion::shutdown_supported() {
        return;
    }
    let runtime = runtime();
    let directory = tempfile::tempdir().unwrap();
    let fixture = advanced_fixture(true);
    let destination = directory.path().join("close-before-publication.bin");
    let (_root, view, cx) = mount(cx, &runtime);
    cx.update(|_, cx| cx.set_reduce_motion(true));
    let id = advanced_add(&view, cx, &fixture, destination.clone(), true);
    poll(cx, |cx| {
        view.read_with(cx, |view, _| {
            let telemetry = &view.queue.item(id).unwrap().telemetry;
            telemetry.resume == ResumeCapability::Supported
                && telemetry.connections.len() == 2
                && telemetry.progress.downloaded_bytes >= 2
        })
    });
    let (details, mut child) = advanced_details(&view, id, cx);
    details.update(&mut child, |_, cx| {
        cx.emit(crate::download_details::RequestShutdown(id))
    });
    draw(&mut child);
    assert!(!details.read_with(cx, |d, _| d.snapshot.shutdown_armed));
    view.read_with(cx, |view, _| {
        assert!(!view.queue.item(id).unwrap().completion.is_armed());
        assert!(!destination.exists());
    });
    let confirm = child.debug_bounds("confirm-poweroff").unwrap();
    child.simulate_click(confirm.center(), gpui_kit::Modifiers::default());
    draw(&mut child);
    poll(cx, |cx| {
        details.read_with(cx, |d, _| d.snapshot.shutdown_armed)
    });
    assert!(child.simulate_close(), "status must allow native close");
    child.update(|window, _| window.remove_window());
    child.run_until_parked();
    draw(cx);
    view.read_with(cx, |view, _| {
        assert!(view.detail_windows.is_empty());
        assert!(!view.queue.item(id).unwrap().completion.is_armed());
        assert_eq!(view.active, Some(id));
        assert!(view.transfer.is_some());
        assert!(view.shutdown_id.is_none());
        assert!(view._shutdown_countdown.is_none());
    });
    fixture.release_all();
    advanced_finished(&view, id, cx);
    assert_eq!(std::fs::read(&destination).unwrap(), *fixture.body);
    view.update(cx, |view, cx| {
        assert!(view.active.is_none());
        assert!(view.shutdown_id.is_none());
        assert!(view.shutdown_remaining.is_none());
        assert!(view._shutdown_countdown.is_none());
        assert!(!view.queue.item(id).unwrap().completion.is_armed());
        assert!(!view.completion_tick(Instant::now() + Duration::from_secs(35), cx));
    });
}

#[gpui::test]
fn advanced_status_close_cancels_countdown_at_shutdown_deadline(cx: &mut TestAppContext) {
    if !crate::completion::shutdown_supported() {
        return;
    }
    let runtime = runtime();
    let directory = tempfile::tempdir().unwrap();
    let fixture = advanced_fixture(true);
    let destination = directory.path().join("close-during-countdown.bin");
    let (_root, view, cx) = mount(cx, &runtime);
    cx.update(|_, cx| cx.set_reduce_motion(true));
    let id = advanced_add(&view, cx, &fixture, destination.clone(), true);
    let (details, mut child) = advanced_details(&view, id, cx);
    details.update(&mut child, |_, cx| {
        cx.emit(crate::download_details::RequestShutdown(id))
    });
    draw(&mut child);
    assert!(!details.read_with(cx, |d, _| d.snapshot.shutdown_armed));
    let confirm = child.debug_bounds("confirm-poweroff").unwrap();
    child.simulate_click(confirm.center(), gpui_kit::Modifiers::default());
    draw(&mut child);
    poll(cx, |cx| {
        details.read_with(cx, |d, _| d.snapshot.shutdown_armed)
    });
    fixture.release_all();
    advanced_finished(&view, id, cx);
    details.read_with(cx, |d, _| {
        assert_eq!(d.snapshot.shutdown_remaining, Some(30))
    });
    view.read_with(cx, |view, _| {
        assert_eq!(view.shutdown_id, Some(id));
        assert_eq!(view.shutdown_remaining, Some(30));
        assert!(view.queue.item(id).unwrap().completion.is_armed());
    });
    // Exercise the authored close event, then check the deadline before drawing
    // the main window. A surviving main window must not authorize shutdown.
    details.update(&mut child, |_, cx| {
        cx.emit(crate::download_details::CloseDetails)
    });
    child.run_until_parked();
    view.update(cx, |view, cx| {
        assert!(!view.completion_tick(Instant::now() + Duration::from_secs(35), cx));
        assert!(!view.completion_tick(Instant::now() + Duration::from_secs(36), cx));
        assert!(!view.queue.item(id).unwrap().completion.is_armed());
        assert!(view.shutdown_id.is_none());
        assert!(view.shutdown_remaining.is_none());
        assert!(view._shutdown_countdown.is_none());
        assert_eq!(cx.windows().len(), 1);
    });
    draw(cx);
    view.read_with(cx, |view, _| assert!(view.detail_windows.is_empty()));
    assert_eq!(std::fs::read(&destination).unwrap(), *fixture.body);
}

#[gpui::test]
fn advanced_unfinished_queue_and_failure_block_shutdown_and_terminal_rearming(
    cx: &mut TestAppContext,
) {
    if !crate::completion::shutdown_supported() {
        return;
    }
    let runtime = runtime();
    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("published.bin");
    let bytes = b"published receipt fixture";
    let (_root, view, cx) = mount(cx, &runtime);
    let (id, unfinished) = view.update(cx, |view, cx| {
        let id = view
            .enqueue(
                "https://example.invalid/published".into(),
                destination.clone(),
                false,
                cx,
            )
            .unwrap();
        let unfinished = view
            .enqueue(
                "https://example.invalid/unfinished".into(),
                directory.path().join("unfinished.bin"),
                false,
                cx,
            )
            .unwrap();
        (id, unfinished)
    });
    let (details, mut child) = advanced_details(&view, id, cx);
    // This model-only test injects terminal receipts, not HTTP success. Explicit
    // button consent is covered by both real-transfer close tests above.
    std::fs::write(&destination, bytes).unwrap();
    view.update(cx, |view, cx| {
        view.arm_shutdown(id, cx);
        assert!(view.queue.item(id).unwrap().completion.is_armed());
        view.active = Some(id);
        view.apply_state(
            id,
            TransferState::Completed(DownloadReceipt {
                destination: destination.clone(),
                downloaded_bytes: bytes.len() as u64,
            }),
            cx,
        );
        assert!(view.queue.item(unfinished).unwrap().is_queued());
        assert!(!view.queue.item(id).unwrap().completion.is_armed());
        assert!(view.shutdown_id.is_none());
        assert!(view._shutdown_countdown.is_none());
        assert_eq!(
            view.completion_error.as_deref(),
            Some("Shutdown cancelled because other downloads are unfinished.")
        );
        assert!(!view.completion_tick(Instant::now() + Duration::from_secs(35), cx));
    });
    details.update(&mut child, |_, cx| {
        cx.emit(crate::download_details::RequestShutdown(id))
    });
    draw(&mut child);
    assert!(child.debug_bounds("confirm-poweroff").is_none());
    view.update(cx, |view, cx| {
        view.arm_shutdown(id, cx);
        assert!(!view.queue.item(id).unwrap().completion.is_armed());
        view.arm_shutdown(unfinished, cx);
        assert!(view.queue.item(unfinished).unwrap().completion.is_armed());
        view.active = Some(unfinished);
        view.apply_state(
            unfinished,
            TransferState::Failed("injected transfer failure".into()),
            cx,
        );
        assert!(!view.queue.item(unfinished).unwrap().completion.is_armed());
        assert!(view.active.is_none());
        assert!(view.shutdown_id.is_none());
        assert!(view.shutdown_remaining.is_none());
        assert!(view._shutdown_countdown.is_none());
        view.arm_shutdown(unfinished, cx);
        assert!(!view.queue.item(unfinished).unwrap().completion.is_armed());
        assert!(!view.completion_tick(Instant::now() + Duration::from_secs(35), cx));
    });
    assert_eq!(std::fs::read(&destination).unwrap(), bytes);
    assert!(!directory.path().join("unfinished.bin").exists());
}
