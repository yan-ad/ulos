use super::*;
use gpui_kit::{
    PathPromptOptions,
    component::{
        button::Button,
        dialog::DialogFooter,
        menu::{PopupMenu, PopupMenuItem},
    },
};
use std::path::{Path, PathBuf};
use ulos_core::migration::{self as codec, MigrationBatch, MigrationFormat};

const MAX_IMPORT_PATH: usize = 16 * 1024;

/// One retained observer and one single-use review token per manager.
#[derive(Default)]
pub(super) struct MigrationState {
    pub(super) busy: bool,
    generation: u64,
    task: Option<Task<()>>,
}
impl MigrationState {
    pub(super) fn reset(&mut self) {
        self.busy = false;
        self.generation = self.generation.wrapping_add(1);
        self.task = None;
    }
    fn begin(&mut self) -> u64 {
        self.task = None;
        self.generation = self.generation.wrapping_add(1);
        self.busy = true;
        self.generation
    }
    fn accepts(&self, token: u64) -> bool {
        self.busy && self.generation == token
    }
    // Do not drop the currently executing observer from inside its own update.
    fn finish(&mut self) {
        self.busy = false;
    }
}

fn import_plan(
    queue: &SessionQueue,
    batch: &MigrationBatch,
    folder: &Path,
) -> Result<Vec<PathBuf>, &'static str> {
    if batch.entries.is_empty()
        || queue.items.len().saturating_add(batch.entries.len()) > crate::queue::MAX_ITEMS
    {
        return Err("The complete import does not fit in the session queue.");
    }
    let mut destinations = Vec::with_capacity(batch.entries.len());
    for entry in &batch.entries {
        let name = ulos_core::naming::suggest_filename(&entry.source)
            .unwrap_or_else(|_| "download.bin".into());
        let destination = (0..=10_000)
            .map(|number| {
                let candidate = if number == 0 {
                    name.clone()
                } else {
                    // Prefix survives the portable name's byte bound, unlike an appended suffix.
                    ulos_core::naming::sanitize_filename(&format!("{number}-{name}"))
                };
                folder.join(candidate)
            })
            .find(|candidate| {
                !destinations.contains(candidate)
                    && !queue.items.iter().any(|item| {
                        item.destination == *candidate && (item.is_queued() || item.is_active())
                    })
            })
            .ok_or("Could not allocate distinct bounded import filenames.")?;
        destinations.push(destination);
    }
    Ok(destinations)
}

/// No dispatch, filesystem access or history write; rollback leaves existing records untouched.
fn append_import(
    queue: &mut SessionQueue,
    batch: &MigrationBatch,
    folder: &Path,
) -> Result<(), &'static str> {
    let plan = import_plan(queue, batch, folder)?;
    let original_len = queue.items.len();
    for (entry, destination) in batch.entries.iter().zip(plan) {
        match queue.add(entry.source.clone(), destination) {
            Ok(id) => {
                if let Some(item) = queue.item_mut(id) {
                    item.options = TransferOptions {
                        connections: 4,
                        collision: CollisionPolicy::Reject,
                        ..Default::default()
                    };
                }
            }
            Err(error) => {
                // IDs are monotonic and may be consumed, but no partial records escape.
                queue.items.truncate(original_len);
                return Err(error);
            }
        }
    }
    Ok(())
}

impl DownloadManager {
    pub(super) fn migration_menu(
        menu: PopupMenu,
        window: &mut Window,
        cx: &mut Context<PopupMenu>,
        owner: WeakEntity<Self>,
    ) -> PopupMenu {
        let handle = window.window_handle();
        let import_owner = owner.clone();
        let menu = menu
            .separator()
            .submenu("Import", window, cx, move |mut submenu, _, _| {
                for (label, format) in [
                    ("From IDM export file…", MigrationFormat::IdmEf2),
                    ("From text file…", MigrationFormat::Text),
                ] {
                    let owner = import_owner.clone();
                    submenu = submenu.item(PopupMenuItem::new(label).on_click(move |_, _, cx| {
                        let owner = owner.clone();
                        cx.defer(move |cx| {
                            let _ = handle.update(cx, |_, window, cx| {
                                let _ = owner.update(cx, |view, cx| {
                                    view.import_migration(format, window, cx)
                                });
                            });
                        });
                    }));
                }
                submenu
            });
        menu.submenu("Export", window, cx, move |mut submenu, _, _| {
            for (label, format) in [
                ("To IDM export file…", MigrationFormat::IdmEf2),
                ("To text file…", MigrationFormat::Text),
            ] {
                let owner = owner.clone();
                submenu = submenu.item(PopupMenuItem::new(label).on_click(move |_, _, cx| {
                    let owner = owner.clone();
                    cx.defer(move |cx| {
                        let _ = handle.update(cx, |_, window, cx| {
                            let _ = owner
                                .update(cx, |view, cx| view.export_migration(format, window, cx));
                        });
                    });
                }));
            }
            submenu
        })
    }

    pub(super) fn migration_available(&self, window: &Window, cx: &App) -> bool {
        window.window_handle() == self.main_window
            && cx.windows().contains(&self.main_window)
            && !self.options_open
            && !self.about_open
            && !self.removal_open
            && !self.removal_busy
            && !self.torrent_picker_open
            && self.add_windows.is_empty()
            && self.torrent_windows.is_empty()
            && self.detail_windows.is_empty()
            && self.scheduler_window.is_none()
    }
    fn migration_import_idle(&self) -> bool {
        self.active.is_none()
            && !self.queue_running
            && !self.scheduler.any_running()
            && !self.scheduler.has_enabled_schedule()
    }
    fn migration_finish(&mut self, token: u64, notice: Option<String>, cx: &mut Context<Self>) {
        if self.migration.accepts(token) {
            self.migration.finish();
            self.notice = notice;
            cx.notify();
        }
    }
    pub(crate) fn import_migration(
        &mut self,
        format: MigrationFormat,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.migration.busy || !self.migration_available(window, cx) {
            return;
        }
        if !self.migration_import_idle() {
            self.notice =
                Some("Stop all queues and disable schedules before importing downloads.".into());
            cx.notify();
            return;
        }
        let token = self.migration.begin();
        let prompt = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Choose migration file".into()),
        });
        let runtime = self.runtime.clone();
        self.migration.task = Some(cx.spawn_in(window, async move |view, cx| {
            let Ok(Ok(Some(paths))) = prompt.await else {
                let _ = view.update(cx, |view, cx| view.migration_finish(token, None, cx)); return;
            };
            let Some(path) = paths.into_iter().next().filter(|path| path.as_os_str().len() <= MAX_IMPORT_PATH) else {
                let _ = view.update(cx, |view, cx| view.migration_finish(token, Some("Migration path is too long.".into()), cx)); return;
            };
            let allowed = view.update_in(cx, |view, window, cx| view.migration.accepts(token) && view.migration_available(window, cx) && view.migration_import_idle()).unwrap_or(false);
            if !allowed { let _ = view.update(cx, |view, cx| view.migration_finish(token, Some("Import cancelled because manager state changed.".into()), cx)); return; }
            let worker = runtime.spawn_blocking(move || codec::read_file(&path, format));
            let result = worker.await.unwrap_or(Err(codec::MigrationError::Read));
            let batch = match result { Ok(batch) => batch, Err(error) => {
                let _ = view.update(cx, |view, cx| view.migration_finish(token, Some(error.to_string()), cx)); return;
            }};
            let folder_prompt = view.update_in(cx, |view, window, cx| {
                if !view.migration.accepts(token) || !view.migration_available(window, cx) || !view.migration_import_idle() { return None; }
                Some(cx.prompt_for_paths(PathPromptOptions { files: false, directories: true, multiple: false, prompt: Some("Choose destination folder for imported downloads".into()) }))
            }).ok().flatten();
            let Some(folder_prompt) = folder_prompt else { let _ = view.update(cx, |view, cx| view.migration_finish(token, Some("Import cancelled because manager state changed.".into()), cx)); return; };
            let Ok(Ok(Some(folders))) = folder_prompt.await else { let _ = view.update(cx, |view, cx| view.migration_finish(token, None, cx)); return; };
            let Some(folder) = folders.into_iter().next().filter(|path| path.is_absolute() && path.as_os_str().len() <= MAX_IMPORT_PATH) else {
                let _ = view.update(cx, |view, cx| view.migration_finish(token, Some("Choose a bounded absolute destination folder.".into()), cx)); return;
            };
            let _ = view.update_in(cx, |view, window, cx| {
                if !view.migration.accepts(token) || !view.migration_available(window, cx) || !view.migration_import_idle() {
                    view.migration_finish(token, Some("Import cancelled because manager state changed.".into()), cx); return;
                }
                if let Err(error) = import_plan(&view.queue, &batch, &folder) {
                    view.migration_finish(token, Some(error.into()), cx); return;
                }
                let owner = cx.weak_entity();
                let count = batch.entries.len();
                let ignored = batch.ignored_fields;
                // Escaped path only: never render source URLs, signed queries or header values.
                let destination = folder.to_string_lossy().chars().flat_map(char::escape_default).collect::<String>();
                window.open_alert_dialog(cx, move |alert, _, _| {
                    let cancel_owner = owner.clone(); let close_owner = owner.clone(); let confirm_owner = owner.clone();
                    let batch = batch.clone(); let folder = folder.clone();
                    alert.title("Review migration")
                        .description(format!("{count} HTTP download(s) to: {destination}\n{ignored} recognized referer/User-Agent field(s) will be ignored. Authentication, cookies, POST data and unknown fields are rejected. Duplicate queue filenames are numbered. Existing files are never replaced. Confirm adds records to a stopped queue only; nothing starts automatically."))
                        .on_close(move |_, _, cx| { let _ = close_owner.update(cx, |view, cx| view.migration_finish(token, None, cx)); })
                        .footer(DialogFooter::new()
                            .child(Button::new("cancel-migration").debug_selector(|| "cancel-migration".into()).label("Cancel").on_click(move |_, window, cx| {
                                let _ = cancel_owner.update(cx, |view, cx| view.migration_finish(token, None, cx)); window.close_dialog(cx);
                            }))
                            .child(Button::new("confirm-migration").debug_selector(|| "confirm-migration".into()).label("Add to stopped queue").on_click(move |_, window, cx| {
                                let _ = confirm_owner.update(cx, |view, cx| {
                                    if !view.migration.accepts(token) { return; }
                                    if !view.migration_available(window, cx) || !view.migration_import_idle() {
                                        view.migration_finish(token, Some("Import cancelled: stop all queues, disable schedules and close other windows first.".into()), cx); return;
                                    }
                                    // Consume consent before any mutation; repeated callbacks cannot import twice.
                                    view.migration.finish();
                                    match append_import(&mut view.queue, &batch, &folder) {
                                        Ok(()) => {
                                            view.cancel_all_shutdowns(cx); view.sync_table(cx); view.save_history();
                                            view.notice = Some(format!("Imported {count} download(s) into the stopped queue; nothing was started."));
                                        }
                                        Err(error) => view.notice = Some(error.into()),
                                    }
                                    cx.notify();
                                });
                                window.close_dialog(cx);
                            })))
                });
                cx.notify();
            });
        }));
        cx.notify();
    }

    pub(crate) fn export_migration(
        &mut self,
        format: MigrationFormat,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.migration.busy || !self.migration_available(window, cx) {
            return;
        }
        let candidates: Vec<_> = self
            .queue
            .items
            .iter()
            .filter(|item| self.queue.selected.is_empty() || self.queue.selected.contains(&item.id))
            .collect();
        let sources: Vec<String> = candidates
            .iter()
            .filter(|item| !item.is_torrent && !item.source().is_empty())
            .map(|item| item.source().to_owned())
            .collect();
        let skipped = candidates.len() - sources.len();
        if let Err(error) = codec::encode(&sources, format) {
            self.notice = Some(error.to_string());
            cx.notify();
            return;
        }
        let token = self.migration.begin();
        let owner = cx.weak_entity();
        window.open_alert_dialog(cx, move |alert, _, _| {
            let cancel_owner = owner.clone(); let close_owner = owner.clone(); let confirm_owner = owner.clone(); let sources = sources.clone();
            alert.title("Export source URLs?")
                .description(format!("Export {} HTTP source URL(s); {skipped} torrent or source-less history record(s) skipped. The file contains raw URLs, including private signed query values. Store and share it carefully. No cookies, authorization headers or transfer settings are exported. Choose a NEW filename: existing files are never replaced.", sources.len()))
                .on_close(move |_, _, cx| { let _ = close_owner.update(cx, |view, cx| {
                    view.migration_finish(token, None, cx);
                }); })
                .footer(DialogFooter::new()
                    .child(Button::new("cancel-export").label("Cancel").on_click(move |_, window, cx| {
                        let _ = cancel_owner.update(cx, |view, cx| view.migration_finish(token, None, cx)); window.close_dialog(cx);
                    }))
                    .child(Button::new("confirm-export").label("Choose new file").on_click(move |_, window, cx| {
                        let _ = confirm_owner.update(cx, |view, cx| {
                            if !view.migration.accepts(token) || !view.migration_available(window, cx) { return; }
                            view.start_migration_export(token, sources.clone(), format, window, cx);
                        });
                        window.close_dialog(cx);
                    })))
        });
        cx.notify();
    }
    fn start_migration_export(
        &mut self,
        token: u64,
        sources: Vec<String>,
        format: MigrationFormat,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // New token consumes warning consent; close/repeated callbacks hold the old token.
        debug_assert!(self.migration.accepts(token));
        let token = self.migration.begin();
        let extension = if format == MigrationFormat::IdmEf2 {
            "ef2"
        } else {
            "txt"
        };
        let prompt =
            cx.prompt_for_new_path(Path::new("."), Some(&format!("ulos-export.{extension}")));
        let runtime = self.runtime.clone();
        self.migration.task = Some(cx.spawn_in(window, async move |view, cx| {
            let Ok(Ok(Some(path))) = prompt.await else {
                let _ = view.update(cx, |view, cx| view.migration_finish(token, None, cx));
                return;
            };
            let allowed = view
                .update_in(cx, |view, window, cx| {
                    view.migration.accepts(token) && view.migration_available(window, cx)
                })
                .unwrap_or(false);
            if !allowed {
                let _ = view.update(cx, |view, cx| {
                    view.migration_finish(
                        token,
                        Some("Export cancelled because manager state changed.".into()),
                        cx,
                    )
                });
                return;
            }
            let worker =
                runtime.spawn_blocking(move || codec::export_file(&path, &sources, format));
            let result = worker.await.unwrap_or(Err(codec::MigrationError::Write));
            let notice = match result {
                Ok(()) => "Migration export saved without replacing an existing file.".into(),
                Err(error) => error.to_string(),
            };
            let _ = view.update(cx, |view, cx| {
                view.migration_finish(token, Some(notice), cx)
            });
        }));
    }
}

#[cfg(test)]
#[path = "manager_migration_tests.rs"]
mod tests;
