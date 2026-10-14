mod add_download;
mod clipboard;
mod download_details;
mod formatting;
mod manager;
mod palette;
mod queue;

use gpui_kit::*;
use manager::DownloadManager;

fn main() {
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("Could not start the transfer runtime: {error}");
            return;
        }
    };
    let handle = runtime.handle().clone();
    application().with_assets(assets::AllAssets).run(move |cx| {
        init(cx);
        palette::init(cx);
        cx.bind_keys([KeyBinding::new(
            "secondary-n",
            manager::AddUrl,
            Some("DownloadManager"),
        )]);
        cx.on_window_closed(|cx, _| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                None,
                size(px(1280.0), px(760.0)),
                cx,
            ))),
            window_min_size: Some(size(px(880.0), px(540.0))),
            titlebar: Some(TitlebarOptions {
                title: Some("Ulos".into()),
                ..Default::default()
            }),
            ..Default::default()
        };
        if let Err(error) = open_window(options, cx, |window, cx| {
            cx.new(|cx| DownloadManager::new(handle, window, cx))
        }) {
            eprintln!("Could not open the Ulos window: {error}");
            cx.quit();
            return;
        }
        cx.activate(true);
    });
    runtime.shutdown_timeout(std::time::Duration::from_secs(2));
}
