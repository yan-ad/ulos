use super::*;
use crate::{add_torrent::TorrentRequest, native_windows::NativeTorrentWindow};
use std::path::PathBuf;

impl DownloadManager {
    fn can_set_torrent_controls(&self, id: DownloadId) -> bool {
        self.queue.item(id).is_some_and(|item| {
            item.is_torrent
                && item.torrent_settings.allow_upload
                && !item.cancelling
                && (self.active == Some(id) || item.is_queued() || item.can_retry())
        })
    }
    pub(crate) fn set_torrent_limits(
        &mut self,
        id: DownloadId,
        download: Option<u64>,
        upload: Option<u64>,
        cx: &mut Context<Self>,
    ) {
        if !self.can_set_torrent_controls(id)
            || [download, upload]
                .into_iter()
                .flatten()
                .any(|rate| !(1..=1_073_741_824).contains(&rate))
        {
            return;
        }
        if self.active == Some(id)
            && !self
                .transfer
                .as_ref()
                .is_some_and(|task| task.set_torrent_limits(download, upload))
        {
            return;
        }
        if let Some(item) = self.queue.item_mut(id) {
            item.torrent_settings.download_bytes_per_second = download;
            item.torrent_settings.upload_bytes_per_second = upload;
        }
        self.refresh_details(cx);
        cx.notify();
    }
    pub(crate) fn set_torrent_peers(
        &mut self,
        id: DownloadId,
        peers: usize,
        cx: &mut Context<Self>,
    ) {
        if !self.can_set_torrent_controls(id) || !(1..=10000).contains(&peers) {
            return;
        }
        if self.active == Some(id)
            && !self
                .transfer
                .as_ref()
                .is_some_and(|task| task.set_peer_limit(peers))
        {
            return;
        }
        if let Some(item) = self.queue.item_mut(id) {
            item.torrent_settings.peer_limit = peers;
        }
        self.refresh_details(cx);
        cx.notify();
    }

    pub(crate) fn watch_torrent_intake(
        &mut self,
        inbox: std::rc::Rc<std::cell::RefCell<Vec<PathBuf>>>,
        error: std::rc::Rc<std::cell::RefCell<Option<String>>>,
        cx: &mut Context<Self>,
    ) {
        self._torrent_intake = Some(cx.spawn(async move |view, cx| {
            loop {
                let paths = std::mem::take(&mut *inbox.borrow_mut());
                let problem = error.borrow_mut().take();
                if (!paths.is_empty() || problem.is_some())
                    && view
                        .update(cx, |view, cx| {
                            if !paths.is_empty() {
                                view.offer_torrent_paths(paths, cx);
                            }
                            if let Some(problem) = problem {
                                view.notice = Some(problem);
                                cx.notify();
                            }
                        })
                        .is_err()
                {
                    break;
                }
                cx.background_executor()
                    .timer(Duration::from_millis(100))
                    .await;
            }
        }));
    }
    pub(crate) fn open_torrent(&mut self, path: Option<PathBuf>, cx: &mut Context<Self>) {
        self.torrent_windows
            .retain(|_, child| child.upgrade().is_some());
        if self.add_windows.len() + self.torrent_windows.len() >= crate::intake::MAX_INTAKE_FILES {
            self.notice = Some("Close an Add window before opening another torrent.".into());
            cx.notify();
            return;
        }
        if self.removal_busy || self.options_open || self.about_open || self.removal_open {
            self.notice = Some("Finish the open dialog before adding a torrent.".into());
            cx.notify();
            return;
        }
        let owner = cx.entity();
        let runtime = self.runtime.clone();
        let directory = self
            .paths
            .as_ref()
            .and_then(|paths| paths.downloads.clone());
        let options = add_options(cx);
        match open_window(options, cx, move |window, cx| {
            cx.new(|cx| NativeTorrentWindow::new(owner, path, directory, runtime, window, cx))
        }) {
            Ok((handle, root)) => {
                let content = root.read(cx).content.downgrade();
                self.torrent_windows
                    .insert(handle.window_id(), ChildWindow { handle, content });
                #[cfg(any(target_os = "macos", test))]
                cx.emit(MenuStateChanged);
            }
            Err(_) => {
                self.notice = Some("Could not open a torrent review window.".into());
                cx.notify();
            }
        }
    }

    pub(crate) fn offer_torrent_paths(&mut self, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
        match crate::intake::torrent_paths(paths) {
            Ok(paths) => {
                if self.add_windows.len() + self.torrent_windows.len() + paths.len()
                    > crate::intake::MAX_INTAKE_FILES
                {
                    self.notice =
                        Some("Too many Add windows. Close drafts before adding this batch.".into());
                    cx.notify();
                    return;
                }
                for path in paths {
                    self.open_torrent(Some(path), cx);
                }
            }
            Err(error) => {
                self.notice = Some(error.into());
                cx.notify();
            }
        }
    }

    pub(crate) fn choose_torrent_files(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        if self.torrent_picker_open
            || self.removal_busy
            || self.options_open
            || self.about_open
            || self.removal_open
        {
            return;
        }
        let prompt = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some("Choose .torrent files to review".into()),
        });
        self.torrent_picker_open = true;
        self._torrent_picker = Some(cx.spawn(async move |view, cx| {
            let result = prompt.await;
            let _ = view.update(cx, |view, cx| {
                view.torrent_picker_open = false;
                match result {
                    Ok(Ok(Some(paths))) if !paths.is_empty() => view.offer_torrent_paths(paths, cx),
                    Ok(Ok(_)) => (),
                    _ => {
                        view.notice = Some(
                            "Could not open the torrent picker. Drop a .torrent file instead."
                                .into(),
                        );
                        cx.notify();
                    }
                }
            });
        }));
    }

    pub(crate) fn enqueue_torrent(
        &mut self,
        request: &TorrentRequest,
        cx: &mut Context<Self>,
    ) -> Result<DownloadId, &'static str> {
        if self.removal_busy {
            return Err("Wait for file deletion to finish.");
        }
        if request.options.collision == CollisionPolicy::Overwrite {
            return Err("Torrent overwrite is not supported.");
        }
        let settings = &request.options.settings;
        if !(1..=10000).contains(&settings.peer_limit)
            || [
                settings.download_bytes_per_second,
                settings.upload_bytes_per_second,
            ]
            .into_iter()
            .flatten()
            .any(|rate| !(1..=1_073_741_824).contains(&rate))
        {
            return Err("Invalid torrent limits.");
        }
        if settings.dht && request.source.is_private() {
            return Err("Private torrents cannot use DHT.");
        }
        let native = ulos_core::torrent::native_worker_available();
        if request.source.protocol() != ulos_core::torrent::TorrentProtocol::V1
            && !settings.allow_upload
        {
            return Err("V2 and hybrid torrents require explicit peer-upload consent.");
        }
        if (request.source.protocol() != ulos_core::torrent::TorrentProtocol::V1
            || settings.allow_upload)
            && !native
        {
            return Err("The native torrent worker is unavailable.");
        }
        if !settings.allow_upload
            && (settings.dht
                || settings.download_bytes_per_second.is_some()
                || settings.upload_bytes_per_second.is_some()
                || settings.peer_limit != 100)
        {
            return Err("Torrent limits and DHT require peer-upload consent.");
        }
        let id = self
            .queue
            .add_torrent(request.source.clone(), request.destination.clone())?;
        self.queue.item_mut(id).unwrap().options.collision = request.options.collision;
        self.queue.item_mut(id).unwrap().torrent_settings = settings.clone();
        self.cancel_all_shutdowns(cx);
        self.notice = None;
        if request.start_now && self.active.is_none() {
            self.start_item(id, cx);
        } else if request.start_now {
            self.queue_running = true;
        } else {
            self.advance(cx);
        }
        self.sync_table(cx);
        self.save_history();
        cx.notify();
        Ok(id)
    }
}
