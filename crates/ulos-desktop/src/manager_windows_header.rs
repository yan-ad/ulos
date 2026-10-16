//! Windows-only command surfaces; tests mount these same methods on other hosts.
use super::*;
use gpui_kit::component::{
    TitleBar,
    button::Button,
    tab::{Tab, TabBar},
    toolbar::Toolbar,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CommandTab {
    Tasks,
    File,
    Downloads,
    View,
    Help,
}

impl CommandTab {
    const ALL: [Self; 5] = [
        Self::Tasks,
        Self::File,
        Self::Downloads,
        Self::View,
        Self::Help,
    ];
    fn selector(self) -> &'static str {
        match self {
            Self::Tasks => "windows-tab-Tasks",
            Self::File => "windows-tab-File",
            Self::Downloads => "windows-tab-Downloads",
            Self::View => "windows-tab-View",
            Self::Help => "windows-tab-Help",
        }
    }
    fn label(self) -> &'static str {
        match self {
            Self::Tasks => "Tasks",
            Self::File => "File",
            Self::Downloads => "Downloads",
            Self::View => "View",
            Self::Help => "Help",
        }
    }
}

#[derive(Clone, Copy)]
enum Command {
    Add,
    Torrent,
    Details,
    Reveal,
    StopSelected,
    Remove,
    Clear,
    Options,
    ToggleSidebar,
    Filter(Filter),
    About,
    Updates,
}

impl DownloadManager {
    pub(super) fn render_windows_header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        TitleBar::new().child(
            TabBar::new("windows-command-tabs")
                .underline()
                .selected_index(
                    CommandTab::ALL
                        .iter()
                        .position(|tab| *tab == self.windows_command_tab)
                        .unwrap_or(0),
                )
                .children(CommandTab::ALL.map(|tab| {
                    Tab::new()
                        .label(tab.label())
                        .debug_selector(move || tab.selector().to_string())
                }))
                .on_click(cx.listener(|view, index: &usize, _, cx| {
                    if let Some(tab) = CommandTab::ALL.get(*index) {
                        view.windows_command_tab = *tab;
                        cx.notify();
                    }
                })),
        )
    }

    fn windows_command_enabled(&self, command: Command) -> bool {
        let selected = |predicate: fn(&crate::queue::QueueItem) -> bool| {
            self.queue
                .items
                .iter()
                .any(|item| self.queue.selected.contains(&item.id) && predicate(item))
        };
        match command {
            Command::Add | Command::Torrent => {
                self.add_windows.len() + self.torrent_windows.len() < 16
            }
            Command::Details => self
                .queue
                .items
                .iter()
                .any(|item| self.queue.selected.contains(&item.id)),
            Command::Reveal => selected(|item| item.is_finished()),
            Command::StopSelected => selected(|item| item.is_active()),
            Command::Remove => selected(|item| !item.is_active()) && !self.removal_busy,
            Command::Clear => {
                self.queue.items.iter().any(|item| item.is_finished()) && !self.removal_busy
            }
            Command::Options => {
                !self.options_open && !self.about_open && self.scheduler_window.is_none()
            }
            Command::About => {
                !self.about_open && !self.options_open && !self.removal_open && !self.removal_busy
            }
            Command::Updates => !self.update_checking,
            _ => true,
        }
    }

    fn windows_command_button(
        &self,
        id: &'static str,
        label: &'static str,
        command: Command,
        cx: &mut Context<Self>,
    ) -> Button {
        let owner = cx.weak_entity();
        let main_window = self.main_window;
        Button::new(id)
            .label(label)
            .accessibility_label(label)
            .tooltip(label)
            .debug_selector(move || id.into())
            .disabled(!self.windows_command_enabled(command))
            .on_click(move |_, _, cx| {
                // Enter the target window only after the event's current update ends.
                let owner = owner.clone();
                cx.defer(move |cx| {
                    let _ = main_window.update(cx, |_, window, cx| {
                        let _ = owner.update(cx, |view, cx| {
                            if !view.windows_command_enabled(command) {
                                return;
                            }
                            match command {
                                Command::Add => view.open_add(window, cx),
                                Command::Torrent => view.choose_torrent_files(window, cx),
                                Command::Details => view.open_selected_details(window, cx),
                                Command::Reveal => view.reveal_selected(cx),
                                Command::StopSelected => view.stop_selected(cx),
                                Command::Remove => view.remove_selected(window, cx),
                                Command::Clear => view.clear_finished(window, cx),
                                Command::Options => view.open_options(window, cx),
                                Command::ToggleSidebar => view.toggle_sidebar(cx),
                                Command::Filter(filter) => view.set_filter(filter, cx),
                                Command::About => view.about(window, cx),
                                Command::Updates => view.check_updates(window, cx, true),
                            }
                        });
                    });
                });
            })
    }

    pub(super) fn render_windows_command_toolbar(&self, cx: &mut Context<Self>) -> Toolbar {
        if self.windows_command_tab == CommandTab::Tasks {
            return self.render_queue_toolbar(cx);
        }
        let commands: &[(&str, &str, Command)] = match self.windows_command_tab {
            CommandTab::File => &[
                ("windows-add", "Add URL…", Command::Add),
                ("windows-torrent", "Open Torrent…", Command::Torrent),
                ("windows-details", "Details", Command::Details),
                ("windows-reveal", "Show in folder", Command::Reveal),
            ],
            CommandTab::Downloads => &[
                (
                    "windows-stop-selected",
                    "Stop selected",
                    Command::StopSelected,
                ),
                ("windows-remove", "Remove records", Command::Remove),
                ("windows-clear", "Clear finished", Command::Clear),
                ("windows-options", "Options", Command::Options),
            ],
            CommandTab::View => &[
                ("windows-sidebar", "Toggle sidebar", Command::ToggleSidebar),
                ("windows-all", "All downloads", Command::Filter(Filter::All)),
                (
                    "windows-finished",
                    "Finished",
                    Command::Filter(Filter::Finished),
                ),
                (
                    "windows-unfinished",
                    "Unfinished",
                    Command::Filter(Filter::Unfinished),
                ),
                ("windows-queued", "Queue", Command::Filter(Filter::Queued)),
            ],
            CommandTab::Help => &[
                ("windows-about", "About Ulos", Command::About),
                ("windows-updates", "Check for Updates…", Command::Updates),
            ],
            CommandTab::Tasks => unreachable!(),
        };
        let mut toolbar = Toolbar::new("windows-command-toolbar")
            .h_auto()
            .px_2()
            .py_2()
            .flex_shrink_0();
        for &(id, label, command) in commands {
            toolbar = toolbar.content(self.windows_command_button(id, label, command, cx));
        }
        if self.windows_command_tab == CommandTab::View {
            toolbar = toolbar.content(self.theme_selector("windows-theme", cx));
        }
        toolbar
            .content(div().flex_1().min_w(px(8.)))
            .content(self.render_toolbar_search())
    }
}

#[cfg(test)]
mod tests {
    use super::{Command, CommandTab, DownloadManager};
    use crate::queue::Filter;
    use gpui_kit::{
        AppContext, Context, Entity, IntoElement, Modifiers, ParentElement, Render, Styled,
        TestAppContext, VisualTestContext, Window, component::Root, div, px, size,
    };

    struct HeaderHarness(Entity<DownloadManager>);
    impl Render for HeaderHarness {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            self.0.update(cx, |view, cx| {
                div()
                    .size_full()
                    .flex()
                    .flex_col()
                    .child(view.render_windows_header(cx))
                    .child(view.render_windows_command_toolbar(cx))
            })
        }
    }

    fn draw(cx: &mut VisualTestContext) {
        cx.run_until_parked();
        cx.update(|window, cx| window.draw(cx).clear(cx));
    }
    fn click(cx: &mut VisualTestContext, selector: &'static str) {
        let bounds = cx
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("Missing {selector}"));
        cx.simulate_click(bounds.center(), Modifiers::default());
        draw(cx);
    }
    fn mount<'a>(
        cx: &'a mut TestAppContext,
        runtime: &tokio::runtime::Runtime,
    ) -> (Entity<DownloadManager>, &'a mut VisualTestContext) {
        cx.update(gpui_kit::init);
        cx.update(crate::palette::init);
        let mut manager = None;
        let handle = runtime.handle().clone();
        let (_, cx) = cx.add_window_view(|window, cx| {
            let view = cx.new(|cx| DownloadManager::new(handle, window, cx));
            manager = Some(view.clone());
            let harness = cx.new(|cx| {
                cx.observe(&view, |_, _, cx| cx.notify()).detach();
                HeaderHarness(view)
            });
            Root::new(harness, window, cx)
        });
        let view = manager.unwrap();
        cx.simulate_resize(size(px(880.), px(540.)));
        draw(cx);
        (view, cx)
    }

    #[gpui::test]
    fn windows_header_tabs_keep_stock_controls_and_search_at_minimum_width(
        cx: &mut TestAppContext,
    ) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let (view, cx) = mount(cx, &runtime);
        cx.update(|window, cx| {
            let input = view.read(cx).search.clone();
            input.update(cx, |input, cx| input.focus(window, cx));
        });
        cx.simulate_input("needle");
        draw(cx);
        for (tab, controls) in [
            (
                CommandTab::Tasks,
                vec!["add-url", "options", "selected-details"],
            ),
            (
                CommandTab::File,
                vec![
                    "windows-add",
                    "windows-torrent",
                    "windows-details",
                    "windows-reveal",
                ],
            ),
            (
                CommandTab::Downloads,
                vec![
                    "windows-stop-selected",
                    "windows-remove",
                    "windows-clear",
                    "windows-options",
                ],
            ),
            (
                CommandTab::View,
                vec![
                    "windows-sidebar",
                    "windows-all",
                    "windows-finished",
                    "windows-unfinished",
                    "windows-queued",
                    "windows-theme",
                ],
            ),
            (CommandTab::Help, vec!["windows-about", "windows-updates"]),
        ] {
            click(cx, tab.selector());
            view.read_with(cx, |view, cx| {
                assert_eq!(view.windows_command_tab, tab);
                assert_eq!(view.search.read(cx).value().as_str(), "needle");
                assert_eq!(view.queue.search, "needle");
            });
            let search = cx
                .debug_bounds("download-search")
                .expect("Search on every tab");
            assert!(search.size.width >= px(160.));
            assert!(search.right() <= px(880.));
            for control in controls {
                let bounds = cx
                    .debug_bounds(control)
                    .unwrap_or_else(|| panic!("Missing {control}"));
                assert!(
                    bounds.size.height > px(24.),
                    "Stock button height: {control}"
                );
                assert!(bounds.size.width > px(0.), "No empty controls: {control}");
                assert!(
                    (bounds.center().y - search.center().y).abs() <= px(1.),
                    "Single command row: {control}"
                );
                assert!(
                    bounds.right() <= search.left(),
                    "No search overlap: {control}"
                );
            }
        }
    }

    #[gpui::test]
    fn windows_header_switching_exposes_real_actions_and_disabled_commands(
        cx: &mut TestAppContext,
    ) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let (view, cx) = mount(cx, &runtime);
        click(cx, "windows-tab-View");
        click(cx, "windows-sidebar");
        view.read_with(cx, |view, _| assert!(view.sidebar_collapsed));
        click(cx, "windows-finished");
        view.read_with(cx, |view, _| {
            assert_eq!(view.queue.filter, Filter::Finished)
        });
        click(cx, "windows-tab-Downloads");
        view.read_with(cx, |view, _| {
            for command in [
                Command::StopSelected,
                Command::Remove,
                Command::Clear,
                Command::Details,
                Command::Reveal,
            ] {
                assert!(!view.windows_command_enabled(command));
            }
        });
        click(cx, "windows-remove");
        view.read_with(cx, |view, _| assert!(!view.removal_open));
        click(cx, "windows-tab-File");
        click(cx, "windows-add");
        view.read_with(cx, |view, _| assert_eq!(view.add_windows.len(), 1));
        click(cx, "windows-tab-Help");
        click(cx, "windows-about");
        view.read_with(cx, |view, _| assert!(view.about_open));
    }
}
