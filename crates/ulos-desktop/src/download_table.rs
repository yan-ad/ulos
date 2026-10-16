//! Snapshot-backed Kit table. Only viewport cells instantiate visual controls.
use std::time::{Duration, SystemTime};

use chrono::{DateTime, Local};

use gpui_kit::{
    component::{
        Disableable, Icon,
        checkbox::Checkbox,
        h_flex,
        label::Label,
        link::Link,
        menu::PopupMenu,
        table::{Column, ColumnSort, TableDelegate, TableState},
    },
    *,
};

use crate::{
    download_details::{status_color, status_icon},
    formatting,
    manager::DownloadManager,
    queue::{Category, DownloadId, SortColumn},
};

/// Presentation data, refreshed by the manager when queue state changes.
#[derive(Clone, Debug)]
pub struct DownloadRow {
    pub is_torrent: bool,
    pub id: DownloadId,
    pub name: String,
    pub category: Category,
    pub size: Option<u64>,
    pub status: String,
    pub progress: Option<ulos_core::DownloadProgress>,
    pub rate: Option<f64>,
    pub selected: bool,
    pub queue_name: String,
    pub time_left: Option<Duration>,
    pub last_try: Option<SystemTime>,
    pub description: String,
}

pub struct DownloadTable {
    pub rows: Vec<DownloadRow>,
    pub owner: WeakEntity<DownloadManager>,
    pub columns: Vec<Column>,
    pub header_all_selected: bool,
    #[cfg(test)]
    pub rendered_cells: std::rc::Rc<std::cell::Cell<usize>>,
}

impl DownloadTable {
    pub fn new(owner: WeakEntity<DownloadManager>) -> Self {
        Self {
            rows: Vec::new(),
            owner,
            columns: columns(),
            header_all_selected: false,
            #[cfg(test)]
            rendered_cells: Default::default(),
        }
    }
}

/// Preserve the original sortable column indexes when adding snapshot columns.
fn columns() -> Vec<Column> {
    vec![
        Column::new("selected", "")
            .width(px(40.))
            .resizable(false)
            .movable(false)
            .selectable(false),
        Column::new("name", "Name")
            .width(px(300.))
            .sortable()
            .movable(false),
        Column::new("size", "Size")
            .width(px(100.))
            .sortable()
            .movable(false),
        Column::new("status", "Status")
            .width(px(180.))
            .movable(false),
        Column::new("rate", "Transfer rate")
            .width(px(120.))
            .movable(false),
        Column::new("queue", "Queue").width(px(140.)).movable(false),
        Column::new("time-left", "Time left")
            .width(px(100.))
            .movable(false),
        Column::new("last-try", "Last try")
            .width(px(160.))
            .movable(false),
        Column::new("description", "Description")
            .width(px(240.))
            .movable(false),
        Column::new("protocol", "Type")
            .width(px(100.))
            .movable(false),
    ]
}

impl DownloadRow {
    fn time_left_label(&self) -> String {
        self.time_left.map(formatting::duration).unwrap_or_default()
    }

    fn last_try_label(&self) -> String {
        self.last_try
            .map(|value| {
                DateTime::<Local>::from(value)
                    .format("%Y-%m-%d %H:%M:%S")
                    .to_string()
            })
            .unwrap_or_default()
    }
}

impl TableDelegate for DownloadTable {
    fn context_menu(
        &mut self,
        row_ix: usize,
        menu: PopupMenu,
        window: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> PopupMenu {
        let Some(id) = self.rows.get(row_ix).map(|row| row.id) else {
            return menu;
        };
        let Some(owner) = self.owner.upgrade() else {
            return menu;
        };
        owner
            .read(cx)
            .row_context_menu(id, menu, window, self.owner.clone())
    }

    fn columns_count(&self, _: &App) -> usize {
        self.columns.len()
    }
    fn rows_count(&self, _: &App) -> usize {
        self.rows.len()
    }
    fn column(&self, col_ix: usize, _: &App) -> Column {
        self.columns[col_ix].clone()
    }

    fn perform_sort(
        &mut self,
        col_ix: usize,
        sort: ColumnSort,
        _: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) {
        let column = match col_ix {
            1 => SortColumn::Name,
            2 => SortColumn::Size,
            _ => return,
        };
        let owner = self.owner.clone();
        // TableState owns this delegate during sorting. Updating it through the
        // manager now would reborrow that entity; schedule after the borrow ends.
        cx.defer(move |cx| {
            let _ = owner.update(cx, |manager, cx| manager.sort_table(column, sort, cx));
        });
    }

    fn render_th(
        &mut self,
        col_ix: usize,
        _: &mut Window,
        _: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        if col_ix == 0 {
            let owner = self.owner.clone();
            Checkbox::new("select-visible")
                .debug_selector(|| "select-visible".into())
                .accessibility_label("Select all visible downloads")
                .checked(self.header_all_selected)
                .disabled(self.rows.is_empty())
                .on_change(move |checked, _, cx| {
                    cx.stop_propagation();
                    let checked = *checked;
                    let owner = owner.clone();
                    cx.defer(move |cx| {
                        let _ = owner.update(cx, |manager, cx| manager.select_visible(checked, cx));
                    });
                })
                .into_any_element()
        } else {
            let key = self.columns[col_ix].key.clone();
            div()
                .debug_selector(move || format!("download-header-{key}"))
                .child(Label::new(self.columns[col_ix].name.clone()))
                .into_any_element()
        }
    }

    fn render_td(
        &mut self,
        row_ix: usize,
        col_ix: usize,
        _: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        #[cfg(test)]
        self.rendered_cells.set(self.rendered_cells.get() + 1);
        let row = &self.rows[row_ix];
        let id = row.id;
        match col_ix {
            0 => {
                let owner = self.owner.clone();
                Checkbox::new(("select-download", id))
                    .debug_selector(move || format!("select-download-{id}"))
                    .accessibility_label(format!("Select {}", row.name))
                    .checked(row.selected)
                    .on_change(move |checked, _, cx| {
                        cx.stop_propagation();
                        let checked = *checked;
                        let owner = owner.clone();
                        cx.defer(move |cx| {
                            let _ = owner
                                .update(cx, |manager, cx| manager.select_item(id, checked, cx));
                        });
                    })
                    .into_any_element()
            }
            1 => {
                let owner = self.owner.clone();
                h_flex()
                    .gap_2()
                    .child(crate::file_icons::category_icon(row.category, cx))
                    .child(
                        Link::new(("download-name", id))
                            // A target URL would launch externally even with on_click.
                            // Details are an application action, so deliberately omit href.
                            .child(
                                div()
                                    .debug_selector(move || format!("download-name-{id}"))
                                    .child(Label::new(row.name.clone())),
                            )
                            .on_click(move |_, window, cx| {
                                let _ = owner
                                    .update(cx, |manager, cx| manager.open_details(id, window, cx));
                            }),
                    )
                    .into_any_element()
            }
            2 => Label::new(row.size.map(formatting::bytes).unwrap_or_default()).into_any_element(),
            3 => h_flex()
                .gap_2()
                .child(
                    Icon::new(status_icon(&row.status)).text_color(status_color(&row.status, cx)),
                )
                .child(
                    Label::new(status_label(&row.status, row.progress))
                        .text_color(status_color(&row.status, cx)),
                )
                .into_any_element(),
            4 => Label::new(
                row.rate
                    .map(|rate| format!("{}/s", formatting::bytes(rate as u64)))
                    .unwrap_or_default(),
            )
            .into_any_element(),
            5 => Label::new(row.queue_name.clone()).into_any_element(),
            6 => Label::new(row.time_left_label()).into_any_element(),
            7 => Label::new(row.last_try_label()).into_any_element(),
            8 => Label::new(row.description.clone()).into_any_element(),
            9 => Label::new(if row.is_torrent { "Torrent" } else { "HTTP" }).into_any_element(),
            _ => unreachable!("unknown download table column"),
        }
    }
}

/// Unknown lengths and in-progress zero totals must not imply completion.
fn status_label(status: &str, progress: Option<ulos_core::DownloadProgress>) -> String {
    let percent = progress.and_then(|progress| {
        if progress.total_bytes == Some(0) {
            return (status == "Finished" && progress.downloaded_bytes == 0).then_some(100.0);
        }
        formatting::percentage(progress)
    });
    match percent {
        Some(percent) => format!("{status} · {percent:.1}%"),
        None => status.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::{DownloadRow, columns, status_label};
    use crate::queue::Category;
    use chrono::{DateTime, Local};
    use std::time::{Duration, SystemTime};
    use ulos_core::DownloadProgress;

    fn snapshot() -> DownloadRow {
        DownloadRow {
            is_torrent: false,
            id: 7,
            name: "archive.zip".into(),
            category: Category::Compressed,
            size: None,
            status: "Queued".into(),
            progress: None,
            rate: None,
            selected: false,
            queue_name: "Main queue".into(),
            time_left: None,
            last_try: None,
            description: String::new(),
        }
    }

    #[test]
    fn headers_preserve_original_indexes_and_append_snapshot_columns() {
        let columns = columns();
        let contract = [
            ("selected", "", 40.),
            ("name", "Name", 300.),
            ("size", "Size", 100.),
            ("status", "Status", 180.),
            ("rate", "Transfer rate", 120.),
            ("queue", "Queue", 140.),
            ("time-left", "Time left", 100.),
            ("last-try", "Last try", 160.),
            ("description", "Description", 240.),
            ("protocol", "Type", 100.),
        ];
        assert_eq!(columns.len(), contract.len());
        for (index, (column, (key, label, width))) in columns.iter().zip(contract).enumerate() {
            assert_eq!(column.key.as_ref(), key);
            assert_eq!(column.name.as_ref(), label);
            assert_eq!(column.width, gpui_kit::px(width));
            assert!(!column.movable);
            assert_eq!(column.sort.is_some(), matches!(index, 1 | 2));
        }
        assert!(!columns[0].resizable);
        assert!(!columns[0].selectable);
    }

    #[test]
    fn missing_snapshot_values_stay_empty() {
        let row = snapshot();
        assert_eq!(row.queue_name, "Main queue");
        assert_eq!(row.time_left_label(), "");
        assert_eq!(row.last_try_label(), "");
        assert_eq!(row.description, "");
    }

    #[test]
    fn snapshot_formats_actual_duration_and_local_attempt_time() {
        let mut row = snapshot();
        for (seconds, expected) in [(0, "0s"), (59, "59s"), (61, "1m 1s"), (3661, "1h 1m")] {
            row.time_left = Some(Duration::from_secs(seconds));
            assert_eq!(row.time_left_label(), expected);
        }
        let attempted = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        row.last_try = Some(attempted);
        assert_eq!(
            row.last_try_label(),
            DateTime::<Local>::from(attempted)
                .format("%Y-%m-%d %H:%M:%S")
                .to_string(),
        );
        row.description = "Release archive".into();
        assert_eq!(row.description, "Release archive");
    }

    #[test]
    fn known_progress_supplements_readable_status() {
        assert_eq!(
            status_label(
                "Downloading",
                Some(DownloadProgress {
                    downloaded_bytes: 5,
                    total_bytes: Some(10)
                })
            ),
            "Downloading · 50.0%"
        );
        assert_eq!(
            status_label(
                "Failed",
                Some(DownloadProgress {
                    downloaded_bytes: 5,
                    total_bytes: Some(10)
                })
            ),
            "Failed · 50.0%"
        );
        assert_eq!(
            status_label(
                "Finished",
                Some(DownloadProgress {
                    downloaded_bytes: 0,
                    total_bytes: Some(0)
                })
            ),
            "Finished · 100.0%"
        );
    }

    #[test]
    fn unknown_totals_do_not_invent_percentages() {
        for progress in [
            None,
            Some(DownloadProgress {
                downloaded_bytes: 5,
                total_bytes: None,
            }),
            Some(DownloadProgress {
                downloaded_bytes: 0,
                total_bytes: Some(0),
            }),
        ] {
            assert_eq!(status_label("Downloading", progress), "Downloading");
        }
    }
}
