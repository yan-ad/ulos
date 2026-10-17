use super::*;
use gpui_kit::{
    PathPromptOptions,
    component::{button::Button, label::Label},
};
use ulos_core::migration::{self, MigrationFormat};

const MAX_IMPORT_PATH: usize = 16 * 1024;

impl DownloadManager {
    pub(crate) fn import_migration(
        &mut self,
        format: MigrationFormat,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.active.is_some() || self.removal_busy || self.queue.items.len() >= 500 {
            return;
        }
        let prompt = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Choose migration file".into()),
        });
        let owner = cx.weak_entity();
        let target = self.main_window;
        cx.spawn_in(window, async move |_, _cx| {
            let Ok(Ok(Some(paths))) = prompt.await else { return; };
            let Some(path) = paths.into_iter().next() else { return; };
            if path.to_string_lossy().len() > MAX_IMPORT_PATH { return; }
            let Ok(bytes) = tokio::fs::read(path).await else { return; };
            let Ok(batch) = migration::parse(&bytes, format) else { return; };
            let allowed = owner.update(_cx, |view, _| view.active.is_none() && !view.removal_busy).unwrap_or(false);
            if allowed {
                let owner = owner.clone();
                let count = batch.entries.len();
                let _ = target.update(_cx, |_, window, cx| window.open_dialog(cx, move |dialog, _, _| {
                    let cancel_owner = owner.clone();
                    let confirm_owner = owner.clone();
                    let entries = batch.entries.clone();
                    dialog.title("Import downloads?")
                        .child(Label::new(format!("Review found {count} HTTP download(s). They will be queued only; no download starts automatically.")))
                        .footer(gpui_kit::component::toolbar::Toolbar::new("migration-actions")
                            .content(Button::new("cancel-migration").label("Cancel").on_click(|_, window, cx| window.close_dialog(cx)))
                            .content(Button::new("confirm-migration").label("Queue downloads").on_click(move |_, window, cx| {
                                let _ = confirm_owner.update(cx, |view, cx| {
                                    for entry in &entries {
                                        if view.queue.items.len() >= 500 { break; }
                                        let name = ulos_core::naming::suggest_filename(&entry.source).unwrap_or_else(|_| "download.bin".into());
                                        let destination = view.paths.as_ref().and_then(|paths| paths.downloads.clone()).unwrap_or_else(|| std::path::PathBuf::from(".")).join(name);
                                        let _ = view.enqueue_with_options(entry.source.clone(), destination, false, ulos_core::transfer::TransferOptions::default(), cx);
                                    }
                                });
                                let _ = cancel_owner.update(cx, |view, cx| { view.notice = Some("Imported downloads were queued; nothing was started.".into()); cx.notify(); });
                                window.close_dialog(cx);
                            })))
                }));
            }
        }).detach();
    }

    pub(crate) fn export_migration(
        &mut self,
        format: MigrationFormat,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let sources: Vec<String> = self
            .queue
            .items
            .iter()
            .filter(|item| self.queue.selected.is_empty() || self.queue.selected.contains(&item.id))
            .filter(|item| !item.is_torrent && !item.source().is_empty())
            .map(|item| item.source().to_owned())
            .collect();
        if sources.is_empty() {
            window.push_notification("Select an HTTP download before exporting.", cx);
            return;
        }
        let extension = if format == MigrationFormat::IdmEf2 {
            "ef2"
        } else {
            "txt"
        };
        let prompt = cx.prompt_for_new_path(
            std::path::Path::new("."),
            Some(&format!("ulos-export.{extension}")),
        );
        cx.spawn_in(window, async move |_, _cx| {
            let Ok(Ok(Some(path))) = prompt.await else {
                return;
            };
            let Ok(bytes) = migration::encode(&sources, format) else {
                return;
            };
            let _ = tokio::fs::write(path, bytes).await;
        })
        .detach();
    }
}
