//! Stateless presentation of the bounded, session-only download queue.
use super::*;
use crate::queue::{Category, Filter, SortColumn};
use gpui_kit::{
    assets::IconName as AssetIcon,
    component::{
        Icon, ThemeMode,
        button::{Button, ButtonVariants},
        checkbox::Checkbox,
        input::Input,
        menu::{DropdownMenu, PopupMenuItem},
        sidebar::{Sidebar, SidebarMenu, SidebarMenuItem},
        table::{Table, TableBody, TableCell, TableHead, TableHeader, TableRow},
    },
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

// Override the primitive's default 100px minimum, notably for checkboxes.
fn head(width: f32) -> TableHead {
    TableHead::new()
        .py_0()
        .w(px(width))
        .min_w(px(width))
        .flex_shrink_0()
}
fn cell(width: f32) -> TableCell {
    TableCell::new()
        .py_0()
        .w(px(width))
        .min_w(px(width))
        .flex_shrink_0()
}

fn toolbar_button(id: &'static str, label: &'static str, icon: AssetIcon) -> Button {
    Button::new(id)
        .accessibility_label(label)
        .debug_selector(move || id.into())
        .ghost()
        .h(px(64.))
        .min_w(px(76.))
        .px_2()
        .flex_shrink_0()
        .child(
            div()
                .flex()
                .flex_col()
                .items_center()
                .gap_1()
                .child(Icon::new(icon).size_6())
                .child(div().text_xs().child(label)),
        )
}

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
    Filter(Filter),
}

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
            .icon(Icon::new(icon).size_4())
            .active(self.queue.filter == filter)
            .suffix(move |_, cx| {
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(count.to_string())
            })
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
        let all = self
            .sidebar_filter(
                Filter::All,
                "All downloads",
                AssetIcon::Download,
                self.queue.items.len(),
                cx,
            )
            .default_open(true)
            .children(categories);
        Sidebar::new("download-sidebar")
            .w(px(210.))
            .header(
                div()
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Categories"),
            )
            .child(
                SidebarMenu::new()
                    .child(all)
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
                    ),
            )
    }

    fn render_queue_toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
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
        div()
            .id("download-toolbar")
            .debug_selector(|| "download-toolbar".into())
            .flex()
            .items_center()
            .gap_1()
            .h(px(74.))
            .px_2()
            .flex_shrink_0()
            .overflow_x_scroll()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(
                toolbar_button("add-url", "Add URL", AssetIcon::Plus)
                    .primary()
                    .on_click(cx.listener(|view, _, window, cx| view.open_add(window, cx))),
            )
            .child(
                toolbar_button("start-queue", "Start Queue", AssetIcon::Play)
                    .disabled(self.queue_running || self.queue.next_queued().is_none())
                    .on_click(cx.listener(|view, _, _, cx| view.start_queue(cx))),
            )
            .child(
                toolbar_button("stop-selected", "Stop selected", AssetIcon::Square)
                    .disabled(!selected_active)
                    .on_click(cx.listener(|view, _, _, cx| view.stop_selected(cx))),
            )
            .child(
                toolbar_button("stop-all", "Stop All", AssetIcon::CircleX)
                    .disabled(self.queue.active_count() == 0)
                    .on_click(cx.listener(|view, _, _, cx| view.stop_all(cx))),
            )
            .child(
                toolbar_button("remove-selected", "Remove", AssetIcon::Trash)
                    .disabled(!removable)
                    .tooltip("Remove selected records from this session; files are not deleted")
                    .on_click(cx.listener(|view, _, _, cx| view.remove_selected(cx))),
            )
            .child(
                toolbar_button("clear-finished", "Clear finished", AssetIcon::Check)
                    .disabled(!finished)
                    .tooltip("Clear finished session records; downloaded files are kept")
                    .on_click(cx.listener(|view, _, _, cx| view.clear_finished(cx))),
            )
            .child(
                toolbar_button("scheduler", "Scheduler", AssetIcon::Calendar)
                    .disabled(true)
                    .tooltip("Scheduling is not implemented yet"),
            )
            .child(
                toolbar_button("grabber", "Grabber", AssetIcon::Globe)
                    .disabled(true)
                    .tooltip("Site grabbing is not implemented yet"),
            )
            .child(
                toolbar_button("options", "Options", AssetIcon::Settings)
                    .on_click(cx.listener(|view, _, window, cx| view.open_options(window, cx))),
            )
            .child(
                toolbar_button("selected-details", "Details", AssetIcon::Info)
                    .disabled(self.queue.selected.is_empty())
                    .on_click(
                        cx.listener(|view, _, window, cx| view.open_selected_details(window, cx)),
                    ),
            )
    }

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
            .border_b_1()
            .border_color(cx.theme().border)
            .child(
                Button::new("menu-tasks")
                    .small()
                    .ghost()
                    .label("Tasks")
                    .dropdown_menu(move |menu, _, _| {
                        menu.item(manager_menu_item(
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
                    .small()
                    .ghost()
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
                    .small()
                    .ghost()
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
                    .small()
                    .ghost()
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
                    .small()
                    .ghost()
                    .label("Help")
                    .dropdown_menu(|menu, _, _| {
                        menu.label("Add URL: choose a destination to download")
                            .label("Click a filename to inspect transfer details")
                            .separator()
                            .label("Session-only queue; no persistent history")
                            .label("Remove / clear records never deletes files")
                    }),
            )
            .child(div().flex_1())
            .child(
                Input::new(&self.search)
                    .small()
                    .w(px(240.))
                    .prefix(Icon::new(AssetIcon::Search).size_4()),
            )
    }

    fn render_sort_button(
        &self,
        label: &'static str,
        column: SortColumn,
        cx: &mut Context<Self>,
    ) -> Button {
        let label = if self.queue.sort == column {
            format!("{label} {}", if self.queue.descending { "↓" } else { "↑" })
        } else {
            label.to_owned()
        };
        Button::new(("sort-column", column as usize))
            .small()
            .ghost()
            .label(label)
            .on_click(cx.listener(move |view, _, _, cx| {
                view.queue.sort_by(column);
                cx.notify();
            }))
    }

    fn render_queue_table(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let visible = self.queue.visible();
        let empty = visible.is_empty();
        let header = TableHeader::new().child(
            TableRow::new()
                .h(px(32.))
                .child(
                    head(40.).px_2().child(
                        Checkbox::new("select-visible")
                            .accessibility_label("Select all visible downloads")
                            .checked(self.queue.all_visible_selected())
                            .disabled(empty)
                            .on_change(cx.listener(|view, checked, _, cx| {
                                view.queue.select_visible(*checked);
                                cx.notify();
                            })),
                    ),
                )
                .child(
                    TableHead::new()
                        .py_0()
                        .flex_1()
                        .min_w(px(260.))
                        .child(self.render_sort_button("Name", SortColumn::Name, cx)),
                )
                .child(head(110.).child(self.render_sort_button("Size", SortColumn::Size, cx)))
                .child(head(160.).child("Status"))
                .child(head(130.).child("Transfer rate")),
        );
        let rows = visible
            .into_iter()
            .map(|item| {
                let id = item.id;
                let selected = self.queue.selected.contains(&id);
                let status = item.status().to_owned();
                TableRow::new()
                    .h(px(32.))
                    .when(selected, |this| {
                        this.bg(cx.theme().table_active)
                            .border_l_2()
                            .border_color(cx.theme().table_active_border)
                    })
                    .child(
                        cell(40.).px_2().child(
                            Checkbox::new(("select-download", id))
                                .accessibility_label(format!("Select {}", item.name()))
                                .checked(selected)
                                .on_change(cx.listener(move |view, checked, _, cx| {
                                    view.queue.select(id, *checked);
                                    cx.notify();
                                })),
                        ),
                    )
                    .child(
                        TableCell::new().py_0().flex_1().min_w(px(260.)).child(
                            Button::new(("download-name", id))
                                .accessibility_label(format!("Open details for {}", item.name()))
                                .small()
                                .ghost()
                                .px_0()
                                .h(px(28.))
                                .tooltip(item.destination.display().to_string())
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap_2()
                                        .child(
                                            Icon::new(category_icon(item.category))
                                                .size_4()
                                                .text_color(cx.theme().muted_foreground),
                                        )
                                        .whitespace_nowrap()
                                        .child(item.name().to_owned()),
                                )
                                .on_click(cx.listener(move |view, _, window, cx| {
                                    view.open_details(id, window, cx)
                                })),
                        ),
                    )
                    .child(
                        cell(110.).child(
                            item.total_bytes
                                .map(formatting::bytes)
                                .unwrap_or_else(|| "—".into()),
                        ),
                    )
                    .child(cell(160.).child(status))
                    .child(
                        cell(130.).child(
                            item.speed_bytes_per_second()
                                .map(|rate| format!("{}/s", formatting::bytes(rate as u64)))
                                .unwrap_or_else(|| "—".into()),
                        ),
                    )
            })
            .collect::<Vec<_>>();
        div().id("download-table").debug_selector(|| "download-table".into())
            .flex_1().min_h_0().min_w_0().overflow_scroll()
            .child(div().min_w(px(700.)).min_h_full().flex().flex_col()
                .child(Table::new().small().accessibility_label("Session downloads")
                    .child(header).child(TableBody::new().children(rows)))
                .when(empty, |this| this.child(div().id("empty-downloads")
                    .debug_selector(|| "empty-downloads".into())
                    .flex_1().min_h(px(260.)).flex().flex_col().items_center().justify_center().gap_3().py_8()
                    .child(Icon::new(AssetIcon::Download).size_8().text_color(cx.theme().muted_foreground))
                    .child(div().text_lg().child(if self.queue.items.is_empty() { "No downloads yet" } else { "No matching downloads" }))
                    .child(div().text_sm().text_color(cx.theme().muted_foreground).child(
                        if self.queue.items.is_empty() { "Add a URL to choose a destination and start downloading." }
                        else { "Change the category or search, or add a new URL." }))
                    .child(Button::new("empty-add-url").small().primary().icon(AssetIcon::Plus).label("Add URL")
                        .on_click(cx.listener(|view, _, window, cx| view.open_add(window, cx)))))))
    }
}

impl Render for DownloadManager {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let count = self.queue.items.len();
        let selected_count = self.queue.selected.len();
        let active_count = self.queue.active_count();
        let rate = self
            .queue
            .items
            .iter()
            .any(|item| item.speed_bytes_per_second().is_some())
            .then(|| self.queue.total_speed());

        div()
            .id("manager-shell")
            .debug_selector(|| "manager-shell".into())
            .track_focus(&self.focus)
            .key_context("DownloadManager")
            .on_action(cx.listener(|view, _: &AddUrl, window, cx| view.open_add(window, cx)))
            .on_action(cx.listener(|view, _: &StartQueue, _, cx| view.start_queue(cx)))
            .on_action(cx.listener(|view, _: &StopAll, _, cx| view.stop_all(cx)))
            .size_full()
            .flex()
            .flex_col()
            .overflow_hidden()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(self.render_menu_strip(cx))
            .child(self.render_queue_toolbar(cx))
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
                                this.child(
                                    div()
                                        .px_4()
                                        .py_2()
                                        .text_xs()
                                        .border_b_1()
                                        .border_color(cx.theme().border)
                                        .text_color(cx.theme().muted_foreground)
                                        .child(notice),
                                )
                            })
                            .child(self.render_queue_table(cx)),
                    ),
            )
            .child(
                div()
                    .h(px(26.))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_3()
                    .px_4()
                    .bg(cx.theme().status_bar)
                    .border_t_1()
                    .border_color(cx.theme().status_bar_border)
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(format!("{count} downloads · {selected_count} selected"))
                    .child(format!(
                        "Queue {} · Session only",
                        if self.queue_running {
                            "Running"
                        } else {
                            "stopped"
                        }
                    ))
                    .child(if active_count == 0 {
                        "0 active".to_owned()
                    } else if let Some(rate) = rate {
                        format!(
                            "{active_count} active · {}/s",
                            formatting::bytes(rate as u64)
                        )
                    } else {
                        format!("{active_count} active")
                    }),
            )
    }
}
