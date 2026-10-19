use gpui_kit::component::{ActiveTheme, v_flex};

use super::*;
use crate::{
    config::LoadedSettings,
    settings_window::{SettingsDismiss, SettingsSaved, SettingsWindow},
};

impl DownloadManager {
    pub(crate) fn initialize_settings(&mut self, settings: LoadedSettings, cx: &mut Context<Self>) {
        self.settings = settings;
        apply_theme(self.settings.snapshot.theme, cx);
        cx.notify();
    }
    pub(crate) fn open_settings(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        if let Some(child) = &self.settings_window
            && cx.windows().contains(&child.handle)
        {
            let handle = child.handle;
            cx.defer(move |cx| {
                let _ = handle.update(cx, |_, window, _| window.activate_window());
            });
            return;
        }
        self.settings_window = None;
        if !cx.windows().contains(&self.main_window)
            || self.migration.busy
            || !self.add_windows.is_empty()
            || !self.torrent_windows.is_empty()
            || self.torrent_picker_open
            || !self.detail_windows.is_empty()
            || self.about_open
            || self.removal_open
            || self.removal_busy
            || self.scheduler_window.is_some()
        {
            return;
        }
        let owner = cx.entity();
        let baseline = self.settings.clone();
        let clipboard = self.clipboard_enabled;
        let runtime = self.runtime.clone();
        let mut options = crate::native_windows::child_options("Settings", 840., 720., cx);
        options.kind = WindowKind::Dialog;
        options.is_minimizable = false;
        options.is_resizable = false;
        match open_window(options, cx, move |window, cx| {
            cx.new(|cx| NativeSettingsWindow::new(owner, baseline, clipboard, runtime, window, cx))
        }) {
            Ok((handle, root)) => {
                self.settings_window = Some(ChildWindow {
                    handle,
                    content: root.read(cx).content.downgrade(),
                });
                self.options_open = true;
                cx.notify();
            }
            Err(_) => {
                self.notice = Some("Could not open Settings.".into());
                cx.notify();
            }
        }
    }
}
fn apply_theme(theme: crate::config::ThemePreference, cx: &mut App) {
    crate::palette::choose(
        match theme {
            crate::config::ThemePreference::System => crate::palette::ThemePreference::System,
            crate::config::ThemePreference::Light => crate::palette::ThemePreference::Light,
            crate::config::ThemePreference::Dark => crate::palette::ThemePreference::Dark,
        },
        cx,
    );
}
struct NativeSettingsWindow {
    content: Entity<SettingsWindow>,
    _owner: Entity<DownloadManager>,
    _subscriptions: Vec<Subscription>,
}
impl NativeSettingsWindow {
    fn new(
        owner: Entity<DownloadManager>,
        baseline: LoadedSettings,
        clipboard: bool,
        runtime: tokio::runtime::Handle,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let content = cx.new(|cx| SettingsWindow::new(baseline, clipboard, runtime, window, cx));
        let weak = content.downgrade();
        window.on_window_should_close(cx, move |_, cx| {
            weak.upgrade().is_none_or(|content| !content.read(cx).busy)
        });
        let close_owner = owner.downgrade();
        let closed = cx.on_release(move |_, cx| {
            let _ = close_owner.update(cx, |view, cx| {
                view.options_open = false;
                view.settings_window = None;
                cx.notify();
            });
        });
        let saved = cx.subscribe_in(
            &content,
            window,
            |root, _, event: &SettingsSaved, window, cx| {
                if !root.content.read(cx).accepts_saved(event) {
                    return;
                }
                root._owner.update(cx, |view, cx| {
                    view.settings = event.loaded.clone();
                    view.clipboard_enabled = event.clipboard_enabled;
                    apply_theme(view.settings.snapshot.theme, cx);
                    cx.notify();
                });
                window.remove_window();
            },
        );
        let dismiss = cx.subscribe_in(
            &content,
            window,
            |root, _, _: &SettingsDismiss, window, cx| {
                if !root.content.read(cx).busy {
                    window.remove_window();
                }
            },
        );
        Self {
            content,
            _owner: owner,
            _subscriptions: vec![
                closed,
                saved,
                dismiss,
                crate::palette::observe_window(window, cx),
            ],
        }
    }
}
impl Render for NativeSettingsWindow {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .size_full()
            .bg(cx.theme().background)
            .child(self.content.clone())
    }
}

#[cfg(test)]
#[path = "manager_settings_tests.rs"]
mod tests;
