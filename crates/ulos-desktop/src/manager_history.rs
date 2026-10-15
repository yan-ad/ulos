use super::*;
use gpui_kit::component::{button::Button, dialog::DialogFooter};

impl DownloadManager {
    pub(crate) fn initialize_history(
        &mut self,
        paths: Option<PlatformPaths>,
        records: Vec<HistoryRecord>,
        writer: Option<HistoryWriter>,
        error: Option<String>,
        cx: &mut Context<Self>,
    ) {
        self.paths = paths;
        self.history_error = error;
        if let Err(error) = self.queue.restore_history(records) {
            self.history_error = Some(error.into());
            self.history = None;
        } else {
            self.history = writer;
        }
        if let Some(writer) = &self.history {
            let mut acknowledgements = writer.subscribe();
            self._history_updates = Some(cx.spawn(async move |view, cx| {
                loop {
                    cx.background_executor()
                        .timer(Duration::from_millis(250))
                        .await;
                    if matches!(acknowledgements.has_changed(), Ok(true)) {
                        let (_, result) = acknowledgements.borrow_and_update().clone();
                        if view
                            .update(cx, |view, cx| {
                                view.history_error = result.err();
                                cx.notify();
                            })
                            .is_err()
                        {
                            break;
                        }
                    } else if acknowledgements.has_changed().is_err() {
                        break;
                    }
                }
            }));
        }
        self.sync_table(cx);
        cx.notify();
    }

    pub(super) fn save_history(&self) {
        if let Some(writer) = &self.history {
            writer.save(self.queue.history_records());
        }
    }

    pub(super) fn confirm_record_removal(
        &mut self,
        ids: Vec<DownloadId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.removal_open || self.removal_busy || self.options_open {
            return;
        }
        let ids: Vec<_> = ids
            .into_iter()
            .filter(|id| {
                self.active != Some(*id)
                    && self.queue.item(*id).is_some_and(|item| !item.is_active())
            })
            .collect();
        if ids.is_empty() {
            return;
        }
        let can_delete = self
            .queue
            .items
            .iter()
            .any(|item| ids.contains(&item.id) && item.is_finished() && !item.output_directory);
        self.pending_removal = Some(ids.clone());
        self.removal_open = true;
        let owner = cx.weak_entity();
        window.open_alert_dialog(cx, move |alert, _, _| {
            let cancel_owner = owner.clone();
            let record_owner = owner.clone();
            let delete_owner = owner.clone();
            let close_owner = owner.clone();
            let record_ids = ids.clone();
            let delete_ids = ids.clone();
            alert.title("Remove downloads?")
                .description("Remove records only to keep files. Delete files permanently deletes completed regular files and removes their records. Torrent directories are kept. Active and paused downloads are excluded; failed deletions keep their records.")
                .width(px(540.))
                .on_close(move |_, _, cx| {
                    let _ = close_owner.update(cx, |view, cx| {
                        view.removal_open = false;
                        view.pending_removal = None;
                        cx.notify();
                    });
                })
                .footer(DialogFooter::new()
                    .child(Button::new("cancel-removal").debug_selector(|| "cancel-removal".into()).label("Cancel").on_click(move |_, window, cx| {
                        let _ = cancel_owner.update(cx, |view, cx| { view.removal_open = false; view.pending_removal = None; cx.notify(); });
                        window.close_dialog(cx);
                    }))
                    .child(Button::new("remove-records").debug_selector(|| "remove-records".into()).label("Remove records only").on_click(move |_, window, cx| {
                        let _ = record_owner.update(cx, |view, cx| {
                            if view.removal_open && view.pending_removal.as_ref() == Some(&record_ids) {
                                view.finish_record_removal(&record_ids, cx);
                                view.removal_open = false;
                                view.pending_removal = None;
                            }
                        });
                        window.close_dialog(cx);
                    }))
                    .child(Button::new("delete-download-files").debug_selector(|| "delete-download-files".into()).label("Delete files and records").disabled(!can_delete).on_click(move |_, window, cx| {
                        let _ = delete_owner.update(cx, |view, cx| {
                            if view.removal_open && view.pending_removal.as_ref() == Some(&delete_ids) {
                                view.removal_open = false;
                                view.pending_removal = None;
                                view.delete_files_and_records(delete_ids.clone(), cx);
                            }
                        });
                        window.close_dialog(cx);
                    })))
        });
    }

    pub(crate) fn finish_record_removal(&mut self, ids: &[DownloadId], cx: &mut Context<Self>) {
        if self.removal_busy {
            return;
        }
        self.cancel_all_shutdowns(cx);
        let ids: Vec<_> = ids
            .iter()
            .copied()
            .filter(|id| self.active != Some(*id))
            .collect();
        let removed = self.queue.remove_records(&ids);
        self.close_orphaned_status_windows(cx);
        self.save_history();
        self.sync_table(cx);
        self.notice = Some(format!(
            "Removed {removed} record(s). Files were not deleted."
        ));
        cx.notify();
    }

    fn delete_files_and_records(&mut self, ids: Vec<DownloadId>, cx: &mut Context<Self>) {
        if self.removal_busy {
            return;
        }
        self.cancel_all_shutdowns(cx);
        // Freeze starts/retries until the authorized destinations have been processed.
        let records: Vec<_> = ids
            .into_iter()
            .filter_map(|id| {
                let item = self.queue.item(id)?;
                if item.output_directory {
                    return None;
                }
                (self.active != Some(id) && !item.is_active())
                    .then(|| (id, item.destination.clone(), item.is_finished()))
            })
            .collect();
        self.removal_busy = true;
        let task = self.runtime.spawn(async move {
            let mut removed = Vec::new();
            let mut failed = 0;
            for (id, destination, completed) in records {
                if !completed
                    || crate::storage::delete_downloaded_file(destination)
                        .await
                        .is_ok()
                {
                    removed.push(id);
                } else {
                    failed += 1;
                }
            }
            (removed, failed)
        });
        self._file_removal = Some(cx.spawn(async move |view, cx| {
            let result = task.await;
            let _ = view.update(cx, |view, cx| {
                view.removal_busy = false;
                match result {
                    Ok((ids, failed)) => {
                        view.finish_record_removal(&ids, cx);
                        view.notice = Some(if failed == 0 {
                            "Removed records and deleted the confirmed completed files.".into()
                        } else {
                            format!("{failed} file deletion(s) failed. Those records were kept; symbolic links and directories are never deleted.")
                        });
                    }
                    Err(_) => view.notice = Some("File deletion worker stopped. Records were kept; check the files before trying again.".into()),
                }
                view.advance(cx);
                cx.notify();
            });
        }));
        cx.notify();
    }
}
