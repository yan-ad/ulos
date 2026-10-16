//! Independent native roots. Roots own the manager; its registries own only weak content.
use crate::{
    add_download::{AddDownload, AddRequest, DismissAdd},
    add_torrent::{AddTorrent, DismissTorrent, TorrentRequest},
    download_details::{
        CancelDownload, CancelShutdown, CloseDetails, DetailsSnapshot, DownloadDetails,
        RequestShutdown, RetryDownload, SetConnections, SetRateLimit, SetTorrentLimits,
        SetTorrentPeers, TogglePause,
    },
    manager::DownloadManager,
    scheduler_window::{SchedulerRequest, SchedulerSnapshot, SchedulerWindow},
};
use gpui_kit::{
    component::{ActiveTheme, WindowExt, v_flex},
    *,
};

pub(crate) struct ChildWindow<T: 'static> {
    pub handle: AnyWindowHandle,
    pub content: WeakEntity<T>,
}

pub(crate) struct NativeTorrentWindow {
    pub content: Entity<AddTorrent>,
    _owner: Entity<DownloadManager>,
    _subscriptions: Vec<Subscription>,
}
impl NativeTorrentWindow {
    pub fn new(
        owner: Entity<DownloadManager>,
        path: Option<std::path::PathBuf>,
        directory: Option<std::path::PathBuf>,
        runtime: tokio::runtime::Handle,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let content = cx.new(|cx| AddTorrent::new(path, directory, runtime, window, cx));
        let weak = content.downgrade();
        window.on_window_should_close(cx, move |_, cx| {
            weak.upgrade()
                .is_none_or(|view| !view.read(cx).choosing_path)
        });
        let submit = cx.subscribe_in(
            &content,
            window,
            |view, _, request: &TorrentRequest, window, cx| {
                if !view.content.read(cx).matches_request(request, cx) {
                    return;
                }
                let result = view
                    ._owner
                    .update(cx, |owner, cx| owner.enqueue_torrent(request, cx));
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
        let dismiss = cx.subscribe_in(
            &content,
            window,
            |view, _, _: &DismissTorrent, window, cx| {
                if !view.content.read(cx).choosing_path {
                    window.remove_window();
                }
            },
        );
        Self {
            content,
            _owner: owner,
            _subscriptions: vec![submit, dismiss, crate::palette::observe_window(window, cx)],
        }
    }
}
impl Render for NativeTorrentWindow {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .size_full()
            .bg(cx.theme().background)
            .child(self.content.clone())
    }
}

pub(crate) struct NativeSchedulerWindow {
    pub content: Entity<SchedulerWindow>,
    _owner: Entity<DownloadManager>,
    _subscriptions: Vec<Subscription>,
}

impl NativeSchedulerWindow {
    pub fn new(
        owner: Entity<DownloadManager>,
        snapshot: SchedulerSnapshot,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let content = cx.new(|cx| SchedulerWindow::new(snapshot, window, cx));
        let subscription = cx.subscribe_in(
            &content,
            window,
            |view, _, request: &SchedulerRequest, window, cx| {
                if matches!(request, SchedulerRequest::Close) {
                    window.remove_window();
                    return;
                }
                let draft = view.content.read(cx).snapshot.clone();
                view._owner
                    .update(cx, |owner, cx| owner.scheduler_request(request, &draft, cx));
            },
        );
        Self {
            content,
            _owner: owner,
            _subscriptions: vec![subscription, crate::palette::observe_window(window, cx)],
        }
    }
}

impl Render for NativeSchedulerWindow {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .size_full()
            .bg(cx.theme().background)
            .child(self.content.clone())
    }
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

pub(crate) fn add_options(cx: &App) -> WindowOptions {
    let mut options = child_options("", 560., 540., cx);
    options.is_minimizable = false;
    options.is_resizable = false;
    options
        .titlebar
        .as_mut()
        .expect("Add titlebar configured")
        .title = None;
    options
}

/// Status-only full-content chrome.
pub(crate) fn status_options(cx: &App) -> WindowOptions {
    let mut options = child_options("Download", 680., 640., cx);
    options.window_min_size = Some(size(px(580.), px(440.)));
    #[cfg(target_os = "macos")]
    {
        let titlebar = options
            .titlebar
            .as_mut()
            .expect("Status titlebar configured");
        titlebar.appears_transparent = true;
        titlebar.traffic_light_position = Some(point(px(12.), px(17.)));
        options.app_owns_titlebar_drag =
            gpui_kit::component::TitleBar::window_options().app_owns_titlebar_drag;
    }
    options
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
        default_directory: Option<std::path::PathBuf>,
        runtime: tokio::runtime::Handle,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let content = cx.new(|cx| AddDownload::new(default_directory, runtime, window, cx));
        if let Some(source) = source {
            content.update(cx, |form, cx| form.prefill(source, window, cx));
        }
        let weak_content = content.downgrade();
        window.on_window_should_close(cx, move |_, cx| {
            weak_content.upgrade().is_none_or(|form| {
                form.update(cx, |form, _| {
                    if form.choosing_path {
                        return false;
                    }
                    form.invalidate_preview();
                    true
                })
            })
        });
        // Subscribe on the wrapper, not the manager: these handlers must execute
        // in the child window even after the manager's original window closes.
        let submit = cx.subscribe_in(
            &content,
            window,
            |view, _, request: &AddRequest, window, cx| {
                if !view.content.read(cx).can_submit(cx) {
                    return;
                }
                // Revalidate destination-specific consent against the live draft,
                // rather than trusting a child event's collision policy.
                let form = view.content.read(cx);
                let raw = std::path::PathBuf::from(form.destination.read(cx).value().as_ref());
                let destination = raw
                    .file_name()
                    .and_then(|name| name.to_str())
                    .map(|name| raw.with_file_name(ulos_core::naming::sanitize_filename(name)));
                if destination.as_ref() != Some(&request.destination)
                    || form.source.read(cx).value().trim() != request.source
                    || form.options != request.options
                {
                    return;
                }
                let result = view._owner.update(cx, |owner, cx| {
                    owner.enqueue_with_options(
                        request.source.clone(),
                        request.destination.clone(),
                        request.start_now,
                        request.options.clone(),
                        cx,
                    )
                });
                match result {
                    Ok(id) => {
                        view.content.update(cx, |form, _| form.invalidate_preview());
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
                view.content.update(cx, |form, _| form.invalidate_preview());
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
        let download_id = snapshot.id;
        let close_owner = owner.downgrade();
        window.on_window_should_close(cx, move |_, cx| {
            let _ = close_owner.update(cx, |owner, cx| owner.cancel_shutdown(download_id, cx));
            true
        });
        let content = cx.new(|_| DownloadDetails::new(snapshot));
        let cancel = cx.subscribe_in(
            &content,
            window,
            |view, _, event: &CancelDownload, _, cx| {
                view._owner
                    .update(cx, |owner, cx| owner.cancel_id(event.0, cx));
            },
        );
        let close = cx.subscribe_in(&content, window, |view, _, _: &CloseDetails, window, cx| {
            // Closing passive telemetry never cancels its manager-owned transfer.
            view._owner.update(cx, |owner, cx| {
                owner.cancel_shutdown(view.content.read(cx).snapshot.id, cx)
            });
            window.remove_window();
        });
        let pause = cx.subscribe_in(&content, window, |view, _, event: &TogglePause, _, cx| {
            view._owner
                .update(cx, |owner, cx| owner.toggle_pause(event.0, cx));
        });
        let rate = cx.subscribe_in(&content, window, |view, _, event: &SetRateLimit, _, cx| {
            view._owner.update(cx, |owner, cx| {
                owner.set_rate_limit(event.id, event.bytes_per_second, cx)
            });
        });
        let connections = cx.subscribe_in(
            &content,
            window,
            |view, _, event: &SetConnections, _, cx| {
                view._owner.update(cx, |owner, cx| {
                    owner.set_connections(event.id, event.connections, cx)
                });
            },
        );
        let torrent_limits = cx.subscribe_in(
            &content,
            window,
            |view, _, event: &SetTorrentLimits, _, cx| {
                view._owner.update(cx, |owner, cx| {
                    owner.set_torrent_limits(event.id, event.download, event.upload, cx)
                });
            },
        );
        let torrent_peers = cx.subscribe_in(
            &content,
            window,
            |view, _, event: &SetTorrentPeers, _, cx| {
                view._owner.update(cx, |owner, cx| {
                    owner.set_torrent_peers(event.id, event.peers, cx)
                });
            },
        );
        let retry = cx.subscribe_in(
            &content,
            window,
            |view, _, event: &RetryDownload, window, cx| {
                view._owner.update(cx, |owner, cx| {
                    owner.request_retry(event.id, event.collision, window, cx)
                });
            },
        );
        let shutdown = cx.subscribe_in(
            &content,
            window,
            |view, _, event: &RequestShutdown, window, cx| {
                view._owner
                    .update(cx, |owner, cx| owner.request_shutdown(event.0, window, cx));
            },
        );
        let cancel_shutdown = cx.subscribe_in(
            &content,
            window,
            |view, _, event: &CancelShutdown, _, cx| {
                view._owner
                    .update(cx, |owner, cx| owner.cancel_shutdown(event.0, cx));
            },
        );
        Self {
            content,
            _owner: owner,
            _subscriptions: vec![
                cancel,
                close,
                pause,
                rate,
                connections,
                torrent_limits,
                torrent_peers,
                retry,
                shutdown,
                cancel_shutdown,
                crate::palette::observe_window(window, cx),
            ],
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

#[cfg(test)]
mod tests {
    use super::{add_options, status_options};
    #[cfg(target_os = "macos")]
    use gpui_kit::point;
    use gpui_kit::{TestAppContext, px, size};

    #[gpui::test]
    fn status_chrome_does_not_change_add_window_options(cx: &mut TestAppContext) {
        cx.update(|cx| {
            let status = status_options(cx);
            let add = add_options(cx);
            assert!(add.titlebar.as_ref().unwrap().title.is_none());
            assert!(!add.is_minimizable && !add.is_resizable);
            assert!(add.is_movable && add.show && add.focus);
            assert_eq!(
                status.titlebar.as_ref().unwrap().title.as_deref(),
                Some("Download")
            );
            assert_eq!(status.window_min_size, Some(size(px(580.), px(440.))));
            assert_eq!(add.window_min_size, Some(size(px(480.), px(400.))));
            assert!(!add.titlebar.as_ref().unwrap().appears_transparent);
            assert!(status.show && status.focus && status.is_resizable);
            assert!(status.is_minimizable);
            #[cfg(target_os = "macos")]
            {
                assert!(status.titlebar.as_ref().unwrap().appears_transparent);
                assert!(status.app_owns_titlebar_drag);
                assert_eq!(
                    status.titlebar.as_ref().unwrap().traffic_light_position,
                    Some(point(px(12.), px(17.)))
                );
            }
            #[cfg(not(target_os = "macos"))]
            assert!(!status.titlebar.as_ref().unwrap().appears_transparent);
        });
    }
}
