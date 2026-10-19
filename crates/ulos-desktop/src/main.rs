#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

mod add_download;
mod add_torrent;
mod clipboard;
mod completion;
mod config;
mod download_details;
mod download_table;
mod download_task;
mod file_icons;
mod formatting;
mod intake;
mod manager;
#[cfg(any(target_os = "macos", test))]
mod native_menu;
mod native_windows;
mod palette;
mod platform_paths;
mod queue;
mod scheduler;
mod scheduler_window;
mod settings_window;
mod storage;
mod updates;
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
    let (paths, history, writer, history_error, settings) = match paths {
        Ok(paths) => match runtime.block_on(storage::load_history(paths.history.clone())) {
            Ok(records) => {
                let writer = storage::HistoryWriter::new(&handle, paths.history.clone());
                let settings = runtime
                    .block_on(tokio::task::spawn_blocking({
                        let path = paths.config.clone();
                        move || config::load(path)
                    }))
                    .unwrap_or_else(|_| config::LoadedSettings::unavailable());
                (Some(paths), records, Some(writer), None, settings)
            }
            Err(error) => {
                let settings = runtime
                    .block_on(tokio::task::spawn_blocking({
                        let path = paths.config.clone();
                        move || config::load(path)
                    }))
                    .unwrap_or_else(|_| config::LoadedSettings::unavailable());
                (Some(paths), Vec::new(), None, Some(error), settings)
            }
        },
        Err(error) => (
            None,
            Vec::new(),
            None,
            Some(error),
            config::LoadedSettings::unavailable(),
        ),
    };
    let flush_writer = writer.clone();
    let inbox = std::rc::Rc::new(std::cell::RefCell::new(Vec::<std::path::PathBuf>::new()));
    let intake_error = std::rc::Rc::new(std::cell::RefCell::new(None::<String>));
    match intake::argument_paths(std::env::args_os().skip(1)) {
        Ok(paths) => *inbox.borrow_mut() = paths,
        Err(error) => *intake_error.borrow_mut() = Some(error.into()),
    }
    let app = application().with_assets(assets::AllAssets);
    let opened = inbox.clone();
    let opened_error = intake_error.clone();
    app.on_open_urls(move |urls| match intake::opened_urls(urls) {
        Ok(paths) => {
            let mut pending = opened.borrow_mut();
            if let Err(error) = intake::merge_pending(&mut pending, paths) {
                *opened_error.borrow_mut() = Some(error.into());
            }
        }
        Err(error) => *opened_error.borrow_mut() = Some(error.into()),
    });
    app.run(move |cx| {
        init(cx);
        cx.set_app_identity("org.ulos.desktop", "Ulos");
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
            manager.initialize_settings(settings, cx);
            manager.watch_torrent_intake(inbox, intake_error, cx);
        });
        let update_owner = manager.downgrade();
        cx.defer(move |cx| {
            let _ = main_window.update(cx, |_, window, cx| {
                let _ =
                    update_owner.update(cx, |manager, cx| manager.check_updates(window, cx, false));
            });
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
