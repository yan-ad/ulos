//! Snapshot-backed Kit table. Only viewport cells instantiate visual controls.
use std::path::PathBuf;

use gpui_kit::{
    component::{
        Disableable,
        button::Button,
        checkbox::Checkbox,
        label::Label,
        table::{Column, ColumnSort, TableDelegate, TableState},
    },
    *,
};

use crate::{
    formatting,
    manager::DownloadManager,
    queue::{Category, DownloadId, SortColumn},
};
use gpui_kit::assets::IconName as AssetIcon;

/// Presentation data, refreshed by the manager when queue state changes.
#[derive(Clone, Debug)]
pub struct DownloadRow {
    pub id: DownloadId,
    pub name: String,
    pub category: Category,
    pub size: Option<u64>,
    pub status: String,
    pub rate: Option<f64>,
    pub selected: bool,
    pub destination: PathBuf,
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
            columns: vec![
                Column::new("selected", "")
                    .width(px(40.))
                    .resizable(false)
                    .movable(false)
                    .selectable(false),
                Column::new("name", "Name")
                    .width(px(400.))
                    .sortable()
                    .movable(false),
                Column::new("size", "Size")
                    .width(px(100.))
                    .sortable()
                    .movable(false),
                Column::new("status", "Status")
                    .width(px(140.))
                    .movable(false),
                Column::new("rate", "Transfer rate")
                    .width(px(130.))
                    .movable(false),
            ],
            header_all_selected: false,
            #[cfg(test)]
            rendered_cells: Default::default(),
        }
    }
}

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

impl TableDelegate for DownloadTable {
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
                .accessibility_label("Select all visible downloads")
                .checked(self.header_all_selected)
                .disabled(self.rows.is_empty())
                .on_change(move |checked, _, cx| {
                    let checked = *checked;
                    let owner = owner.clone();
                    cx.defer(move |cx| {
                        let _ = owner.update(cx, |manager, cx| manager.select_visible(checked, cx));
                    });
                })
                .into_any_element()
        } else {
            Label::new(self.columns[col_ix].name.clone()).into_any_element()
        }
    }

    fn render_td(
        &mut self,
        row_ix: usize,
        col_ix: usize,
        _: &mut Window,
        _: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        #[cfg(test)]
        self.rendered_cells.set(self.rendered_cells.get() + 1);
        let row = &self.rows[row_ix];
        let id = row.id;
        match col_ix {
            0 => {
                let owner = self.owner.clone();
                Checkbox::new(("select-download", id))
                    .accessibility_label(format!("Select {}", row.name))
                    .checked(row.selected)
                    .on_change(move |checked, _, cx| {
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
                Button::new(("download-name", id))
                    .accessibility_label(format!("Open details for {}", row.name))
                    .icon(category_icon(row.category))
                    .label(row.name.clone())
                    .tooltip(row.destination.display().to_string())
                    .on_click(move |_, window, cx| {
                        let _ =
                            owner.update(cx, |manager, cx| manager.open_details(id, window, cx));
                    })
                    .into_any_element()
            }
            2 => Label::new(
                row.size
                    .map(formatting::bytes)
                    .unwrap_or_else(|| "—".into()),
            )
            .into_any_element(),
            3 => Label::new(row.status.clone()).into_any_element(),
            _ => Label::new(
                row.rate
                    .map(|rate| format!("{}/s", formatting::bytes(rate as u64)))
                    .unwrap_or_else(|| "—".into()),
            )
            .into_any_element(),
        }
    }
}
