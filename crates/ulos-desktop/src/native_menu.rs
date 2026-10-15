//! Native application menus, using only GPUI Kit's platform-neutral menu API.

use std::{cell::RefCell, rc::Rc};

use gpui_kit::{
    Action, AnyWindowHandle, App, KeyBinding, Menu, MenuItem, Subscription, SystemMenuType,
    WeakEntity,
    component::input::{Copy, Cut, Paste, Redo, SelectAll, Undo},
};

use crate::{
    manager::{DownloadManager, MenuStateChanged},
    queue::Filter,
};

gpui_kit::actions!(
    native_commands,
    [
        NewDownload,
        StartDownloads,
        StopDownloadsQueue,
        StopDownloads,
        StopSelectedDownload,
        RemoveDownloads,
        ClearFinishedDownloads,
        OpenOptions,
        OpenScheduler,
        ShowDownloadDetails,
        RevealDownloads,
        ShowAllDownloads,
        ShowFinishedDownloads,
        ShowUnfinishedDownloads,
        ShowQueuedDownloads,
        ToggleDownloadsSidebar,
        UseSystemTheme,
        UseLightTheme,
        UseDarkTheme,
        ShowHelp,
        QuitApplication
    ]
);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    Add,
    Start,
    StopQueue,
    StopAll,
    StopSelected,
    Remove,
    Clear,
    Options,
    Scheduler,
    Details,
    Reveal,
    All,
    Finished,
    Unfinished,
    Queued,
    ToggleSidebar,
    SystemTheme,
    LightTheme,
    DarkTheme,
    Help,
}

/// A value snapshot shared by menu construction and dispatch-time validation.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MenuState {
    pub can_add: bool,
    pub can_start: bool,
    pub queue_running: bool,
    pub can_stop: bool,
    pub selected_active: bool,
    pub removable: bool,
    pub finished: bool,
    pub selected: bool,
    pub revealable: bool,
    pub can_options: bool,
    pub dark: bool,
    pub filter: Filter,
    pub sidebar_collapsed: bool,
    pub theme_preference: crate::palette::ThemePreference,
}

impl Command {
    pub fn enabled(self, state: &MenuState) -> bool {
        match self {
            Self::Add => state.can_add,
            Self::Start => state.can_start,
            Self::StopQueue => state.queue_running,
            Self::StopAll => state.can_stop,
            Self::StopSelected => state.selected_active,
            Self::Remove => state.removable,
            Self::Clear => state.finished,
            Self::Options => state.can_options,
            Self::Details => state.selected,
            Self::Scheduler => true,
            Self::Reveal => state.revealable,
            Self::All
            | Self::Finished
            | Self::Unfinished
            | Self::Queued
            | Self::ToggleSidebar
            | Self::SystemTheme
            | Self::LightTheme
            | Self::DarkTheme
            | Self::Help => true,
        }
    }
}

fn item(label: &str, action: impl Action, command: Command, state: &MenuState) -> MenuItem {
    MenuItem::action(label.to_owned(), action).disabled(!command.enabled(state))
}

pub fn menus(state: &MenuState) -> Vec<Menu> {
    vec![
        Menu::new("Ulos").items([
            item("Options…", OpenOptions, Command::Options, state),
            MenuItem::separator(),
            MenuItem::os_submenu("Services", SystemMenuType::Services),
            MenuItem::separator(),
            MenuItem::action("Quit Ulos", QuitApplication),
        ]),
        Menu::new("Tasks").items([
            item("Add URL…", NewDownload, Command::Add, state),
            item("Start Queue", StartDownloads, Command::Start, state),
            item("Stop Queue", StopDownloadsQueue, Command::StopQueue, state),
            item("Stop All", StopDownloads, Command::StopAll, state),
            item(
                "Scheduling & Queues…",
                OpenScheduler,
                Command::Scheduler,
                state,
            ),
        ]),
        Menu::new("File").items([
            item("Add URL…", NewDownload, Command::Add, state),
            item("Details…", ShowDownloadDetails, Command::Details, state),
            item("Show in Folder", RevealDownloads, Command::Reveal, state),
        ]),
        // These are the input component's real responder-chain actions, not
        // application handlers: editing stays with the focused input control.
        Menu::new("Edit").items([
            MenuItem::action("Undo", Undo),
            MenuItem::action("Redo", Redo),
            MenuItem::separator(),
            MenuItem::action("Cut", Cut),
            MenuItem::action("Copy", Copy),
            MenuItem::action("Paste", Paste),
            MenuItem::separator(),
            MenuItem::action("Select All", SelectAll),
        ]),
        Menu::new("Downloads").items([
            item(
                "Stop Selected",
                StopSelectedDownload,
                Command::StopSelected,
                state,
            ),
            item("Remove", RemoveDownloads, Command::Remove, state),
            item(
                "Clear Finished Records",
                ClearFinishedDownloads,
                Command::Clear,
                state,
            ),
            MenuItem::separator(),
            item("Options…", OpenOptions, Command::Options, state),
        ]),
        Menu::new("View").items([
            item(
                if state.sidebar_collapsed {
                    "Expand Sidebar"
                } else {
                    "Collapse Sidebar"
                },
                ToggleDownloadsSidebar,
                Command::ToggleSidebar,
                state,
            ),
            MenuItem::separator(),
            item("All Downloads", ShowAllDownloads, Command::All, state)
                .checked(state.filter == Filter::All),
            item("Finished", ShowFinishedDownloads, Command::Finished, state)
                .checked(state.filter == Filter::Finished),
            item(
                "Unfinished",
                ShowUnfinishedDownloads,
                Command::Unfinished,
                state,
            )
            .checked(state.filter == Filter::Unfinished),
            item("Queued", ShowQueuedDownloads, Command::Queued, state)
                .checked(state.filter == Filter::Queued),
            MenuItem::separator(),
            item("Follow System", UseSystemTheme, Command::SystemTheme, state)
                .checked(state.theme_preference == crate::palette::ThemePreference::System),
            item("Light Theme", UseLightTheme, Command::LightTheme, state)
                .checked(state.theme_preference == crate::palette::ThemePreference::Light),
            item("Dark Theme", UseDarkTheme, Command::DarkTheme, state)
                .checked(state.theme_preference == crate::palette::ThemePreference::Dark),
        ]),
        Menu::new("Help").items([item("Ulos Help", ShowHelp, Command::Help, state)]),
    ]
}

type MenuCache = Rc<RefCell<Option<MenuState>>>;

fn refresh(owner: &WeakEntity<DownloadManager>, cache: &MenuCache, cx: &mut App) {
    let state = owner
        .upgrade()
        .map(|manager| manager.read(cx).menu_state(cx))
        .unwrap_or_default();
    if cache.borrow().as_ref() != Some(&state) {
        cx.set_menus(menus(&state));
        *cache.borrow_mut() = Some(state);
    }
}

fn register<A: Action>(
    cx: &mut App,
    owner: &WeakEntity<DownloadManager>,
    main: AnyWindowHandle,
    command: Command,
) {
    let owner = owner.clone();
    cx.on_action::<A>(move |_, cx| {
        let Some(manager) = owner.upgrade() else {
            return;
        };
        if !command.enabled(&manager.read(cx).menu_state(cx)) {
            return;
        }
        let target = if command == Command::Options {
            main
        } else {
            cx.active_window().unwrap_or(main)
        };
        cx.defer(move |cx| {
            let _ = target.update(cx, |_, window, cx| {
                if command.enabled(&manager.read(cx).menu_state(cx)) {
                    manager.update(cx, |manager, cx| {
                        manager.run_menu_command(command, window, cx)
                    });
                }
            });
        });
    });
}

/// Install once per application. Retain the returned subscriptions for the
/// application's lifetime; global action callbacks retain only a weak owner.
pub fn install(
    cx: &mut App,
    owner: WeakEntity<DownloadManager>,
    main: AnyWindowHandle,
) -> Vec<Subscription> {
    register::<NewDownload>(cx, &owner, main, Command::Add);
    register::<StartDownloads>(cx, &owner, main, Command::Start);
    register::<StopDownloadsQueue>(cx, &owner, main, Command::StopQueue);
    register::<StopDownloads>(cx, &owner, main, Command::StopAll);
    register::<StopSelectedDownload>(cx, &owner, main, Command::StopSelected);
    register::<RemoveDownloads>(cx, &owner, main, Command::Remove);
    register::<ClearFinishedDownloads>(cx, &owner, main, Command::Clear);
    register::<OpenOptions>(cx, &owner, main, Command::Options);
    register::<OpenScheduler>(cx, &owner, main, Command::Scheduler);
    register::<ShowDownloadDetails>(cx, &owner, main, Command::Details);
    register::<RevealDownloads>(cx, &owner, main, Command::Reveal);
    register::<ShowAllDownloads>(cx, &owner, main, Command::All);
    register::<ShowFinishedDownloads>(cx, &owner, main, Command::Finished);
    register::<ShowUnfinishedDownloads>(cx, &owner, main, Command::Unfinished);
    register::<ShowQueuedDownloads>(cx, &owner, main, Command::Queued);
    register::<ToggleDownloadsSidebar>(cx, &owner, main, Command::ToggleSidebar);
    register::<UseSystemTheme>(cx, &owner, main, Command::SystemTheme);
    register::<UseLightTheme>(cx, &owner, main, Command::LightTheme);
    register::<UseDarkTheme>(cx, &owner, main, Command::DarkTheme);
    register::<ShowHelp>(cx, &owner, main, Command::Help);
    cx.on_action::<QuitApplication>(|_, cx| cx.quit());
    cx.bind_keys([
        KeyBinding::new("secondary-q", QuitApplication, None),
        KeyBinding::new("secondary-n", NewDownload, None),
        KeyBinding::new("secondary-,", OpenOptions, None),
    ]);

    let cache = Rc::new(RefCell::new(None));
    refresh(&owner, &cache, cx);
    let mut subscriptions = Vec::new();
    if let Some(manager) = owner.upgrade() {
        let observed_owner = owner.clone();
        let observed_cache = cache.clone();
        subscriptions.push(cx.observe(&manager, move |_, cx| {
            refresh(&observed_owner, &observed_cache, cx);
        }));
        let observed_owner = owner.clone();
        let observed_cache = cache.clone();
        subscriptions.push(cx.subscribe(&manager, move |_, _: &MenuStateChanged, cx| {
            refresh(&observed_owner, &observed_cache, cx);
        }));
    }
    let theme_owner = owner.clone();
    let theme_cache = cache.clone();
    subscriptions.push(cx.observe_global::<gpui_kit::component::Theme>(move |cx| {
        refresh(&theme_owner, &theme_cache, cx)
    }));
    let preference_owner = owner.clone();
    let preference_cache = cache.clone();
    subscriptions.push(crate::palette::observe_preference(cx, move |cx| {
        refresh(&preference_owner, &preference_cache, cx)
    }));
    subscriptions.push(cx.on_window_closed(move |cx, _| {
        let owner = owner.clone();
        let cache = cache.clone();
        cx.defer(move |cx| refresh(&owner, &cache, cx));
    }));
    subscriptions
}

#[cfg(test)]
mod tests {
    use super::{Command, MenuState, NewDownload, ShowHelp, menus};
    use crate::queue::Filter;
    use gpui_kit::{Action, Menu, MenuItem};

    fn menu<'a>(menus: &'a [Menu], name: &str) -> &'a Menu {
        menus
            .iter()
            .find(|menu| menu.name.as_ref() == name)
            .unwrap()
    }

    #[test]
    fn includes_application_and_requested_native_menus() {
        let menus = menus(&MenuState::default());
        let names: Vec<&str> = menus.iter().map(|menu| menu.name.as_ref()).collect();
        assert_eq!(
            names,
            ["Ulos", "Tasks", "File", "Edit", "Downloads", "View", "Help"]
        );
        assert!(matches!(menus[0].items[2], MenuItem::SystemMenu(_)));
    }

    #[test]
    fn common_commands_use_their_independent_state_flags() {
        let commands = [
            Command::Add,
            Command::Start,
            Command::StopQueue,
            Command::StopAll,
            Command::StopSelected,
            Command::Remove,
            Command::Clear,
            Command::Options,
            Command::Details,
            Command::Reveal,
        ];
        let states = [
            MenuState {
                can_add: true,
                ..Default::default()
            },
            MenuState {
                can_start: true,
                ..Default::default()
            },
            MenuState {
                queue_running: true,
                ..Default::default()
            },
            MenuState {
                can_stop: true,
                ..Default::default()
            },
            MenuState {
                selected_active: true,
                ..Default::default()
            },
            MenuState {
                removable: true,
                ..Default::default()
            },
            MenuState {
                finished: true,
                ..Default::default()
            },
            MenuState {
                can_options: true,
                ..Default::default()
            },
            MenuState {
                selected: true,
                ..Default::default()
            },
            MenuState {
                revealable: true,
                ..Default::default()
            },
        ];
        for (index, command) in commands.iter().enumerate() {
            assert!(!command.enabled(&MenuState::default()));
            for (state_index, state) in states.iter().enumerate() {
                assert_eq!(command.enabled(state), index == state_index);
            }
        }
    }

    #[test]
    fn navigation_theme_and_help_are_always_enabled() {
        for command in [
            Command::All,
            Command::Finished,
            Command::Unfinished,
            Command::Queued,
            Command::ToggleSidebar,
            Command::SystemTheme,
            Command::LightTheme,
            Command::DarkTheme,
            Command::Help,
        ] {
            assert!(command.enabled(&MenuState::default()));
        }
    }

    #[test]
    fn filters_and_theme_have_exclusive_checkmarks() {
        for filter in [
            Filter::All,
            Filter::Finished,
            Filter::Unfinished,
            Filter::Queued,
        ] {
            for theme_preference in crate::palette::ThemePreference::ALL {
                for dark in [false, true] {
                    let menus = menus(&MenuState {
                        filter,
                        dark,
                        theme_preference,
                        ..Default::default()
                    });
                    let view = menu(&menus, "View");
                    for (index, candidate) in [
                        Filter::All,
                        Filter::Finished,
                        Filter::Unfinished,
                        Filter::Queued,
                    ]
                    .iter()
                    .enumerate()
                    {
                        assert_eq!(view.items[index + 2].is_checked(), filter == *candidate);
                    }
                    assert_eq!(
                        view.items[7].is_checked(),
                        theme_preference == crate::palette::ThemePreference::System
                    );
                    assert_eq!(
                        view.items[8].is_checked(),
                        theme_preference == crate::palette::ThemePreference::Light
                    );
                    assert_eq!(
                        view.items[9].is_checked(),
                        theme_preference == crate::palette::ThemePreference::Dark
                    );
                }
            }
        }
    }

    #[test]
    fn disabled_native_items_keep_their_typed_actions() {
        let menus = menus(&MenuState::default());
        let add = &menu(&menus, "File").items[0];
        assert!(add.is_disabled());
        let MenuItem::Action { action, .. } = add else {
            panic!("expected action")
        };
        assert_eq!(action.name(), NewDownload::name_for_type());
        let help = &menu(&menus, "Help").items[0];
        assert!(!help.is_disabled());
        let MenuItem::Action { action, .. } = help else {
            panic!("expected action")
        };
        assert_eq!(action.name(), ShowHelp::name_for_type());
        assert!(menu(&menus, "Ulos").items[0].is_disabled());
        assert!(!menu(&menus, "Ulos").items[4].is_disabled());
    }
}
