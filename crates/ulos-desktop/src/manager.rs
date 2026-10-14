use std::time::{Duration, Instant};

use gpui_kit::{
    component::{
        ActiveTheme, Disableable, Sizable, WindowExt,
        input::{InputEvent, InputState},
    },
    prelude::FluentBuilder as _,
    *,
};
use ulos_core::tasks::{HttpDownloadTask, TransferState};

use crate::{
    add_download::{AddDownload, AddRequest, DismissAdd},
    clipboard::ClipboardDetector,
    download_details::{CancelDownload, CloseDetails, DetailsSnapshot, DownloadDetails},
    formatting,
    queue::{DownloadId, Filter, SessionQueue},
};

actions!(download_manager, [AddUrl, StartQueue, StopAll]);

pub struct DownloadManager {
    queue: SessionQueue,
    search: Entity<InputState>,
    queue_running: bool,
    active: Option<DownloadId>,
    notice: Option<String>,
    focus: FocusHandle,
    runtime: tokio::runtime::Handle,
    transfer: Option<HttpDownloadTask>,
    add_form: Option<Entity<AddDownload>>,
    _subscriptions: Vec<Subscription>,
    _add_subscription: Option<Subscription>,
    _dismiss_subscription: Option<Subscription>,
    _updates: Option<Task<()>>,
    details: Option<Entity<DownloadDetails>>,
    _details_subscription: Option<Subscription>,
    _details_close_subscription: Option<Subscription>,
    clipboard_enabled: bool,
    clipboard: ClipboardDetector,
    _clipboard_task: Option<Task<()>>,
    options_open: bool,
}

impl DownloadManager {
    pub fn new(
        runtime: tokio::runtime::Handle,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search downloads"));
        let subscription = cx.subscribe_in(&search, window, |view, input, event, _, cx| {
            if matches!(event, InputEvent::Change) {
                view.queue.search = input.read(cx).value().to_string();
                cx.notify();
            }
        });
        let clipboard_task = cx.spawn_in(window, async move |view, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                let enabled = view.update_in(cx, |view, window, _| {
                    view.clipboard_enabled
                        && window.is_window_active()
                        && view.add_form.is_none()
                        && view.details.is_none()
                        && !view.options_open
                });
                match enabled {
                    Ok(false) => continue,
                    Err(_) => break,
                    _ => {}
                }
                let read = match cx.update(|_, cx| cx.read_from_clipboard_async()) {
                    Ok(read) => read,
                    Err(_) => break,
                };
                if let Ok(Some(item)) = read.await {
                    let text = item.text();
                    let _ = view.update_in(cx, |view, window, cx| {
                        if window.is_window_active() {
                            view.offer_clipboard(text.as_deref(), window, cx);
                        }
                    });
                }
            }
        });
        Self {
            queue: SessionQueue::default(),
            search,
            queue_running: false,
            active: None,
            notice: None,
            focus: cx.focus_handle(),
            runtime,
            transfer: None,
            add_form: None,
            _subscriptions: vec![subscription],
            _add_subscription: None,
            _dismiss_subscription: None,
            _updates: None,
            details: None,
            _details_subscription: None,
            _details_close_subscription: None,
            clipboard_enabled: false,
            clipboard: ClipboardDetector::default(),
            _clipboard_task: Some(clipboard_task),
            options_open: false,
        }
    }

    fn set_filter(&mut self, filter: Filter, cx: &mut Context<Self>) {
        self.queue.filter = filter;
        cx.notify();
    }

    fn open_add(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_add_source(None, window, cx);
    }

    fn open_add_source(
        &mut self,
        source: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.add_form.is_some() {
            return;
        }
        let form = cx.new(|cx| AddDownload::new(window, cx));
        if let Some(source) = source {
            form.update(cx, |form, cx| form.prefill(source, window, cx));
        }
        self._add_subscription = Some(cx.subscribe_in(
            &form,
            window,
            |view, _, request: &AddRequest, window, cx| match view.enqueue(
                request.source.clone(),
                request.destination.clone(),
                request.start_now,
                cx,
            ) {
                Ok(id) => {
                    view.close_add(window, cx);
                    if request.start_now {
                        view.open_details(id, window, cx);
                    }
                }
                Err(error) => window.push_notification(error, cx),
            },
        ));
        self._dismiss_subscription = Some(cx.subscribe_in(
            &form,
            window,
            |view, _, _: &DismissAdd, window, cx| view.close_add(window, cx),
        ));
        let owner = cx.weak_entity();
        let content = form.clone();
        window.open_dialog(cx, move |dialog, _, cx| {
            let owner = owner.clone();
            let choosing = content.read(cx).choosing_path;
            dialog
                .title("Add a download")
                .w(px(560.))
                .child(content.clone())
                .close_button(!choosing)
                .keyboard(!choosing)
                .overlay_closable(!choosing)
                .on_close(move |_, _, cx| {
                    let _ = owner.update(cx, |view, cx| {
                        view.add_form = None;
                        view._add_subscription = None;
                        view._dismiss_subscription = None;
                        cx.notify();
                    });
                })
        });
        self.add_form = Some(form);
        cx.notify();
    }

    fn close_add(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self
            .add_form
            .as_ref()
            .is_some_and(|form| form.read(cx).choosing_path)
        {
            return;
        }
        window.close_dialog(cx);
        self.add_form = None;
        self._add_subscription = None;
        self._dismiss_subscription = None;
        cx.notify();
    }

    fn enqueue(
        &mut self,
        source: String,
        destination: std::path::PathBuf,
        start_now: bool,
        cx: &mut Context<Self>,
    ) -> Result<DownloadId, &'static str> {
        let id = self.queue.add(source, destination)?;
        self.notice = None;
        if start_now && self.active.is_none() {
            self.start_item(id, cx);
        } else if start_now {
            self.queue_running = true;
            self.notice = Some("Added to queue; it will start after the active download.".into());
        } else {
            self.advance(cx);
        }
        cx.notify();
        Ok(id)
    }

    fn start_item(&mut self, id: DownloadId, cx: &mut Context<Self>) {
        if self.active.is_some() {
            return;
        }
        let Some(item) = self.queue.item_mut(id).filter(|item| item.is_queued()) else {
            return;
        };
        let source = item.source().to_owned();
        let destination = item.destination.clone();
        item.apply_state(TransferState::Preparing, Instant::now());
        self.active = Some(id);
        self._updates = None;
        let task = HttpDownloadTask::start(&self.runtime, source, destination);
        let mut receiver = task.subscribe();
        self.transfer = Some(task);
        self._updates = Some(cx.spawn(async move |view, cx| {
            let mut first = true;
            loop {
                let changed = receiver.has_changed();
                if first || matches!(changed, Ok(true)) {
                    first = false;
                    let state = receiver.borrow_and_update().clone();
                    let terminal = state.is_terminal();
                    if view
                        .update(cx, |view, cx| view.apply_state(id, state, cx))
                        .is_err()
                    {
                        break;
                    }
                    if terminal {
                        break;
                    }
                } else if changed.is_err() {
                    // A closed watch channel can still retain an unseen final value.
                    let state = receiver.borrow_and_update().clone();
                    let terminal = if state.is_terminal() {
                        state
                    } else {
                        TransferState::Failed("Transfer worker stopped unexpectedly".into())
                    };
                    let _ = view.update(cx, |view, cx| view.apply_state(id, terminal, cx));
                    break;
                }
                cx.background_executor()
                    .timer(Duration::from_millis(100))
                    .await;
                if view
                    .update(cx, |view, cx| {
                        view.refresh_details(cx);
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
            }
        }));
        cx.notify();
    }

    fn apply_state(&mut self, id: DownloadId, state: TransferState, cx: &mut Context<Self>) {
        if self.active != Some(id) {
            return;
        }
        let terminal = state.is_terminal();
        if let Some(item) = self.queue.item_mut(id) {
            item.apply_state(state, Instant::now());
        }
        self.refresh_details(cx);
        if terminal {
            self.active = None;
            self.transfer = None;
            // Schedule the next receiver after this receiver task returns.
            let owner = cx.weak_entity();
            cx.defer(move |cx| {
                let _ = owner.update(cx, |view, cx| view.advance(cx));
            });
        }
        cx.notify();
    }

    fn advance(&mut self, cx: &mut Context<Self>) {
        if self.queue_running && self.active.is_none() {
            if let Some(id) = self.queue.next_queued() {
                self.start_item(id, cx);
            } else {
                self.queue_running = false;
                cx.notify();
            }
        }
    }

    fn start_queue(&mut self, cx: &mut Context<Self>) {
        if self.queue.next_queued().is_none() {
            return;
        }
        self.queue_running = true;
        self.advance(cx);
        cx.notify();
    }

    fn stop_queue(&mut self, cx: &mut Context<Self>) {
        self.queue_running = false;
        self.notice = Some("Queue stopped. The active download will finish.".into());
        cx.notify();
    }

    fn stop_all(&mut self, cx: &mut Context<Self>) {
        self.queue_running = false;
        self.cancel_active(cx);
    }

    fn cancel_active(&mut self, cx: &mut Context<Self>) {
        if let Some(id) = self.active
            && let Some(item) = self.queue.item_mut(id)
            && item.request_cancel()
        {
            if let Some(task) = &mut self.transfer {
                task.cancel();
            }
            self.notice = Some(
                "Cancellation requested. A file may exist if finalization had already started."
                    .into(),
            );
            cx.notify();
        }
    }

    fn stop_selected(&mut self, cx: &mut Context<Self>) {
        if self
            .active
            .is_some_and(|id| self.queue.selected.contains(&id))
        {
            self.cancel_active(cx);
        }
    }

    fn remove_selected(&mut self, cx: &mut Context<Self>) {
        let removed = self.queue.remove_selected();
        self.notice = Some(format!(
            "Removed {removed} session record(s). Files were not deleted."
        ));
        cx.notify();
    }

    fn reveal_selected(&self, cx: &mut Context<Self>) {
        for item in &self.queue.items {
            if self.queue.selected.contains(&item.id) && item.is_finished() {
                cx.reveal_path(&item.destination);
            }
        }
    }

    fn offer_clipboard(&mut self, text: Option<&str>, window: &mut Window, cx: &mut Context<Self>) {
        if !self.clipboard_enabled
            || self.add_form.is_some()
            || self.details.is_some()
            || self.options_open
        {
            return;
        }
        if let Some(source) = self.clipboard.candidate(text) {
            self.open_add_source(Some(source), window, cx);
        }
    }

    fn snapshot(&self, id: DownloadId) -> Option<DetailsSnapshot> {
        let item = self.queue.item(id)?;
        let source = match url::Url::parse(item.source()) {
            Ok(mut url) => {
                url.set_query(None);
                url.set_fragment(None);
                let _ = url.set_username("");
                let _ = url.set_password(None);
                url.to_string()
            }
            Err(_) => "Invalid source URL".into(),
        };
        Some(DetailsSnapshot {
            id,
            name: item.name(),
            source,
            destination: item.destination.clone(),
            status: item.status().into(),
            progress: item.progress(),
            speed: item.speed_bytes_per_second(),
            eta: item.eta(),
            active: item.is_active(),
            cancelling: item.cancelling,
            error: match &item.state {
                Some(TransferState::Failed(error)) => Some(error.clone()),
                _ => None,
            },
        })
    }

    fn refresh_details(&self, cx: &mut Context<Self>) {
        if let Some(details) = &self.details
            && let Some(snapshot) = self.snapshot(details.read(cx).snapshot.id)
        {
            details.update(cx, |view, cx| view.update(snapshot, cx));
        }
    }

    fn open_selected_details(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(id) = self.queue.selected.iter().next().copied() {
            self.open_details(id, window, cx);
        }
    }

    fn open_details(&mut self, id: DownloadId, window: &mut Window, cx: &mut Context<Self>) {
        if self.details.is_some() || self.add_form.is_some() || self.options_open {
            return;
        }
        let Some(snapshot) = self.snapshot(id) else {
            return;
        };
        let title = format!("Download — {}", snapshot.name);
        let details = cx.new(|_| DownloadDetails::new(snapshot));
        self._details_subscription = Some(cx.subscribe(
            &details,
            |view, _, event: &CancelDownload, cx| {
                if view.active == Some(event.0) {
                    view.cancel_active(cx);
                }
                view.refresh_details(cx);
            },
        ));
        self._details_close_subscription =
            Some(
                cx.subscribe_in(&details, window, |view, _, _: &CloseDetails, window, cx| {
                    window.close_dialog(cx);
                    view.details = None;
                    view._details_subscription = None;
                    view._details_close_subscription = None;
                    cx.notify();
                }),
            );
        let owner = cx.weak_entity();
        let content = details.clone();
        window.open_dialog(cx, move |dialog, _, _| {
            let owner = owner.clone();
            dialog
                .title(title.clone())
                .w(px(620.))
                .child(content.clone())
                .on_close(move |_, _, cx| {
                    let _ = owner.update(cx, |view, cx| {
                        view.details = None;
                        view._details_subscription = None;
                        view._details_close_subscription = None;
                        cx.notify();
                    });
                })
        });
        self.details = Some(details);
        cx.notify();
    }

    fn clear_finished(&mut self, cx: &mut Context<Self>) {
        let keep: Vec<_> = self
            .queue
            .items
            .iter()
            .filter(|item| item.is_finished())
            .map(|item| item.id)
            .collect();
        self.queue.items.retain(|item| !keep.contains(&item.id));
        self.queue.selected.retain(|id| !keep.contains(id));
        self.notice = Some("Cleared finished records. Downloaded files were not deleted.".into());
        cx.notify();
    }

    fn open_options(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.add_form.is_some() || self.details.is_some() || self.options_open {
            return;
        }
        self.options_open = true;
        let owner = cx.weak_entity();
        window.open_dialog(cx, move |dialog, _, cx| {
            let enabled = owner.read_with(cx, |view, _| view.clipboard_enabled).unwrap_or(false);
            let change_owner = owner.clone();
            let close_owner = owner.clone();
            dialog.title("Options").w(px(480.)).child(div().flex().flex_col().gap_3()
                .child(gpui_kit::component::checkbox::Checkbox::new("clipboard-enabled").label("Detect copied HTTP / HTTPS links").checked(enabled)
                    .on_change(move |checked, _, cx| { let _ = change_owner.update(cx, |view, cx| { view.clipboard_enabled = *checked; cx.notify(); }); }))
                .child("Opt-in, this session only. Checked while the Ulos window is active. Links open a confirmation dialog; they never download automatically.")
                .child("Clipboard contents are not logged or saved. Wayland clipboard access depends on the compositor."))
                .on_close(move |_, _, cx| { let _ = close_owner.update(cx, |view, cx| { view.options_open = false; cx.notify(); }); })
        });
        cx.notify();
    }
}

#[path = "manager_render.rs"]
mod rendering;

#[cfg(test)]
#[path = "manager_tests.rs"]
mod tests;
