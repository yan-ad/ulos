mod add_download;
mod clipboard;
mod download_details;
mod download_table;
mod formatting;
mod manager;
#[cfg(any(target_os = "macos", test))]
mod native_menu;
mod native_windows;
mod palette;
mod queue;
mod window_chrome;

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
        cx.bind_keys([
            KeyBinding::new("secondary-n", manager::AddUrl, Some("DownloadManager")),
            KeyBinding::new(
                "secondary-b",
                manager::ToggleSidebar,
                Some("DownloadManager"),
            ),
        ]);
        cx.on_window_closed(|cx, _| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
        let options = window_chrome::manager_window_options(cx);
        let opened = open_window(options, cx, |window, cx| {
            cx.new(|cx| DownloadManager::new(handle, window, cx))
        });
        let (main_window, manager) = match opened {
            Ok(opened) => opened,
            Err(error) => {
                eprintln!("Could not open the Ulos window: {error}");
                cx.quit();
                return;
            }
        };
        #[cfg(target_os = "macos")]
        {
            let subscriptions = native_menu::install(cx, manager.downgrade(), main_window);
            manager.update(cx, |manager, _| {
                manager.native_menu_subscriptions = subscriptions
            });
        }
        #[cfg(not(target_os = "macos"))]
        let _ = (main_window, manager);
        cx.activate(true);
    });
    runtime.shutdown_timeout(std::time::Duration::from_secs(2));
}
