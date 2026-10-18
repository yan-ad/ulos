use super::RowCommand;
use crate::manager::tests::{draw, mount};
use crate::{
    manager::DownloadManager,
    queue::{DownloadId, SortColumn},
};
use gpui_kit::{Entity, MouseButton, TestAppContext, VisualTestContext, point, px};
use std::time::Instant;
use ulos_core::tasks::TransferState;

#[gpui::test]
fn row_menu_filter_changes_keep_target_and_escape_preserves_selection(cx: &mut TestAppContext) {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let directory = tempfile::tempdir().unwrap();
    let (_root, view, cx) = mount(cx, &runtime);
    let (first, second) = queued_pair(&view, directory.path(), cx);
    right_click(&view, second, cx);
    cx.simulate_keystrokes("escape");
    draw(cx);
    view.read_with(cx, |view, _| {
        assert!(view.detail_windows.is_empty() && view.pending_removal.is_none());
        assert_eq!(
            view.queue.selected.iter().copied().collect::<Vec<_>>(),
            vec![first]
        );
    });
    right_click(&view, second, cx);
    view.update(cx, |view, cx| {
        view.queue.search = "first".into();
        view.sync_table(cx);
        cx.notify();
    });
    draw(cx);
    cx.simulate_keystrokes("down enter");
    draw(cx);
    view.read_with(cx, |view, _| {
        assert!(view.detail_windows.contains_key(&second));
        assert!(!view.detail_windows.contains_key(&first));
        assert_eq!(
            view.queue.selected.iter().copied().collect::<Vec<_>>(),
            vec![first]
        );
    });
}

#[gpui::test]
fn row_menu_removal_cancel_reopen_cycles_keep_real_dialog_clicks(cx: &mut TestAppContext) {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let directory = tempfile::tempdir().unwrap();
    let saved = directory.path().join("second.bin");
    std::fs::write(&saved, b"keep after repeated cancellation").unwrap();
    let (_root, view, cx) = mount(cx, &runtime);
    // Exercise real hit testing at rest, not the legacy wall-clock entrance.
    cx.update(|_, cx| cx.set_reduce_motion(true));
    let (first, second) = queued_pair(&view, directory.path(), cx);

    for _ in 0..8 {
        right_click(&view, second, cx);
        let remove_intent = capture_remove(&view, second, cx);
        activate_remove(&remove_intent, cx);
        view.read_with(cx, |view, _| {
            assert!(view.removal_open);
            assert_eq!(view.pending_removal, Some(vec![second]));
        });

        let cancel = dialog_button(cx, "cancel-removal");
        cx.simulate_click(cancel.center(), Default::default());
        assert_removal_cancelled(&view, second, cx);
    }

    view.read_with(cx, |view, _| {
        assert!(view.queue.item(first).is_some());
        assert!(view.queue.item(second).is_some());
        assert!(view.queue.selected.contains(&first));
    });
    right_click(&view, second, cx);
    let remove_intent = capture_remove(&view, second, cx);
    activate_remove(&remove_intent, cx);
    view.read_with(cx, |view, _| {
        assert!(view.removal_open);
        assert_eq!(view.pending_removal, Some(vec![second]));
    });
    let remove = dialog_button(cx, "remove-records");
    cx.simulate_click(remove.center(), Default::default());
    draw(cx);
    view.read_with(cx, |view, _| {
        assert!(!view.removal_open && view.pending_removal.is_none());
        assert!(view.queue.item(second).is_none());
        assert!(view.queue.item(first).is_some() && view.queue.selected.contains(&first));
        assert!(view.active.is_none());
    });
    assert_eq!(
        std::fs::read(saved).unwrap(),
        b"keep after repeated cancellation"
    );
}

fn capture_remove(
    view: &Entity<DownloadManager>,
    id: DownloadId,
    cx: &mut VisualTestContext,
) -> gpui_kit::component::menu::PopupMenuItem {
    cx.update(|window, cx| {
        view.read(cx).row_menu_item(
            id,
            RowCommand::Remove,
            "Remove…",
            window.window_handle(),
            view.downgrade(),
        )
    })
}

fn activate_remove(item: &gpui_kit::component::menu::PopupMenuItem, cx: &mut VisualTestContext) {
    // Menu keyboard selection can depend on pointer hover after window snapping.
    // Exercise the exact production callback, captured before sorting, rather
    // than assuming a fixed arrow-key count. Other tests cover popup navigation.
    cx.simulate_keystrokes("escape");
    draw(cx);
    let gpui_kit::component::menu::PopupMenuItem::Item {
        label,
        disabled,
        handler: Some(handler),
        ..
    } = item
    else {
        panic!("Remove must be a real enabled menu action");
    };
    assert_eq!(label.as_ref(), "Remove…");
    assert!(!disabled);
    cx.update(|window, cx| handler(&gpui_kit::ClickEvent::default(), window, cx));
    draw(cx);
}

fn dialog_button(
    cx: &mut VisualTestContext,
    selector: &'static str,
) -> gpui_kit::Bounds<gpui_kit::Pixels> {
    let mut previous = None;
    for _ in 0..20 {
        draw(cx);
        if let Some(bounds) = cx.debug_bounds(selector) {
            // Alert dialogs use the legacy wall-clock entrance animation.  A
            // debug registration can therefore exist while its position and
            // hitbox are still changing between input frames.  Require
            // two settled snapshots with reduced motion enabled instead of
            // clicking the first registration we happen to observe.  Advancing
            // the deterministic clock alone cannot settle legacy animations.
            if previous == Some(bounds) {
                return bounds;
            }
            previous = Some(bounds);
        } else {
            previous = None;
        }
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(50));
    }
    panic!("Dialog control did not settle: {selector}");
}

fn assert_removal_cancelled(
    view: &Entity<DownloadManager>,
    second: DownloadId,
    cx: &mut VisualTestContext,
) {
    view.read_with(cx, |view, _| {
        assert!(!view.removal_open, "Cancel must close the confirmation");
        assert!(
            view.pending_removal.is_none(),
            "Cancel must clear its target"
        );
        assert!(view.queue.item(second).is_some());
    });
    for _ in 0..20 {
        draw(cx);
        if cx.debug_bounds("cancel-removal").is_none()
            && cx.debug_bounds("remove-records").is_none()
            && cx.debug_bounds("dialog-layer").is_none()
        {
            return;
        }
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(50));
    }
    panic!("closing dialog did not settle");
}

fn queued_pair(
    view: &Entity<DownloadManager>,
    directory: &std::path::Path,
    cx: &mut VisualTestContext,
) -> (DownloadId, DownloadId) {
    view.update(cx, |view, cx| {
        let first = view
            .enqueue(
                "https://example.invalid/first".into(),
                directory.join("first.bin"),
                false,
                cx,
            )
            .unwrap();
        let second = view
            .enqueue(
                "https://example.invalid/second".into(),
                directory.join("second.bin"),
                false,
                cx,
            )
            .unwrap();
        view.select_item(first, true, cx);
        (first, second)
    })
}

fn right_click(view: &Entity<DownloadManager>, id: DownloadId, cx: &mut VisualTestContext) {
    assert_eq!(id, 2, "fixture target");
    draw(cx);
    let bounds = cx
        .debug_bounds("download-name-2")
        .expect("visible download");
    cx.simulate_mouse_down(bounds.center(), MouseButton::Right, Default::default());
    cx.simulate_mouse_up(bounds.center(), MouseButton::Right, Default::default());
    draw(cx);
    view.read_with(cx, |view, _| assert!(!view.queue.selected.contains(&id)));
}

#[gpui::test]
fn row_menu_details_targets_unchecked_download_without_changing_checkboxes(
    cx: &mut TestAppContext,
) {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let directory = tempfile::tempdir().unwrap();
    let (_root, view, cx) = mount(cx, &runtime);
    let (first, second) = queued_pair(&view, directory.path(), cx);
    right_click(&view, second, cx);
    view.read_with(cx, |view, _| assert!(view.detail_windows.is_empty()));
    cx.simulate_keystrokes("down enter");
    draw(cx);
    view.read_with(cx, |view, _| {
        assert!(view.detail_windows.contains_key(&second));
        assert!(!view.detail_windows.contains_key(&first));
        assert_eq!(
            view.queue.selected.iter().copied().collect::<Vec<_>>(),
            vec![first]
        );
        assert!(view.queue.items.iter().all(|item| item.is_queued()));
    });
}

#[gpui::test]
fn row_menu_removal_confirmation_targets_id_after_sort_and_keeps_files(cx: &mut TestAppContext) {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let directory = tempfile::tempdir().unwrap();
    let saved = directory.path().join("second.bin");
    std::fs::write(&saved, b"keep these bytes").unwrap();
    let (_root, view, cx) = mount(cx, &runtime);
    cx.update(|_, cx| cx.set_reduce_motion(true));
    let (first, second) = queued_pair(&view, directory.path(), cx);
    right_click(&view, second, cx);
    let remove_intent = capture_remove(&view, second, cx);
    view.update(cx, |view, cx| {
        view.sort_table(
            SortColumn::Name,
            gpui_kit::component::table::ColumnSort::Descending,
            cx,
        )
    });
    draw(cx);
    activate_remove(&remove_intent, cx);
    view.read_with(cx, |view, _| {
        assert!(view.removal_open);
        assert_eq!(view.pending_removal, Some(vec![second]));
    });
    view.read_with(cx, |view, _| {
        assert!(view.removal_open);
        assert_eq!(view.pending_removal, Some(vec![second]));
        assert!(view.queue.selected.contains(&first));
    });
    let cancel = dialog_button(cx, "cancel-removal");
    cx.simulate_click(cancel.center(), Default::default());
    draw(cx);
    assert_removal_cancelled(&view, second, cx);
    right_click(&view, second, cx);
    let remove_intent = capture_remove(&view, second, cx);
    activate_remove(&remove_intent, cx);
    view.read_with(cx, |view, _| {
        assert!(view.removal_open, "Remove must open the confirmation");
        assert_eq!(view.pending_removal, Some(vec![second]));
        assert!(
            view.detail_windows.is_empty(),
            "Do not accidentally activate Details"
        );
    });
    let remove = dialog_button(cx, "remove-records");
    cx.simulate_click(remove.center(), Default::default());
    draw(cx);
    view.read_with(cx, |view, _| {
        assert!(!view.removal_open && view.pending_removal.is_none());
        assert!(view.queue.item(first).is_some() && view.queue.selected.contains(&first));
        assert!(view.queue.item(second).is_none());
        assert!(view.active.is_none());
    });
    assert_eq!(std::fs::read(saved).unwrap(), b"keep these bytes");
}

#[gpui::test]
fn row_menu_stale_target_and_blank_space_do_not_act_on_checked_row(cx: &mut TestAppContext) {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let directory = tempfile::tempdir().unwrap();
    let (_root, view, cx) = mount(cx, &runtime);
    let (first, second) = queued_pair(&view, directory.path(), cx);
    right_click(&view, second, cx);
    view.update(cx, |view, cx| view.finish_record_removal(&[second], cx));
    draw(cx);
    cx.simulate_keystrokes("down enter");
    draw(cx);
    view.read_with(cx, |view, _| {
        assert!(view.detail_windows.is_empty() && view.active.is_none());
        assert!(view.queue.item(first).is_some() && view.queue.selected.contains(&first));
    });
    let table = cx.debug_bounds("download-table").unwrap();
    let blank = point(table.left() + px(100.), table.bottom() - px(30.));
    cx.simulate_mouse_down(blank, MouseButton::Right, Default::default());
    cx.simulate_mouse_up(blank, MouseButton::Right, Default::default());
    draw(cx);
    cx.simulate_keystrokes("down enter");
    draw(cx);
    view.read_with(cx, |view, _| {
        assert!(view.detail_windows.is_empty() && view.queue.item(first).is_some())
    });
}

#[gpui::test]
fn row_menu_rechecks_active_pause_retry_and_source_less_history(cx: &mut TestAppContext) {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let directory = tempfile::tempdir().unwrap();
    let (_root, view, cx) = mount(cx, &runtime);
    let (first, second) = queued_pair(&view, directory.path(), cx);
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            assert!(view.row_command_enabled(second, RowCommand::Start));
            assert!(!view.row_command_enabled(second, RowCommand::Retry));
            view.queue.item_mut(second).unwrap().apply_state(
                TransferState::Downloading(ulos_core::DownloadProgress {
                    downloaded_bytes: 0,
                    total_bytes: Some(10),
                }),
                Instant::now(),
            );
            view.active = Some(second);
            assert!(!view.row_command_enabled(first, RowCommand::Start));
            assert!(!view.row_command_enabled(second, RowCommand::Pause));
            view.queue.item_mut(second).unwrap().telemetry.resume =
                ulos_core::transfer::ResumeCapability::Supported;
            assert!(view.row_command_enabled(second, RowCommand::Pause));
            assert!(!view.row_command_enabled(second, RowCommand::Resume));
            assert!(!view.row_command_enabled(second, RowCommand::Remove));
            view.queue.item_mut(second).unwrap().cancelling = true;
            assert!(!view.row_command_enabled(second, RowCommand::Stop));
            assert!(!view.row_command_enabled(second, RowCommand::Pause));
            view.active = None;
            view.queue
                .item_mut(second)
                .unwrap()
                .apply_state(TransferState::Cancelled, Instant::now());
            assert!(view.row_command_enabled(second, RowCommand::Retry));
            assert!(
                view.queue
                    .item_mut(second)
                    .unwrap()
                    .reset_for_retry(ulos_core::transfer::CollisionPolicy::Reject)
            );
            view.queue.item_mut(second).unwrap().apply_state(
                TransferState::Completed(ulos_core::DownloadReceipt {
                    destination: directory.path().join("second.bin"),
                    downloaded_bytes: 10,
                }),
                Instant::now(),
            );
            assert!(!view.row_command_enabled(second, RowCommand::Retry));
            view.run_row_command(second, RowCommand::Retry, window, cx);
            assert!(view.active.is_none());
            assert!(view.row_command_enabled(second, RowCommand::Reveal));
            view.finish_record_removal(&[second], cx);
            for command in [
                RowCommand::Details,
                RowCommand::Start,
                RowCommand::Pause,
                RowCommand::Resume,
                RowCommand::Stop,
                RowCommand::Retry,
                RowCommand::Reveal,
                RowCommand::Remove,
            ] {
                assert!(!view.row_command_enabled(second, command));
                view.run_row_command(second, command, window, cx);
            }
            assert!(view.queue.item(first).is_some() && view.queue.selected.contains(&first));
            let history = view.queue.history_records();
            view.queue = crate::queue::SessionQueue::default();
            view.queue.restore_history(history).unwrap();
            assert!(!view.queue.item(first).unwrap().has_source());
            assert!(!view.row_command_enabled(first, RowCommand::Start));
            assert!(!view.row_command_enabled(first, RowCommand::Retry));
            view.run_row_command(first, RowCommand::Start, window, cx);
            view.run_row_command(first, RowCommand::Retry, window, cx);
            assert!(view.active.is_none());
        })
    });
}
