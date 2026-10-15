use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};

use gpui_kit::{
    component::{
        ActiveTheme, Disableable, WindowExt,
        input::{InputEvent, InputState},
        table::{ColumnSort, TableEvent, TableState},
    },
    prelude::FluentBuilder as _,
    *,
};
use ulos_core::{
    tasks::{HttpDownloadTask, TransferState},
    transfer::{CollisionPolicy, MAX_CONNECTIONS, TransferOptions, TransferTelemetry},
};

use crate::{
    add_download::AddDownload,
    clipboard::ClipboardDetector,
    completion::{self, CompletionAction},
    download_details::{DetailsSnapshot, DownloadDetails},
    download_table::{DownloadRow, DownloadTable},
    formatting,
    native_windows::{ChildWindow, NativeAddWindow, StatusWindow, add_options, status_options},
    platform_paths::PlatformPaths,
    queue::{DownloadId, Filter, SessionQueue, SortColumn},
    scheduler::{DEFAULT_QUEUE, Scheduler},
    scheduler_window::SchedulerWindow,
    storage::{HistoryRecord, HistoryWriter},
};

actions!(
    download_manager,
    [AddUrl, StartQueue, StopAll, ToggleSidebar]
);
#[cfg(any(target_os = "macos", test))]
pub(crate) struct MenuStateChanged;
#[cfg(any(target_os = "macos", test))]
impl EventEmitter<MenuStateChanged> for DownloadManager {}

pub struct DownloadManager {
    queue: SessionQueue,
    search: Entity<InputState>,
    queue_running: bool,
    active: Option<DownloadId>,
    notice: Option<String>,
    focus: FocusHandle,
    runtime: tokio::runtime::Handle,
    transfer: Option<HttpDownloadTask>,
    pub(crate) add_windows: BTreeMap<WindowId, ChildWindow<AddDownload>>,
    pub(crate) detail_windows: BTreeMap<DownloadId, ChildWindow<DownloadDetails>>,
    pub(crate) table: Entity<TableState<DownloadTable>>,
    _window_closed: Subscription,
    last_rate: Option<f64>,
    _subscriptions: Vec<Subscription>,
    _updates: Option<Task<()>>,
    _shutdown_countdown: Option<Task<()>>,
    shutdown_id: Option<DownloadId>,
    shutdown_remaining: Option<u64>,
    completion_error: Option<String>,
    clipboard_enabled: bool,
    clipboard: ClipboardDetector,
    _clipboard_task: Option<Task<()>>,
    options_open: bool,
    sidebar_collapsed: bool,
    main_window: AnyWindowHandle,
    paths: Option<PlatformPaths>,
    history: Option<HistoryWriter>,
    history_error: Option<String>,
    _history_updates: Option<Task<()>>,
    removal_open: bool,
    removal_busy: bool,
    pending_removal: Option<Vec<DownloadId>>,
    _file_removal: Option<Task<()>>,
    scheduler: Scheduler,
    scheduler_window: Option<ChildWindow<SchedulerWindow>>,
    scheduler_error: Option<String>,
    _schedule_timer: Option<Task<()>>,
    #[cfg(any(target_os = "macos", test))]
    pub(crate) native_menu_subscriptions: Vec<Subscription>,
    #[cfg(test)]
    render_count: std::rc::Rc<std::cell::Cell<usize>>,
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
                view.sync_table(cx);
                cx.notify();
            }
        });
        let owner = cx.weak_entity();
        let table = cx.new(|cx| {
            TableState::new(DownloadTable::new(owner.clone()), window, cx)
                .row_selectable(true)
                .col_movable(false)
        });
        let table_subscription =
            cx.subscribe_in(&table, window, |view, table, event, window, cx| {
                let TableEvent::DoubleClickedRow(row) = event else {
                    return;
                };
                let Some(id) = table.read(cx).delegate().rows.get(*row).map(|row| row.id) else {
                    return;
                };
                view.open_details(id, window, cx);
            });
        let owner = cx.weak_entity();
        let window_closed = cx.on_window_closed(move |cx, closed| {
            let owner = owner.clone();
            cx.defer(move |cx| {
                let _ = owner.update(cx, |view, cx| {
                    if cx.windows().is_empty() {
                        view.cancel_all_shutdowns(cx);
                    }
                    view.add_windows.remove(&closed);
                    if view
                        .scheduler_window
                        .as_ref()
                        .is_some_and(|child| child.handle.window_id() == closed)
                    {
                        view.scheduler_window = None;
                    }
                    let shutdown_ids: Vec<_> = view
                        .detail_windows
                        .iter()
                        .filter(|(_, child)| child.handle.window_id() == closed)
                        .map(|(id, _)| *id)
                        .collect();
                    view.detail_windows
                        .retain(|_, child| child.handle.window_id() != closed);
                    for id in shutdown_ids {
                        if view.shutdown_id == Some(id)
                            || view
                                .queue
                                .item(id)
                                .is_some_and(|item| item.completion.is_armed())
                        {
                            view.cancel_shutdown(id, cx);
                        }
                    }
                });
            });
        });
        let clipboard_task = cx.spawn_in(window, async move |view, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                let enabled = view.update_in(cx, |view, window, _| {
                    view.clipboard_enabled
                        && window.is_window_active()
                        && view.add_windows.is_empty()
                        && view.detail_windows.is_empty()
                        && !view.options_open
                        && !view.removal_open
                        && !view.removal_busy
                        && view.scheduler_window.is_none()
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
        let schedule_timer = cx.spawn(async move |view, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                if view
                    .update(cx, |view, cx| view.scheduler_tick(chrono::Local::now(), cx))
                    .is_err()
                {
                    break;
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
            add_windows: BTreeMap::new(),
            detail_windows: BTreeMap::new(),
            table,
            _window_closed: window_closed,
            last_rate: None,
            _subscriptions: vec![
                subscription,
                table_subscription,
                crate::palette::observe_window(window, cx),
            ],
            _updates: None,
            _shutdown_countdown: None,
            shutdown_id: None,
            shutdown_remaining: None,
            completion_error: None,
            clipboard_enabled: false,
            clipboard: ClipboardDetector::default(),
            _clipboard_task: Some(clipboard_task),
            options_open: false,
            sidebar_collapsed: false,
            main_window: window.window_handle(),
            paths: None,
            history: None,
            history_error: None,
            _history_updates: None,
            removal_open: false,
            removal_busy: false,
            pending_removal: None,
            _file_removal: None,
            scheduler: Scheduler::default(),
            scheduler_window: None,
            scheduler_error: None,
            _schedule_timer: Some(schedule_timer),
            #[cfg(any(target_os = "macos", test))]
            native_menu_subscriptions: Vec::new(),
            #[cfg(test)]
            render_count: Default::default(),
        }
    }

    fn set_filter(&mut self, filter: Filter, cx: &mut Context<Self>) {
        self.queue.filter = filter;
        self.sync_table(cx);
        cx.notify();
    }

    fn toggle_sidebar(&mut self, cx: &mut Context<Self>) {
        self.sidebar_collapsed = !self.sidebar_collapsed;
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
        self.add_windows
            .retain(|_, child| child.upgrade().is_some());
        if self.add_windows.len() >= 16 {
            window.push_notification(
                "Close an Add window before opening another (maximum 16).",
                cx,
            );
            return;
        }
        let owner = cx.entity();
        let directory = self
            .paths
            .as_ref()
            .and_then(|paths| paths.downloads.clone());
        let runtime = self.runtime.clone();
        match gpui_kit::open_window(add_options(cx), cx, move |window, cx| {
            cx.new(|cx| NativeAddWindow::new(owner, source, directory, runtime, window, cx))
        }) {
            Ok((handle, wrapper)) => {
                let content = wrapper.read(cx).content.downgrade();
                self.add_windows
                    .insert(handle.window_id(), ChildWindow { handle, content });
            }
            Err(_) => window.push_notification("Could not open an Add window.", cx),
        }
        // Opening an independent window changes neither queue nor manager presentation.
        #[cfg(any(target_os = "macos", test))]
        cx.emit(MenuStateChanged);
    }

    #[cfg(test)]
    pub(crate) fn enqueue(
        &mut self,
        source: String,
        destination: std::path::PathBuf,
        start_now: bool,
        cx: &mut Context<Self>,
    ) -> Result<DownloadId, &'static str> {
        self.enqueue_with_options(
            source,
            destination,
            start_now,
            TransferOptions {
                connections: 4,
                ..TransferOptions::default()
            },
            cx,
        )
    }

    pub(crate) fn enqueue_with_options(
        &mut self,
        source: String,
        destination: std::path::PathBuf,
        start_now: bool,
        options: TransferOptions,
        cx: &mut Context<Self>,
    ) -> Result<DownloadId, &'static str> {
        if self.removal_busy {
            return Err("Wait for file deletion to finish.");
        }
        if !(1..=MAX_CONNECTIONS).contains(&options.connections)
            || options.bytes_per_second == Some(0)
        {
            return Err("Invalid transfer settings.");
        }
        let id = self.queue.add(source, destination)?;
        self.cancel_all_shutdowns(cx);
        self.queue.item_mut(id).unwrap().options = options;
        self.notice = None;
        if start_now && self.active.is_none() {
            self.start_item(id, cx);
        } else if start_now {
            self.queue_running = true;
            self.notice = Some("Added to queue; it will start after the active download.".into());
        } else {
            self.advance(cx);
        }
        self.sync_table(cx);
        self.save_history();
        cx.notify();
        Ok(id)
    }

    fn start_item(&mut self, id: DownloadId, cx: &mut Context<Self>) {
        if self.active.is_some() || self.removal_busy {
            return;
        }
        let Some(item) = self.queue.item_mut(id).filter(|item| item.is_queued()) else {
            return;
        };
        let source = item.source().to_owned();
        let destination = item.destination.clone();
        item.last_try = Some(std::time::SystemTime::now());
        item.apply_state(TransferState::Preparing, Instant::now());
        let options = item.options.clone();
        self.active = Some(id);
        self.save_history();
        self._updates = None;
        let task =
            HttpDownloadTask::start_with_options(&self.runtime, source, destination, options);
        let mut telemetry = task.telemetry();
        let mut receiver = task.subscribe();
        self.transfer = Some(task);
        self._updates = Some(cx.spawn(async move |view, cx| {
            let mut first = true;
            loop {
                let changed = receiver.has_changed();
                let metadata_changed = telemetry
                    .as_ref()
                    .is_some_and(|receiver| matches!(receiver.has_changed(), Ok(true)));
                if first || matches!(changed, Ok(true)) || metadata_changed {
                    first = false;
                    let state = receiver.borrow_and_update().clone();
                    let terminal = state.is_terminal();
                    if view
                        .update(cx, |view, cx| {
                            view.apply_observation(
                                id,
                                state,
                                telemetry
                                    .as_mut()
                                    .map(|receiver| receiver.borrow_and_update().clone()),
                                cx,
                            )
                        })
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
                    let _ = view.update(cx, |view, cx| {
                        view.apply_observation(
                            id,
                            terminal,
                            telemetry
                                .as_mut()
                                .map(|receiver| receiver.borrow_and_update().clone()),
                            cx,
                        )
                    });
                    break;
                }
                cx.background_executor()
                    .timer(Duration::from_millis(100))
                    .await;
                if view
                    .update(cx, |view, cx| {
                        let rate = view.measured_rate();
                        if view.last_rate != rate {
                            view.last_rate = rate;
                            view.refresh_details(cx);
                            view.sync_table(cx);
                            cx.notify();
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
        }));
        self.last_rate = self.measured_rate();
        self.refresh_details(cx);
        self.sync_table(cx);
        cx.notify();
    }

    fn apply_observation(
        &mut self,
        id: DownloadId,
        state: TransferState,
        telemetry: Option<TransferTelemetry>,
        cx: &mut Context<Self>,
    ) {
        if self.active != Some(id) {
            return;
        }
        if let Some(item) = self.queue.item_mut(id)
            && let Some(telemetry) = telemetry
        {
            item.apply_telemetry(telemetry);
        }
        self.apply_state(id, state, cx);
    }

    fn apply_state(&mut self, id: DownloadId, state: TransferState, cx: &mut Context<Self>) {
        if self.active != Some(id) {
            return;
        }
        let terminal = state.is_terminal();
        let success = matches!(state, TransferState::Completed(_));
        let changed_kind = self.queue.item(id).is_some_and(|item| {
            item.state.as_ref().map(std::mem::discriminant) != Some(std::mem::discriminant(&state))
        });
        if let Some(item) = self.queue.item_mut(id) {
            item.apply_state(state, Instant::now());
        }
        if changed_kind || terminal {
            self.save_history();
        }
        self.last_rate = self.measured_rate();
        self.refresh_details(cx);
        self.sync_table(cx);
        if terminal {
            self.active = None;
            self.transfer = None;
            if success {
                let countdown = self
                    .queue
                    .item_mut(id)
                    .is_some_and(|item| item.completion.on_success(Instant::now()));
                if countdown {
                    if self.queue.items.iter().all(|item| item.is_finished())
                        && !self.scheduler.has_enabled_schedule()
                    {
                        self.start_shutdown_countdown(id, cx);
                    } else {
                        self.cancel_all_shutdowns(cx);
                        self.completion_error = Some(
                            "Shutdown cancelled because other downloads are unfinished.".into(),
                        );
                    }
                }
            } else {
                self.cancel_all_shutdowns(cx);
            }
            self.refresh_details(cx);
            // Schedule the next receiver after this receiver task returns.
            let owner = cx.weak_entity();
            cx.defer(move |cx| {
                let _ = owner.update(cx, |view, cx| view.advance(cx));
            });
        }
        cx.notify();
    }

    fn advance(&mut self, cx: &mut Context<Self>) {
        self.scheduler
            .queue_mut(DEFAULT_QUEUE)
            .expect("Main queue exists")
            .running = self.queue_running;
        if self.active.is_some() || self.removal_busy {
            return;
        }
        let mut stopped = false;
        for queue in &mut self.scheduler.queues {
            if queue.running && self.queue.next_in_queue(queue.id).is_none() {
                queue.running = false;
                stopped = true;
            }
        }
        self.queue_running = self
            .scheduler
            .queue(DEFAULT_QUEUE)
            .is_some_and(|queue| queue.running);
        let next = self
            .scheduler
            .queues
            .iter()
            .filter(|queue| queue.running)
            .filter_map(|queue| self.queue.next_in_queue(queue.id))
            .min_by_key(|id| self.queue.item(*id).map(|item| (item.queue_order, item.id)));
        if let Some(id) = next {
            self.start_item(id, cx);
        }
        if stopped {
            self.refresh_scheduler(cx);
            cx.notify();
        }
    }

    fn start_queue(&mut self, cx: &mut Context<Self>) {
        if self.queue.next_in_queue(DEFAULT_QUEUE).is_none() {
            return;
        }
        self.queue_running = true;
        self.advance(cx);
        cx.notify();
    }

    fn stop_queue(&mut self, cx: &mut Context<Self>) {
        self.queue_running = false;
        self.scheduler
            .queue_mut(DEFAULT_QUEUE)
            .expect("Main queue exists")
            .running = false;
        self.notice = Some("Queue stopped. The active download will finish.".into());
        self.refresh_scheduler(cx);
        cx.notify();
    }

    fn stop_all(&mut self, cx: &mut Context<Self>) {
        self.queue_running = false;
        for queue in &mut self.scheduler.queues {
            queue.running = false;
        }
        self.cancel_active(cx);
        self.sync_table(cx);
        cx.notify();
    }

    fn cancel_active(&mut self, cx: &mut Context<Self>) {
        self.cancel_all_shutdowns(cx);
        if let Some(id) = self.active
            && let Some(item) = self.queue.item_mut(id)
            && item.request_cancel()
        {
            if let Some(task) = &mut self.transfer {
                task.cancel();
            }
            self.refresh_details(cx);
            self.sync_table(cx);
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

    fn remove_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.confirm_record_removal(self.queue.selected.iter().copied().collect(), window, cx);
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
            || !self.add_windows.is_empty()
            || !self.detail_windows.is_empty()
            || self.options_open
            || self.removal_open
            || self.removal_busy
            || self.scheduler_window.is_some()
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
            Err(_) if !item.has_source() => "Source not retained".into(),
            Err(_) => "Invalid source URL".into(),
        };
        Some(DetailsSnapshot {
            id,
            name: item.name(),
            source,
            destination: item.destination.clone(),
            folder_display: self
                .paths
                .as_ref()
                .map(|paths| {
                    paths.folder_label(item.destination.parent().unwrap_or(&item.destination))
                })
                .unwrap_or_default(),
            status: item.status().into(),
            progress: item.progress(),
            speed: item.speed_bytes_per_second(),
            eta: item.eta(),
            active: item.is_active(),
            cancelling: item.cancelling,
            telemetry: item.telemetry.clone(),
            options: item.options.clone(),
            can_retry: item.can_retry(),
            shutdown_armed: item.completion.is_armed(),
            shutdown_remaining: if self.shutdown_id == Some(id) {
                self.shutdown_remaining
            } else {
                None
            },
            completion_error: if self.scheduler.has_enabled_schedule() {
                Some("Completion shutdown is unavailable while a schedule is enabled. Clear schedules first.".into())
            } else {
                self.completion_error.clone()
            },
            paused: matches!(item.state, Some(TransferState::Paused(_))),
            error: match &item.state {
                Some(TransferState::Failed(error)) => {
                    Some(if error.starts_with("Network request failed:") {
                        "Network request failed. Check the connection and retry.".into()
                    } else {
                        error
                            .chars()
                            .filter(|character| !character.is_control())
                            .take(512)
                            .collect()
                    })
                }
                _ => None,
            },
        })
    }

    fn refresh_details(&self, cx: &mut Context<Self>) {
        for (&id, child) in &self.detail_windows {
            if let Some(details) = child.upgrade()
                && let Some(snapshot) = self.snapshot(id)
            {
                details.update(cx, |view, cx| view.update(snapshot, cx));
            }
        }
    }

    fn open_selected_details(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(id) = self.queue.selected.iter().next().copied() {
            self.open_details(id, window, cx);
        }
    }

    pub(crate) fn open_details(
        &mut self,
        id: DownloadId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(child) = self.detail_windows.get(&id)
            && child.upgrade().is_some()
            && child
                .handle
                .update(cx, |_, window, _| window.activate_window())
                .is_ok()
        {
            return;
        }
        self.detail_windows.remove(&id);
        let Some(snapshot) = self.snapshot(id) else {
            return;
        };
        let owner = cx.entity();
        match gpui_kit::open_window(status_options(cx), cx, move |window, cx| {
            cx.new(|cx| StatusWindow::new(owner, snapshot, window, cx))
        }) {
            Ok((handle, wrapper)) => {
                let content = wrapper.read(cx).content.downgrade();
                self.detail_windows
                    .insert(id, ChildWindow { handle, content });
            }
            Err(_) => window.push_notification("Could not open a download window.", cx),
        }
        #[cfg(any(target_os = "macos", test))]
        cx.emit(MenuStateChanged);
    }

    pub(crate) fn cancel_id(&mut self, id: DownloadId, cx: &mut Context<Self>) {
        if self.active == Some(id) {
            self.cancel_active(cx);
        }
    }

    pub(crate) fn toggle_pause(&mut self, id: DownloadId, cx: &mut Context<Self>) {
        if self.active != Some(id) {
            return;
        }
        let paused = self
            .queue
            .item(id)
            .is_some_and(|item| matches!(item.state, Some(TransferState::Paused(_))));
        let accepted = self
            .transfer
            .as_ref()
            .is_some_and(|task| if paused { task.resume() } else { task.pause() });
        self.notice = Some(
            if accepted {
                "Transfer control requested."
            } else {
                "Pause/resume is unavailable until verified range support is observed."
            }
            .into(),
        );
        cx.notify();
    }

    pub(crate) fn set_rate_limit(
        &mut self,
        id: DownloadId,
        rate: Option<u64>,
        cx: &mut Context<Self>,
    ) {
        if rate == Some(0) {
            self.notice = Some("Rate must be greater than zero.".into());
            cx.notify();
            return;
        }
        let Some(item) = self.queue.item(id) else {
            return;
        };
        if self.active == Some(id) {
            if !self
                .transfer
                .as_ref()
                .is_some_and(|task| task.set_rate_limit(rate))
            {
                self.notice = Some("The transfer cannot accept a rate change.".into());
                cx.notify();
                return;
            }
        } else if !item.is_queued() && !item.can_retry() {
            return;
        }
        if let Some(item) = self.queue.item_mut(id) {
            item.options.bytes_per_second = rate;
        }
        self.refresh_details(cx);
        cx.notify();
    }

    pub(crate) fn set_connections(
        &mut self,
        id: DownloadId,
        connections: usize,
        cx: &mut Context<Self>,
    ) {
        if !(1..=MAX_CONNECTIONS).contains(&connections) {
            self.notice = Some(format!(
                "Connections must be between 1 and {MAX_CONNECTIONS}."
            ));
            cx.notify();
            return;
        }
        if let Some(item) = self
            .queue
            .item_mut(id)
            .filter(|item| item.is_queued() || item.can_retry())
        {
            item.options.connections = connections;
            self.refresh_details(cx);
            cx.notify();
        }
    }

    pub(crate) fn retry_download(
        &mut self,
        id: DownloadId,
        collision: CollisionPolicy,
        cx: &mut Context<Self>,
    ) {
        if self.removal_busy || !self.queue.item(id).is_some_and(|item| item.can_retry()) {
            return;
        }
        self.cancel_all_shutdowns(cx);
        if !self
            .queue
            .item_mut(id)
            .is_some_and(|item| item.reset_for_retry(collision))
        {
            return;
        }
        if self.active.is_none() {
            self.start_item(id, cx);
        } else {
            let queue_id = self
                .queue
                .item(id)
                .map(|item| item.queue_id)
                .unwrap_or(DEFAULT_QUEUE);
            if let Some(queue) = self.scheduler.queue_mut(queue_id) {
                queue.running = true;
            }
            if queue_id == DEFAULT_QUEUE {
                self.queue_running = true;
            }
        }
        self.refresh_details(cx);
        self.sync_table(cx);
        self.save_history();
        cx.notify();
    }

    pub(crate) fn request_retry(
        &mut self,
        id: DownloadId,
        collision: CollisionPolicy,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if collision != CollisionPolicy::Overwrite {
            self.retry_download(id, collision, cx);
            return;
        }
        let owner = cx.weak_entity();
        window.open_dialog(cx, move |dialog, _, _| {
            let owner = owner.clone();
            dialog.title("Replace existing file?")
                .child(gpui_kit::component::label::Label::new("Retry will replace the destination file after a successful download. This cannot be undone."))
                .footer(gpui_kit::component::h_flex()
                    .child(gpui_kit::component::button::Button::new("cancel-retry").label("Cancel").on_click(|_, window, cx| window.close_dialog(cx)))
                    .child(gpui_kit::component::button::Button::new("confirm-retry").debug_selector(|| "confirm-retry".into()).label("Replace and retry").on_click(move |_, window, cx| {
                        let _ = owner.update(cx, |view, cx| view.retry_download(id, collision, cx));
                        window.close_dialog(cx);
                    })))
        });
    }

    pub(crate) fn request_shutdown(
        &mut self,
        id: DownloadId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !completion::shutdown_supported()
            || self.scheduler.has_enabled_schedule()
            || !self
                .queue
                .item(id)
                .is_some_and(|item| !item.state.as_ref().is_some_and(TransferState::is_terminal))
        {
            return;
        }
        let owner = cx.weak_entity();
        window.open_dialog(cx, move |dialog, _, _| {
            let owner = owner.clone();
            dialog.title("Turn off computer after completion?")
                .child(gpui_kit::component::label::Label::new("After this download finishes successfully, a cancellable 30-second countdown will turn off your computer. Save work in other applications. Other unfinished downloads or new work cancel shutdown. Operating system permissions may be required."))
                .footer(gpui_kit::component::h_flex()
                    .child(gpui_kit::component::button::Button::new("cancel-poweroff").label("Cancel").on_click(|_, window, cx| window.close_dialog(cx)))
                    .child(gpui_kit::component::button::Button::new("confirm-poweroff").debug_selector(|| "confirm-poweroff".into()).label("Turn off after completion").on_click(move |_, window, cx| {
                        let _ = owner.update(cx, |view, cx| view.arm_shutdown(id, cx));
                        window.close_dialog(cx);
                    })))
        });
    }

    fn arm_shutdown(&mut self, id: DownloadId, cx: &mut Context<Self>) {
        if !completion::shutdown_supported()
            || self.scheduler.has_enabled_schedule()
            || !self
                .queue
                .item(id)
                .is_some_and(|item| !item.state.as_ref().is_some_and(TransferState::is_terminal))
        {
            return;
        }
        self.cancel_all_shutdowns(cx);
        if let Some(item) = self.queue.item_mut(id) {
            item.completion.arm(CompletionAction::Shutdown);
        }
        self.completion_error = None;
        self.refresh_details(cx);
        cx.notify();
    }

    pub(crate) fn cancel_shutdown(&mut self, id: DownloadId, cx: &mut Context<Self>) {
        if let Some(item) = self.queue.item_mut(id) {
            item.completion.cancel();
        }
        if self.shutdown_id == Some(id) {
            self._shutdown_countdown = None;
            self.shutdown_id = None;
            self.shutdown_remaining = None;
        }
        self.completion_error = None;
        self.refresh_details(cx);
        cx.notify();
    }

    fn cancel_all_shutdowns(&mut self, cx: &mut Context<Self>) {
        for item in &mut self.queue.items {
            item.completion.on_failure_or_cancel();
        }
        self._shutdown_countdown = None;
        self.shutdown_id = None;
        self.shutdown_remaining = None;
        self.refresh_details(cx);
    }

    /// Recheck every safety condition immediately before consuming the single-use gate.
    fn completion_tick(&mut self, now: Instant, cx: &mut Context<Self>) -> bool {
        let Some(id) = self.shutdown_id else {
            return false;
        };
        if cx.windows().is_empty()
            || !self
                .detail_windows
                .get(&id)
                .is_some_and(|child| cx.windows().contains(&child.handle))
            || self.active.is_some()
            || self.scheduler.has_enabled_schedule()
            || !self.queue.items.iter().all(|item| item.is_finished())
        {
            self.cancel_all_shutdowns(cx);
            cx.notify();
            return false;
        }
        let Some(item) = self.queue.item_mut(id) else {
            self.cancel_all_shutdowns(cx);
            return false;
        };
        self.shutdown_remaining = item
            .completion
            .remaining(now)
            .map(|duration| duration.as_secs() + u64::from(duration.subsec_nanos() != 0));
        let due = item.completion.take_due(now);
        if due {
            self.shutdown_id = None;
            self.shutdown_remaining = None;
        }
        self.refresh_details(cx);
        cx.notify();
        due
    }

    fn start_shutdown_countdown(&mut self, id: DownloadId, cx: &mut Context<Self>) {
        self.shutdown_id = Some(id);
        self.shutdown_remaining = Some(30);
        let runtime = self.runtime.clone();
        self._shutdown_countdown = Some(cx.spawn(async move |view, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                let result = view.update(cx, |view, cx| {
                    let due = view.completion_tick(Instant::now(), cx);
                    (due, view.shutdown_id.is_some())
                });
                match result {
                    Ok((true, _)) => {
                        #[cfg(not(test))]
                        let result = runtime.spawn(completion::shutdown()).await.unwrap_or_else(|_| Err("Shutdown worker failed.".into()));
                        #[cfg(test)]
                        let result: Result<(), String> = {
                            let _ = (&runtime, completion::shutdown);
                            Err("OS shutdown is suppressed in test builds.".into())
                        };
                        let _ = view.update(cx, |view, cx| {
                            view.completion_error = result.err();
                            view.notice = Some(view.completion_error.clone().unwrap_or_else(|| "Shutdown requested; the operating system may still wait for open applications to close.".into()));
                            view.refresh_details(cx);
                            cx.notify();
                        });
                        break;
                    }
                    Ok((false, true)) => {},
                    _ => break,
                }
            }
        }));
    }

    fn measured_rate(&self) -> Option<f64> {
        self.queue
            .items
            .iter()
            .filter_map(|item| item.speed_bytes_per_second())
            .reduce(|a, b| a + b)
    }

    pub(crate) fn sync_table(&mut self, cx: &mut Context<Self>) {
        let rows = self
            .queue
            .visible()
            .into_iter()
            .map(|item| DownloadRow {
                id: item.id,
                name: item.name(),
                category: item.category,
                size: item.total_bytes,
                status: item.status().into(),
                rate: item.speed_bytes_per_second(),
                selected: self.queue.selected.contains(&item.id),
                progress: item.progress(),
                queue_name: self
                    .scheduler
                    .queue(item.queue_id)
                    .map(|queue| queue.name.clone())
                    .unwrap_or_default(),
                time_left: item.eta(),
                last_try: item.last_try,
                description: item.description.clone(),
            })
            .collect();
        let all_selected = self.queue.all_visible_selected();
        self.table.update(cx, |table, cx| {
            table.delegate_mut().rows = rows;
            table.delegate_mut().header_all_selected = all_selected;
            cx.notify();
        });
        self.refresh_scheduler(cx);
    }

    pub(crate) fn select_item(&mut self, id: DownloadId, selected: bool, cx: &mut Context<Self>) {
        self.queue.select(id, selected);
        self.sync_table(cx);
        cx.notify();
    }

    pub(crate) fn select_visible(&mut self, selected: bool, cx: &mut Context<Self>) {
        self.queue.select_visible(selected);
        self.sync_table(cx);
        cx.notify();
    }

    pub(crate) fn sort_table(
        &mut self,
        column: SortColumn,
        sort: ColumnSort,
        cx: &mut Context<Self>,
    ) {
        let column = if sort == ColumnSort::Default {
            SortColumn::Added
        } else {
            column
        };
        self.queue.set_sort(column, sort == ColumnSort::Descending);
        self.sync_table(cx);
        cx.notify();
    }

    fn clear_finished(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let keep: Vec<_> = self
            .queue
            .items
            .iter()
            .filter(|item| item.is_finished())
            .map(|item| item.id)
            .collect();
        self.confirm_record_removal(keep, window, cx);
    }

    fn close_orphaned_status_windows(&mut self, cx: &mut Context<Self>) {
        let removed: Vec<_> = self
            .detail_windows
            .keys()
            .copied()
            .filter(|id| self.queue.item(*id).is_none())
            .collect();
        for id in removed {
            if let Some(child) = self.detail_windows.remove(&id) {
                let _ = child
                    .handle
                    .update(cx, |_, window, _| window.remove_window());
            }
        }
    }

    fn open_options(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.options_open || self.scheduler_window.is_some() {
            return;
        }
        self.options_open = true;
        let owner = cx.weak_entity();
        window.open_dialog(cx, move |dialog, _, cx| {
            let enabled = owner.read_with(cx, |view, _| view.clipboard_enabled).unwrap_or(false);
            let change_owner = owner.clone();
            let close_owner = owner.clone();
            dialog.title("Options").w(px(480.)).child(gpui_kit::component::group_box::GroupBox::new()
                .child(gpui_kit::component::checkbox::Checkbox::new("clipboard-enabled").label("Detect copied HTTP / HTTPS links").checked(enabled)
                    .on_change(move |checked, _, cx| { let _ = change_owner.update(cx, |view, cx| { view.clipboard_enabled = *checked; cx.notify(); }); }))
                .child(gpui_kit::component::label::Label::new("Opt-in, this session only. Checked while the Ulos window is active. Links open a confirmation window; they never download automatically."))
                .child(gpui_kit::component::label::Label::new("Clipboard contents are not logged or saved. Wayland clipboard access depends on the compositor.")))
                .on_close(move |_, _, cx| { let _ = close_owner.update(cx, |view, cx| { view.options_open = false; cx.notify(); }); })
        });
        cx.notify();
    }
}

#[cfg(any(target_os = "macos", test))]
impl DownloadManager {
    pub(crate) fn menu_state(&self, cx: &App) -> crate::native_menu::MenuState {
        use crate::native_menu::MenuState;
        MenuState {
            can_add: self.add_windows.len() < 16,
            can_start: !self.queue_running && self.queue.next_in_queue(DEFAULT_QUEUE).is_some(),
            queue_running: self.queue_running,
            can_stop: self.active.is_some(),
            selected_active: self
                .queue
                .items
                .iter()
                .any(|item| self.queue.selected.contains(&item.id) && item.is_active()),
            removable: self
                .queue
                .items
                .iter()
                .any(|item| self.queue.selected.contains(&item.id) && !item.is_active()),
            finished: self.queue.items.iter().any(|item| item.is_finished()),
            selected: self
                .queue
                .selected
                .iter()
                .any(|id| self.queue.item(*id).is_some()),
            revealable: self
                .queue
                .items
                .iter()
                .any(|item| self.queue.selected.contains(&item.id) && item.is_finished()),
            can_options: !self.options_open
                && self.add_windows.is_empty()
                && self.detail_windows.is_empty()
                && self.scheduler_window.is_none()
                && cx.windows().contains(&self.main_window),
            dark: cx.theme().is_dark(),
            filter: self.queue.filter,
            sidebar_collapsed: self.sidebar_collapsed,
            theme_preference: crate::palette::preference(cx),
        }
    }

    pub(crate) fn run_menu_command(
        &mut self,
        command: crate::native_menu::Command,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use crate::native_menu::Command;
        if !command.enabled(&self.menu_state(cx)) {
            return;
        }
        match command {
            Command::Add => self.open_add(window, cx),
            Command::Start => self.start_queue(cx),
            Command::StopQueue => self.stop_queue(cx),
            Command::StopAll => self.stop_all(cx),
            Command::StopSelected => self.stop_selected(cx),
            Command::Remove => self.remove_selected(window, cx),
            Command::Clear => self.clear_finished(window, cx),
            Command::Options => self.open_options(window, cx),
            Command::Scheduler => self.open_scheduler(window, cx),
            Command::Details => self.open_selected_details(window, cx),
            Command::Reveal => self.reveal_selected(cx),
            Command::All => self.set_filter(Filter::All, cx),
            Command::Finished => self.set_filter(Filter::Finished, cx),
            Command::Unfinished => self.set_filter(Filter::Unfinished, cx),
            Command::Queued => self.set_filter(Filter::Queued, cx),
            Command::ToggleSidebar => self.toggle_sidebar(cx),
            Command::SystemTheme | Command::LightTheme | Command::DarkTheme => {
                use crate::palette::ThemePreference;
                crate::palette::choose(match command {
                    Command::SystemTheme => ThemePreference::System,
                    Command::DarkTheme => ThemePreference::Dark,
                    _ => ThemePreference::Light,
                }, cx);
            }
            Command::Help => window.open_dialog(cx, |dialog, _, _| {
                use gpui_kit::component::{group_box::GroupBox, label::Label};
                dialog.title("Ulos Help").child(GroupBox::new()
                    .child(Label::new("Add URL opens a new download window. Choose a destination and confirm to start, or queue for later."))
                    .child(Label::new("Queue order is FIFO. Stop Queue lets the active file finish; Stop All requests cancellation. Removing records never deletes downloaded files."))
                    .child(Label::new("Click a filename or choose Details to open its native status window. Queue history and clipboard preferences are session-only.")))
            }),
        }
    }
}

#[path = "manager_history.rs"]
mod history;
#[path = "manager_render.rs"]
mod rendering;
#[path = "manager_scheduler.rs"]
mod scheduling;

#[cfg(test)]
#[path = "manager_tests.rs"]
mod tests;
