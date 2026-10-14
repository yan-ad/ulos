//! Stateless presentation of the bounded, session-only download queue.
use super::*;
use crate::queue::{Category, Filter};
use gpui_kit::component::Sizable as _;
use gpui_kit::{
    assets::IconName as AssetIcon,
    component::{
        Icon,
        button::Button,
        empty::{Empty, EmptyContent, EmptyDescription, EmptyHeader, EmptyMedia, EmptyTitle},
        input::Input,
        label::Label,
        sidebar::{Sidebar, SidebarCollapsible, SidebarGroup, SidebarMenu, SidebarMenuItem},
        status_bar::StatusBar,
        table::DataTable,
        toolbar::Toolbar,
    },
};

#[cfg(not(target_os = "macos"))]
use gpui_kit::component::{
    ThemeMode,
    menu::{DropdownMenu, PopupMenuItem},
};

fn category_icon(category: Category) -> AssetIcon {
    match category {
        Category::Image => AssetIcon::Image,
        Category::Music => AssetIcon::Music,
        Category::Video => AssetIcon::Film,
        Category::Apps => AssetIcon::Package,
        Category::Document => AssetIcon::FileText,
        Category::Compressed => AssetIcon::Archive,
        Category::Other => AssetIcon::File,
    }
}

fn toolbar_button(id: &'static str, label: &'static str, icon: AssetIcon) -> Button {
    Button::new(id)
        .accessibility_label(label)
        .debug_selector(move || id.into())
        .icon(icon)
        .tooltip(label)
}

#[cfg(not(target_os = "macos"))]
#[derive(Clone, Copy)]
enum MenuCommand {
    Add,
    Start,
    StopQueue,
    StopAll,
    StopSelected,
    Remove,
    Clear,
    Options,
    Details,
    Reveal,
    ToggleSidebar,
    Filter(Filter),
}

#[cfg(not(target_os = "macos"))]
fn manager_menu_item(
    label: &'static str,
    manager: WeakEntity<DownloadManager>,
    command: MenuCommand,
) -> PopupMenuItem {
    PopupMenuItem::new(label).on_click(move |_, window, cx| {
        let _ = manager.update(cx, |view, cx| match command {
            MenuCommand::Add => view.open_add(window, cx),
            MenuCommand::Start => view.start_queue(cx),
            MenuCommand::StopQueue => view.stop_queue(cx),
            MenuCommand::StopAll => view.stop_all(cx),
            MenuCommand::StopSelected => view.stop_selected(cx),
            MenuCommand::Remove => view.remove_selected(cx),
            MenuCommand::Clear => view.clear_finished(cx),
            MenuCommand::Options => view.open_options(window, cx),
            MenuCommand::Details => view.open_selected_details(window, cx),
            MenuCommand::Reveal => view.reveal_selected(cx),
            MenuCommand::ToggleSidebar => view.toggle_sidebar(cx),
            MenuCommand::Filter(filter) => view.set_filter(filter, cx),
        });
    })
}

impl DownloadManager {
    fn sidebar_filter(
        &self,
        filter: Filter,
        label: &'static str,
        icon: AssetIcon,
        count: usize,
        cx: &mut Context<Self>,
    ) -> SidebarMenuItem {
        SidebarMenuItem::new(label)
            .icon(icon)
            .active(self.queue.filter == filter)
            .accessibility_label(format!("{label}, {count} downloads"))
            .suffix(move |_, _| Label::new(count.to_string()))
            .on_click(cx.listener(move |view, _, _, cx| view.set_filter(filter, cx)))
    }

    fn render_queue_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let finished = self
            .queue
            .items
            .iter()
            .filter(|item| item.is_finished())
            .count();
        let categories = Category::ALL
            .into_iter()
            .map(|category| {
                let count = self
                    .queue
                    .items
                    .iter()
                    .filter(|item| item.category == category)
                    .count();
                self.sidebar_filter(
                    Filter::Category(category),
                    category.label(),
                    category_icon(category),
                    count,
                    cx,
                )
            })
            .collect::<Vec<_>>();
        let downloads = SidebarMenu::new()
            .child(self.sidebar_filter(
                Filter::All,
                "All downloads",
                AssetIcon::Download,
                self.queue.items.len(),
                cx,
            ))
            .child(self.sidebar_filter(
                Filter::Finished,
                "Finished",
                AssetIcon::Check,
                finished,
                cx,
            ))
            .child(self.sidebar_filter(
                Filter::Unfinished,
                "Unfinished",
                AssetIcon::Clock,
                self.queue.items.len() - finished,
                cx,
            ))
            .child(
                self.sidebar_filter(
                    Filter::Queued,
                    "Queue",
                    AssetIcon::List,
                    self.queue
                        .items
                        .iter()
                        .filter(|item| item.is_queued())
                        .count(),
                    cx,
                ),
            );
        let sidebar = Sidebar::new("download-sidebar")
            .collapsible(SidebarCollapsible::Icon)
            .collapsed(self.sidebar_collapsed)
            .child(SidebarGroup::new("Downloads").child(downloads))
            .child(SidebarGroup::new("File types").child(SidebarMenu::new().children(categories)));
        div()
            .debug_selector(|| "manager-sidebar".into())
            .h_full()
            .flex_shrink_0()
            .child(sidebar)
    }

    fn render_queue_toolbar(&self, cx: &mut Context<Self>) -> Toolbar {
        let selected_active = self
            .queue
            .items
            .iter()
            .any(|item| self.queue.selected.contains(&item.id) && item.is_active());
        let removable = self
            .queue
            .items
            .iter()
            .any(|item| self.queue.selected.contains(&item.id) && !item.is_active());
        let finished = self.queue.items.iter().any(|item| item.is_finished());
        // `content` preserves stock controls: Toolbar::child applies its own
        // small size and prepares buttons as ghost/compact variants.
        Toolbar::new("download-toolbar")
            .h_auto()
            .px_2()
            .py_2()
            .flex_shrink_0()
            .content(
                toolbar_button(
                    "toggle-sidebar",
                    if self.sidebar_collapsed {
                        "Expand sidebar"
                    } else {
                        "Collapse sidebar"
                    },
                    if self.sidebar_collapsed {
                        AssetIcon::PanelLeftOpen
                    } else {
                        AssetIcon::PanelLeftClose
                    },
                )
                .on_click(cx.listener(|view, _, _, cx| view.toggle_sidebar(cx))),
            )
            .content(
                toolbar_button("add-url", "Add URL", AssetIcon::Plus)
                    .label("Add URL")
                    .on_click(cx.listener(|view, _, window, cx| view.open_add(window, cx))),
            )
            .content(
                toolbar_button("start-queue", "Start Queue", AssetIcon::Play)
                    .disabled(self.queue_running || self.queue.next_queued().is_none())
                    .on_click(cx.listener(|view, _, _, cx| view.start_queue(cx))),
            )
            .content(
                toolbar_button("stop-selected", "Stop selected", AssetIcon::Square)
                    .disabled(!selected_active)
                    .on_click(cx.listener(|view, _, _, cx| view.stop_selected(cx))),
            )
            .content(
                toolbar_button("stop-all", "Stop All", AssetIcon::CircleX)
                    .disabled(self.queue.active_count() == 0)
                    .on_click(cx.listener(|view, _, _, cx| view.stop_all(cx))),
            )
            .content(
                toolbar_button("remove-selected", "Remove", AssetIcon::Trash)
                    .disabled(!removable)
                    .tooltip("Remove selected records from this session; files are not deleted")
                    .on_click(cx.listener(|view, _, _, cx| view.remove_selected(cx))),
            )
            .content(
                toolbar_button("clear-finished", "Clear finished", AssetIcon::Check)
                    .disabled(!finished)
                    .tooltip("Clear finished session records; downloaded files are kept")
                    .on_click(cx.listener(|view, _, _, cx| view.clear_finished(cx))),
            )
            .content(
                toolbar_button("scheduler", "Scheduler", AssetIcon::Calendar)
                    .disabled(true)
                    .tooltip("Scheduling is not implemented yet"),
            )
            .content(
                toolbar_button("grabber", "Grabber", AssetIcon::Globe)
                    .disabled(true)
                    .tooltip("Site grabbing is not implemented yet"),
            )
            .content(
                toolbar_button("options", "Options", AssetIcon::Settings)
                    .on_click(cx.listener(|view, _, window, cx| view.open_options(window, cx))),
            )
            .content(
                toolbar_button("selected-details", "Details", AssetIcon::Info)
                    .disabled(self.queue.selected.is_empty())
                    .on_click(
                        cx.listener(|view, _, window, cx| view.open_selected_details(window, cx)),
                    ),
            )
            .content(div().flex_1().min_w(px(8.)))
            .content(
                div()
                    .id("download-search")
                    .debug_selector(|| "download-search".into())
                    .w(px(240.))
                    .min_w(px(160.))
                    .flex_shrink_1()
                    .child(Input::new(&self.search).prefix(Icon::new(AssetIcon::Search))),
            )
    }

    #[cfg(not(target_os = "macos"))]
    fn render_menu_strip(&self, cx: &mut Context<Self>) -> impl IntoElement {
        // AppMenuBar reads global native menus and needs a persistent Entity. These
        // Kit dropdowns keep the manager-local commands scoped to this window.
        let entity = cx.entity().downgrade();
        let tasks = entity.clone();
        let files = entity.clone();
        let downloads = entity.clone();
        let view = entity;
        let running = self.queue_running;
        let can_start = !running && self.queue.next_queued().is_some();
        let can_stop = self.queue.active_count() > 0;
        let selected = !self.queue.selected.is_empty();
        let selected_active = self
            .queue
            .items
            .iter()
            .any(|item| self.queue.selected.contains(&item.id) && item.is_active());
        let removable = self
            .queue
            .items
            .iter()
            .any(|item| self.queue.selected.contains(&item.id) && !item.is_active());
        let finished = self.queue.items.iter().any(|item| item.is_finished());
        let revealable = self
            .queue
            .items
            .iter()
            .any(|item| self.queue.selected.contains(&item.id) && item.is_finished());
        let dark = cx.theme().is_dark();
        div()
            .id("manager-menubar")
            .debug_selector(|| "manager-menubar".into())
            .flex()
            .items_center()
            .h(px(36.))
            .px_2()
            .gap_1()
            .flex_shrink_0()
            .child(
                Button::new("menu-tasks")
                    .label("Tasks")
                    .dropdown_menu(move |menu, _, _| {
                        menu.item(manager_menu_item(
                            "Toggle sidebar",
                            view.clone(),
                            MenuCommand::ToggleSidebar,
                        ))
                        .separator()
                        .item(manager_menu_item(
                            "Add URL…",
                            tasks.clone(),
                            MenuCommand::Add,
                        ))
                        .item(
                            manager_menu_item("Start Queue", tasks.clone(), MenuCommand::Start)
                                .disabled(!can_start),
                        )
                        .item(
                            manager_menu_item("Stop Queue", tasks.clone(), MenuCommand::StopQueue)
                                .disabled(!running),
                        )
                        .item(
                            manager_menu_item("Stop All", tasks.clone(), MenuCommand::StopAll)
                                .disabled(!can_stop),
                        )
                    }),
            )
            .child(
                Button::new("menu-file")
                    .label("File")
                    .dropdown_menu(move |menu, _, _| {
                        menu.item(manager_menu_item(
                            "Add URL…",
                            files.clone(),
                            MenuCommand::Add,
                        ))
                        .item(
                            manager_menu_item("Details…", files.clone(), MenuCommand::Details)
                                .disabled(!selected),
                        )
                        .item(
                            manager_menu_item("Show in folder", files.clone(), MenuCommand::Reveal)
                                .disabled(!revealable),
                        )
                    }),
            )
            .child(
                Button::new("menu-downloads")
                    .label("Downloads")
                    .dropdown_menu(move |menu, _, _| {
                        menu.item(
                            manager_menu_item(
                                "Stop selected",
                                downloads.clone(),
                                MenuCommand::StopSelected,
                            )
                            .disabled(!selected_active),
                        )
                        .item(
                            manager_menu_item(
                                "Remove selected records",
                                downloads.clone(),
                                MenuCommand::Remove,
                            )
                            .disabled(!removable),
                        )
                        .item(
                            manager_menu_item(
                                "Clear finished records",
                                downloads.clone(),
                                MenuCommand::Clear,
                            )
                            .disabled(!finished),
                        )
                        .separator()
                        .item(manager_menu_item(
                            "Options…",
                            downloads.clone(),
                            MenuCommand::Options,
                        ))
                    }),
            )
            .child(
                Button::new("menu-view")
                    .label("View")
                    .dropdown_menu(move |menu, _, _| {
                        menu.item(manager_menu_item(
                            "All downloads",
                            view.clone(),
                            MenuCommand::Filter(Filter::All),
                        ))
                        .item(manager_menu_item(
                            "Finished",
                            view.clone(),
                            MenuCommand::Filter(Filter::Finished),
                        ))
                        .item(manager_menu_item(
                            "Unfinished",
                            view.clone(),
                            MenuCommand::Filter(Filter::Unfinished),
                        ))
                        .item(manager_menu_item(
                            "Queue",
                            view.clone(),
                            MenuCommand::Filter(Filter::Queued),
                        ))
                        .separator()
                        .item(
                            PopupMenuItem::new(if dark { "Light theme" } else { "Dark theme" })
                                .on_click(|_, _, cx| {
                                    crate::palette::apply(
                                        if cx.theme().is_dark() {
                                            ThemeMode::Light
                                        } else {
                                            ThemeMode::Dark
                                        },
                                        cx,
                                    );
                                }),
                        )
                    }),
            )
            .child(
                Button::new("menu-help")
                    .label("Help")
                    .dropdown_menu(|menu, _, _| {
                        menu.label("Add URL: choose a destination to download")
                            .label("Click a filename to inspect transfer details")
                            .separator()
                            .label("Session-only queue; no persistent history")
                            .label("Remove / clear records never deletes files")
                    }),
            )
    }

    fn render_queue_table(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let empty = self.table.read(cx).delegate().rows.is_empty();
        div()
            .id("download-table")
            .debug_selector(|| "download-table".into())
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .when(!empty, |this| {
                this.child(
                    DataTable::new(&self.table)
                        .with_size(gpui_kit::component::Size::Size(px(48.)))
                        .bordered(true),
                )
            })
            .when(empty, |this| {
                let no_downloads = self.queue.items.is_empty();
                this.child(
                    Empty::new()
                        .header(
                            EmptyHeader::new()
                                .media(EmptyMedia::new().child(Icon::new(AssetIcon::Download)))
                                .title(EmptyTitle::new().child(Label::new(if no_downloads {
                                    "No downloads yet"
                                } else {
                                    "No matching downloads"
                                })))
                                .description(EmptyDescription::new().child(Label::new(
                                    if no_downloads {
                                        "Add a URL to choose a destination and start downloading."
                                    } else {
                                        "Change the category or search, or add a new URL."
                                    },
                                ))),
                        )
                        .content(
                            EmptyContent::new().child(
                                Button::new("empty-add-url")
                                    .icon(AssetIcon::Plus)
                                    .label("Add URL")
                                    .on_click(
                                        cx.listener(|view, _, window, cx| {
                                            view.open_add(window, cx)
                                        }),
                                    ),
                            ),
                        ),
                )
            })
    }
}

impl Render for DownloadManager {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        #[cfg(test)]
        self.render_count.set(self.render_count.get() + 1);
        let count = self.queue.items.len();
        let selected_count = self.queue.selected.len();
        let active_count = self.queue.active_count();
        let rate = self
            .queue
            .items
            .iter()
            .any(|item| item.speed_bytes_per_second().is_some())
            .then(|| self.queue.total_speed());

        #[cfg(not(target_os = "macos"))]
        let menubar = Some(self.render_menu_strip(cx).into_any_element());
        #[cfg(target_os = "macos")]
        let menubar: Option<AnyElement> = None;
        #[cfg(target_os = "macos")]
        let toolbar = gpui_kit::component::TitleBar::new()
            .h(px(48.))
            .bg(cx.theme().background)
            .child(self.render_queue_toolbar(cx).flex_1().min_w_0())
            .into_any_element();
        #[cfg(not(target_os = "macos"))]
        let toolbar = self.render_queue_toolbar(cx).into_any_element();

        div()
            .id("manager-shell")
            .debug_selector(|| "manager-shell".into())
            .track_focus(&self.focus)
            .key_context("DownloadManager")
            .on_action(cx.listener(|view, _: &AddUrl, window, cx| view.open_add(window, cx)))
            .on_action(cx.listener(|view, _: &StartQueue, _, cx| view.start_queue(cx)))
            .on_action(cx.listener(|view, _: &StopAll, _, cx| view.stop_all(cx)))
            .on_action(cx.listener(|view, _: &ToggleSidebar, _, cx| view.toggle_sidebar(cx)))
            .size_full()
            .flex()
            .flex_col()
            .overflow_hidden()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .when_some(menubar, |this, menubar| this.child(menubar))
            .child(toolbar)
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .child(self.render_queue_sidebar(cx))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w_0()
                            .min_h_0()
                            .when_some(self.notice.clone(), |this, notice| {
                                this.child(Label::new(notice).px_4().py_2().text_xs())
                            })
                            .child(self.render_queue_table(cx)),
                    ),
            )
            .child(
                StatusBar::new()
                    .flex_shrink_0()
                    .left(Label::new(format!(
                        "{count} downloads · {selected_count} selected"
                    )))
                    .child(Label::new(format!(
                        "Queue {} · Session only",
                        if self.queue_running {
                            "Running"
                        } else {
                            "stopped"
                        }
                    )))
                    .right(Label::new(if active_count == 0 {
                        "0 active".to_owned()
                    } else if let Some(rate) = rate {
                        format!(
                            "{active_count} active · {}/s",
                            formatting::bytes(rate as u64)
                        )
                    } else {
                        format!("{active_count} active")
                    })),
            )
    }
}
