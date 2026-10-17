use super::*;
use gpui_kit::component::{
    Icon, button::Button, group_box::GroupBox, label::Label, notification::Notification,
    toolbar::Toolbar,
};

struct ReleaseNotice;

pub(crate) fn release_notification(release: crate::updates::Release) -> Notification {
    Notification::new()
        .id::<ReleaseNotice>()
        .title(format!("Ulos {} is available", release.version))
        .message("Download the new version from GitHub Releases.")
        .autohide(false)
        .content(move |_, _, _| {
            let url = release.url.clone();
            Toolbar::new("update-actions")
                .content(
                    Button::new("whats-new")
                        .debug_selector(|| "whats-new".into())
                        .label("What's New")
                        .on_click(move |_, _, cx| cx.open_url(&url)),
                )
                .content(
                    Button::new("close-update")
                        .debug_selector(|| "close-update".into())
                        .label("Close")
                        .on_click(|_, window, cx| window.remove_notification::<ReleaseNotice>(cx)),
                )
                .into_any_element()
        })
}

impl DownloadManager {
    pub(crate) fn check_updates(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
        manual: bool,
    ) {
        if self.update_checking || !cx.windows().contains(&self.main_window) {
            return;
        }
        self.update_checking = true;
        let worker = self
            .runtime
            .spawn(crate::updates::check(crate::updates::VERSION));
        self._update_check = Some(cx.spawn(async move |view, cx| {
            while !worker.is_finished() {
                cx.background_executor()
                    .timer(Duration::from_millis(25))
                    .await;
            }
            let result = worker
                .await
                .unwrap_or(Err("The update check stopped unexpectedly."));
            let _ = view.update(cx, |view, cx| {
                view.update_checking = false;
                let target = view.main_window;
                cx.defer(move |cx| {
                    let _ = target.update(cx, |_, window, cx| match result {
                        Ok(Some(release)) => {
                            window.push_notification(release_notification(release), cx)
                        }
                        Ok(None) if manual => window.push_notification(
                            "You're using the latest available Ulos release.",
                            cx,
                        ),
                        Err(error) if manual => window.push_notification(error, cx),
                        _ => (),
                    });
                });
            });
        }));
    }

    pub(crate) fn about(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.migration.busy
            || self.about_open
            || self.options_open
            || self.removal_open
            || self.removal_busy
        {
            return;
        }
        self.about_open = true;
        let owner = cx.weak_entity();
        window.open_dialog(cx, move |dialog, _, _| {
            let owner = owner.clone();
            let close_owner = owner.clone();
            dialog
                .title("About Ulos")
                .child(
                    GroupBox::new()
                        .child(
                            Icon::default()
                                .data(include_bytes!("../../../assets/branding/ulos.svg"))
                                .size(px(80.)),
                        )
                        .child(Label::new("Ulos Download Manager"))
                        .child(Label::new(format!("Version {}", crate::updates::VERSION)))
                        .child(Label::new(
                            "Copyright © 2026 Ulos Download Manager contributors.",
                        ))
                        .child(Label::new(
                            "MIT licensed. Includes third-party software; see the bundled notices.",
                        )),
                )
                .footer(
                    Toolbar::new("about-actions")
                        .content(
                            Button::new("about-github")
                                .debug_selector(|| "about-github".into())
                                .label("GitHub")
                                .on_click(|_, _, cx| cx.open_url(crate::updates::REPOSITORY_URL)),
                        )
                        .content(
                            Button::new("about-close")
                                .debug_selector(|| "about-close".into())
                                .label("Close")
                                .on_click(move |_, window, cx| {
                                    let _ = close_owner.update(cx, |view, cx| {
                                        view.about_open = false;
                                        cx.notify();
                                    });
                                    window.close_dialog(cx);
                                }),
                        ),
                )
                .on_close(move |_, _, cx| {
                    let _ = owner.update(cx, |view, cx| {
                        view.about_open = false;
                        cx.notify();
                    });
                })
        });
        cx.notify();
    }
}
