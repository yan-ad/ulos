use super::{DownloadManager, application::release_notification};
use gpui_kit::{
    AppContext, Entity, TestAppContext, VisualTestContext,
    component::{Root, WindowExt},
};

fn mount<'a>(
    cx: &'a mut TestAppContext,
    runtime: &tokio::runtime::Runtime,
) -> (
    Entity<Root>,
    Entity<DownloadManager>,
    &'a mut VisualTestContext,
) {
    cx.update(gpui_kit::init);
    cx.update(crate::palette::init);
    let mut owner = None;
    let handle = runtime.handle().clone();
    let (root, cx) = cx.add_window_view(|window, cx| {
        let view = cx.new(|cx| DownloadManager::new(handle, window, cx));
        owner = Some(view.clone());
        Root::new(view, window, cx)
    });
    cx.update(|_, cx| cx.set_reduce_motion(true));
    draw(cx);
    (root, owner.unwrap(), cx)
}
fn draw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
}
#[gpui::test]
fn update_toast_opens_release_notes_and_close_dismisses(cx: &mut TestAppContext) {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let (_root, _view, cx) = mount(cx, &runtime);
    let url = "https://github.com/yan-ad/ulos/releases/tag/v0.0.2";
    cx.update(|window, cx| {
        window.push_notification(
            release_notification(crate::updates::Release {
                version: "0.0.2".into(),
                url: url.into(),
            }),
            cx,
        )
    });
    draw(cx);
    let notes = cx
        .debug_bounds("whats-new")
        .expect("What's New stock Button");
    assert!(cx.debug_bounds("close-update").is_some());
    cx.simulate_click(notes.center(), gpui_kit::Modifiers::default());
    draw(cx);
    assert_eq!(cx.opened_url().as_deref(), Some(url));
    let close = cx.debug_bounds("close-update").unwrap();
    cx.simulate_click(close.center(), gpui_kit::Modifiers::default());
    draw(cx);
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(1));
    draw(cx);
    assert!(cx.debug_bounds("close-update").is_none());
    assert!(cx.shown_system_notifications().is_empty());
}
#[gpui::test]
fn native_about_has_version_and_repository_and_resets_modal_state(cx: &mut TestAppContext) {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let (_root, view, cx) = mount(cx, &runtime);
    let _menus = cx.update(|window, cx| {
        crate::native_menu::install(cx, view.downgrade(), window.window_handle())
    });
    cx.dispatch_action(crate::native_menu::AboutUlos);
    draw(cx);
    view.read_with(cx, |view, _| assert!(view.about_open));
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.clipboard_enabled = true;
            view.offer_clipboard(Some("https://example.invalid/file"), window, cx);
            assert!(view.add_windows.is_empty());
        })
    });
    let github = cx.debug_bounds("about-github").unwrap();
    cx.simulate_click(github.center(), gpui_kit::Modifiers::default());
    draw(cx);
    assert_eq!(
        cx.opened_url().as_deref(),
        Some(crate::updates::REPOSITORY_URL)
    );
    assert_eq!(crate::updates::VERSION, env!("CARGO_PKG_VERSION"));
    let close = cx.debug_bounds("about-close").unwrap();
    cx.simulate_click(close.center(), gpui_kit::Modifiers::default());
    draw(cx);
    view.read_with(cx, |view, _| assert!(!view.about_open));
    cx.update(|window, cx| assert!(!window.has_active_dialog(cx)));
}
