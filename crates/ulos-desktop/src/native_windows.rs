//! Independent native roots. Roots own the manager; its registries own only weak content.
use crate::{
    add_download::{AddDownload, AddRequest, DismissAdd},
    download_details::{CancelDownload, CloseDetails, DetailsSnapshot, DownloadDetails},
    manager::DownloadManager,
};
use gpui_kit::{
    component::{ActiveTheme, WindowExt, v_flex},
    *,
};

pub(crate) struct ChildWindow<T: 'static> {
    pub handle: AnyWindowHandle,
    pub content: WeakEntity<T>,
}
impl<T: 'static> ChildWindow<T> {
    pub fn upgrade(&self) -> Option<Entity<T>> {
        self.content.upgrade()
    }
}

pub(crate) fn child_options(title: &str, width: f32, height: f32, cx: &App) -> WindowOptions {
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
            None,
            size(px(width), px(height)),
            cx,
        ))),
        window_min_size: Some(size(px(480.), px(400.))),
        titlebar: Some(TitlebarOptions {
            title: Some(title.to_owned().into()),
            ..Default::default()
        }),
        ..Default::default()
    }
}

pub(crate) struct NativeAddWindow {
    pub content: Entity<AddDownload>,
    _owner: Entity<DownloadManager>,
    _subscriptions: Vec<Subscription>,
}
impl NativeAddWindow {
    pub fn new(
        owner: Entity<DownloadManager>,
        source: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let content = cx.new(|cx| AddDownload::new(window, cx));
        if let Some(source) = source {
            content.update(cx, |form, cx| form.prefill(source, window, cx));
        }
        let weak_content = content.downgrade();
        window.on_window_should_close(cx, move |_, cx| {
            weak_content
                .upgrade()
                .is_none_or(|form| !form.read(cx).choosing_path)
        });
        // Subscribe on the wrapper, not the manager: these handlers must execute
        // in the child window even after the manager's original window closes.
        let submit = cx.subscribe_in(
            &content,
            window,
            |view, _, request: &AddRequest, window, cx| {
                if view.content.read(cx).choosing_path {
                    return;
                }
                let result = view._owner.update(cx, |owner, cx| {
                    owner.enqueue(
                        request.source.clone(),
                        request.destination.clone(),
                        request.start_now,
                        cx,
                    )
                });
                match result {
                    Ok(id) => {
                        window.remove_window();
                        if request.start_now {
                            view._owner
                                .update(cx, |owner, cx| owner.open_details(id, window, cx));
                        }
                    }
                    Err(error) => window.push_notification(error, cx),
                }
            },
        );
        let dismiss = cx.subscribe_in(&content, window, |view, _, _: &DismissAdd, window, cx| {
            if !view.content.read(cx).choosing_path {
                window.remove_window();
            }
        });
        Self {
            content,
            _owner: owner,
            _subscriptions: vec![submit, dismiss, crate::palette::observe_window(window, cx)],
        }
    }
}
impl Render for NativeAddWindow {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .size_full()
            .bg(cx.theme().background)
            .child(self.content.clone())
    }
}

pub(crate) struct StatusWindow {
    pub content: Entity<DownloadDetails>,
    _owner: Entity<DownloadManager>,
    _subscriptions: Vec<Subscription>,
}
impl StatusWindow {
    pub fn new(
        owner: Entity<DownloadManager>,
        snapshot: DetailsSnapshot,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let content = cx.new(|_| DownloadDetails::new(snapshot));
        let cancel = cx.subscribe_in(
            &content,
            window,
            |view, _, event: &CancelDownload, _, cx| {
                view._owner
                    .update(cx, |owner, cx| owner.cancel_id(event.0, cx));
            },
        );
        let close = cx.subscribe_in(&content, window, |_, _, _: &CloseDetails, window, _| {
            // Closing passive telemetry never cancels its manager-owned transfer.
            window.remove_window();
        });
        Self {
            content,
            _owner: owner,
            _subscriptions: vec![cancel, close, crate::palette::observe_window(window, cx)],
        }
    }
}
impl Render for StatusWindow {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .size_full()
            .bg(cx.theme().background)
            .child(self.content.clone())
    }
}
