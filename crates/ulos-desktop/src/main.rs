mod add_download;
mod clipboard;
mod completion;
mod download_details;
mod download_table;
mod file_icons;
mod formatting;
mod manager;
#[cfg(any(target_os = "macos", test))]
mod native_menu;
mod native_windows;
mod palette;
mod platform_paths;
mod queue;
mod scheduler;
mod scheduler_window;
mod storage;
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
    // Resolve platform paths and load SQLite before any queue snapshot can be saved.
    let paths = runtime.block_on(async {
        tokio::task::spawn_blocking(platform_paths::PlatformPaths::detect)
            .await
            .unwrap_or_else(|_| Err("Platform location worker stopped unexpectedly.".into()))
    });
    let (paths, history, writer, history_error) = match paths {
        Ok(paths) => match runtime.block_on(storage::load_history(paths.history.clone())) {
            Ok(records) => {
                let writer = storage::HistoryWriter::new(&handle, paths.history.clone());
                (Some(paths), records, Some(writer), None)
            }
            Err(error) => (Some(paths), Vec::new(), None, Some(error)),
        },
        Err(error) => (None, Vec::new(), None, Some(error)),
    };
    let flush_writer = writer.clone();
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
        manager.update(cx, |manager, cx| {
            manager.initialize_history(paths, history, writer, history_error, cx);
        });
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
    if let Some(writer) = flush_writer
        && let Err(error) = runtime.block_on(writer.flush())
    {
        eprintln!("History could not be saved: {error}");
    }
    runtime.shutdown_timeout(std::time::Duration::from_secs(2));
}
