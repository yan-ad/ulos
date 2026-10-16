use super::RowCommand;
use crate::manager::tests::{draw, mount};
use crate::{
    manager::DownloadManager,
    queue::{DownloadId, SortColumn},
};
use gpui_kit::{Entity, MouseButton, TestAppContext, VisualTestContext, point, px};
use std::time::Instant;
use ulos_core::tasks::TransferState;

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
    draw(cx);
    let bounds = cx
        .debug_bounds(&format!("download-name-{id}"))
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
    let (first, second) = queued_pair(&view, directory.path(), cx);
    right_click(&view, second, cx);
    view.update(cx, |view, cx| {
        view.sort_table(
            SortColumn::Name,
            gpui_kit::component::table::ColumnSort::Descending,
            cx,
        )
    });
    draw(cx);
    cx.simulate_keystrokes("end enter");
    draw(cx);
    view.read_with(cx, |view, _| {
        assert!(view.removal_open);
        assert_eq!(view.pending_removal, Some(vec![second]));
        assert!(view.queue.selected.contains(&first));
    });
    let cancel = cx.debug_bounds("cancel-removal").unwrap();
    cx.simulate_click(cancel.center(), Default::default());
    draw(cx);
    view.read_with(cx, |view, _| assert!(view.queue.item(second).is_some()));
    right_click(&view, second, cx);
    cx.simulate_keystrokes("end enter");
    draw(cx);
    let remove = cx.debug_bounds("remove-records").unwrap();
    cx.simulate_click(remove.center(), Default::default());
    draw(cx);
    view.read_with(cx, |view, _| {
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
        })
    });
}
