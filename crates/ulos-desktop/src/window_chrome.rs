use gpui_kit::*;

pub(crate) fn manager_window_options(cx: &App) -> WindowOptions {
    #[cfg(target_os = "windows")]
    let mut base = gpui_kit::component::TitleBar::window_options();
    #[cfg(not(target_os = "windows"))]
    let mut base = WindowOptions::default();
    let mut titlebar = base.titlebar.take().unwrap_or_default();
    titlebar.title = Some("Ulos".into());
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
            None,
            size(px(1280.), px(760.)),
            cx,
        ))),
        window_min_size: Some(size(px(880.), px(540.))),
        titlebar: Some(titlebar),
        ..base
    };
    #[cfg(target_os = "macos")]
    {
        let mut options = options;
        let titlebar = options
            .titlebar
            .as_mut()
            .expect("Manager titlebar configured");
        titlebar.appears_transparent = true;
        titlebar.traffic_light_position = Some(point(px(12.), px(17.)));
        options.app_owns_titlebar_drag =
            gpui_kit::component::TitleBar::window_options().app_owns_titlebar_drag;
        options
    }
    #[cfg(not(target_os = "macos"))]
    options
}

#[cfg(test)]
mod tests {
    use super::manager_window_options;

    #[gpui::test]
    fn manager_chrome_preserves_native_controls_and_window_metadata(
        cx: &mut gpui_kit::TestAppContext,
    ) {
        cx.update(|cx| {
            let options = manager_window_options(cx);
            assert_eq!(
                options.titlebar.as_ref().unwrap().title.as_deref(),
                Some("Ulos")
            );
            assert!(options.show && options.focus && options.is_resizable);
            assert_eq!(
                options.window_min_size,
                Some(gpui_kit::size(gpui_kit::px(880.), gpui_kit::px(540.)))
            );
            assert!(options.window_bounds.is_some());
            #[cfg(target_os = "windows")]
            {
                assert!(options.titlebar.as_ref().unwrap().appears_transparent);
                assert_eq!(
                    options.app_owns_titlebar_drag,
                    gpui_kit::component::TitleBar::window_options().app_owns_titlebar_drag
                );
            }
            #[cfg(target_os = "macos")]
            {
                assert!(options.titlebar.as_ref().unwrap().appears_transparent);
                assert!(options.app_owns_titlebar_drag);
                assert!(
                    options
                        .titlebar
                        .as_ref()
                        .unwrap()
                        .traffic_light_position
                        .is_some()
                );
            }
            #[cfg(not(any(target_os = "macos", target_os = "windows")))]
            assert!(!options.titlebar.as_ref().unwrap().appears_transparent);
        });
    }
}
