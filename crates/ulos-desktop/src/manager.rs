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
use ulos_core::tasks::{HttpDownloadTask, TransferState};

use crate::{
    add_download::AddDownload,
    clipboard::ClipboardDetector,
    download_details::{DetailsSnapshot, DownloadDetails},
    download_table::{DownloadRow, DownloadTable},
    formatting,
    native_windows::{ChildWindow, NativeAddWindow, StatusWindow, child_options},
    queue::{DownloadId, Filter, SessionQueue, SortColumn},
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
    clipboard_enabled: bool,
    clipboard: ClipboardDetector,
    _clipboard_task: Option<Task<()>>,
    options_open: bool,
    sidebar_collapsed: bool,
    main_window: AnyWindowHandle,
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
                let (TableEvent::SelectRow(row) | TableEvent::DoubleClickedRow(row)) = event else {
                    return;
                };
                let Some(id) = table.read(cx).delegate().rows.get(*row).map(|row| row.id) else {
                    return;
                };
                if matches!(event, TableEvent::DoubleClickedRow(_)) {
                    view.open_details(id, window, cx);
                } else {
                    view.select_item(id, true, cx);
                }
            });
        let owner = cx.weak_entity();
        let window_closed = cx.on_window_closed(move |cx, closed| {
            let owner = owner.clone();
            cx.defer(move |cx| {
                let _ = owner.update(cx, |view, _| {
                    view.add_windows.remove(&closed);
                    view.detail_windows
                        .retain(|_, child| child.handle.window_id() != closed);
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
            clipboard_enabled: false,
            clipboard: ClipboardDetector::default(),
            _clipboard_task: Some(clipboard_task),
            options_open: false,
            sidebar_collapsed: false,
            main_window: window.window_handle(),
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
        match gpui_kit::open_window(
            child_options("Add a download", 560., 520., cx),
            cx,
            move |window, cx| cx.new(|cx| NativeAddWindow::new(owner, source, window, cx)),
        ) {
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

    pub(crate) fn enqueue(
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
        self.sync_table(cx);
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

    fn apply_state(&mut self, id: DownloadId, state: TransferState, cx: &mut Context<Self>) {
        if self.active != Some(id) {
            return;
        }
        let terminal = state.is_terminal();
        if let Some(item) = self.queue.item_mut(id) {
            item.apply_state(state, Instant::now());
        }
        self.last_rate = self.measured_rate();
        self.refresh_details(cx);
        self.sync_table(cx);
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

    fn remove_selected(&mut self, cx: &mut Context<Self>) {
        let removed = self.queue.remove_selected();
        self.close_orphaned_status_windows(cx);
        self.sync_table(cx);
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
            || !self.add_windows.is_empty()
            || !self.detail_windows.is_empty()
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
        let title = format!("Download — {}", snapshot.name);
        let owner = cx.entity();
        match gpui_kit::open_window(
            child_options(&title, 680., 640., cx),
            cx,
            move |window, cx| cx.new(|cx| StatusWindow::new(owner, snapshot, window, cx)),
        ) {
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
                destination: item.destination.clone(),
            })
            .collect();
        let all_selected = self.queue.all_visible_selected();
        self.table.update(cx, |table, cx| {
            table.delegate_mut().rows = rows;
            table.delegate_mut().header_all_selected = all_selected;
            cx.notify();
        });
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
        self.close_orphaned_status_windows(cx);
        self.sync_table(cx);
        self.notice = Some("Cleared finished records. Downloaded files were not deleted.".into());
        cx.notify();
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
        if self.options_open {
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
            can_start: !self.queue_running && self.queue.next_queued().is_some(),
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
            Command::Remove => self.remove_selected(cx),
            Command::Clear => self.clear_finished(cx),
            Command::Options => self.open_options(window, cx),
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

#[path = "manager_render.rs"]
mod rendering;

#[cfg(test)]
#[path = "manager_tests.rs"]
mod tests;
