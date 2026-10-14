//! Passive presentation of a parent-owned transfer. This view never starts or stops I/O.
use std::{path::PathBuf, time::Duration};

use gpui_kit::{
    component::{
        ActiveTheme, Disableable,
        button::Button,
        label::Label,
        progress::Progress,
        spinner::Spinner,
        tab::{Tab, TabBar},
        table::{Table, TableBody, TableCell, TableHead, TableHeader, TableRow},
    },
    prelude::FluentBuilder as _,
    *,
};
use ulos_core::DownloadProgress;

use crate::{formatting, queue::DownloadId};

/// Display-only data; the parent must remove URL credentials and query values
/// before supplying `source` (and sanitize any error text). Deliberately not Debug.
#[derive(Clone)]
pub struct DetailsSnapshot {
    pub id: DownloadId,
    pub name: String,
    pub source: String,
    pub destination: PathBuf,
    pub status: String,
    pub progress: Option<DownloadProgress>,
    pub speed: Option<f64>,
    pub eta: Option<Duration>,
    pub active: bool,
    pub cancelling: bool,
    pub error: Option<String>,
}

pub struct CancelDownload(pub DownloadId);
pub struct CloseDetails;

#[derive(Clone, Copy, PartialEq, Eq)]
enum DetailsTab {
    Status,
    SpeedLimiter,
    Completion,
}

pub struct DownloadDetails {
    pub snapshot: DetailsSnapshot,
    pub show_details: bool,
    tab: DetailsTab,
}

impl EventEmitter<CancelDownload> for DownloadDetails {}
impl EventEmitter<CloseDetails> for DownloadDetails {}

impl DownloadDetails {
    pub fn new(snapshot: DetailsSnapshot) -> Self {
        Self {
            snapshot,
            show_details: true,
            tab: DetailsTab::Status,
        }
    }

    /// Keep the same entity while updating telemetry, preserving tab and disclosure state.
    pub fn update(&mut self, snapshot: DetailsSnapshot, cx: &mut Context<Self>) {
        self.snapshot = snapshot;
        cx.notify();
    }

    fn percentage(&self) -> Option<f32> {
        self.snapshot.progress.and_then(|progress| {
            // A zero-length body has no meaningful byte ratio; don't invent 100%.
            progress
                .total_bytes
                .filter(|total| *total > 0)
                .and_then(|_| formatting::percentage(progress))
        })
    }

    fn downloaded(&self) -> String {
        match self.snapshot.progress {
            Some(progress) => match self.percentage() {
                Some(percent) => format!("{} ({percent:.1}%)", formatting::progress_text(progress)),
                None => format!(
                    "{} · percentage unknown",
                    formatting::progress_text(progress)
                ),
            },
            None => "Not available · percentage unknown".into(),
        }
    }

    fn connection_info(&self) -> &'static str {
        if self.snapshot.error.is_some() || self.snapshot.status == "Failed" {
            "Error"
        } else if self.snapshot.cancelling {
            "Cancelling"
        } else if self.snapshot.status == "Finished" {
            "Finished"
        } else if self.snapshot.active && self.snapshot.progress.is_some() {
            "Downloading"
        } else if self.snapshot.active {
            "Connecting"
        } else if self.snapshot.status == "Cancelled" {
            "Cancelled"
        } else {
            "Inactive"
        }
    }

    fn status_content(&self, cx: &App) -> AnyElement {
        let snapshot = &self.snapshot;
        let rate = snapshot
            .speed
            .filter(|rate| rate.is_finite() && *rate >= 0.0)
            .map(|rate| format!("{}/s", formatting::bytes(rate.min(u64::MAX as f64) as u64)))
            .unwrap_or_else(|| "Not available".into());
        let fields = [
            ("Status", snapshot.status.clone()),
            (
                "File size",
                snapshot
                    .progress
                    .and_then(|progress| progress.total_bytes)
                    .map(formatting::bytes)
                    .unwrap_or_else(|| "Unknown".into()),
            ),
            ("Downloaded", self.downloaded()),
            ("Transfer rate", rate),
            (
                "Time left",
                snapshot
                    .eta
                    .map(formatting::duration)
                    .unwrap_or_else(|| "Unknown".into()),
            ),
            ("Resume capability", "Not implemented".into()),
            (
                "Save to",
                snapshot.destination.to_string_lossy().into_owned(),
            ),
        ];
        div()
            .flex()
            .flex_col()
            .gap_2()
            .border_1()
            .border_color(cx.theme().border)
            .rounded_md()
            .p_3()
            .children(fields.into_iter().map(|(label, value)| {
                div()
                    .flex()
                    .items_start()
                    .gap_3()
                    .child(Label::new(label).w(px(126.)).flex_shrink_0())
                    .child(Label::new(value).flex_1().min_w_0())
            }))
            .when_some(snapshot.error.clone(), |panel, error| {
                panel.child(Label::new(format!("Error: {error}")))
            })
            .into_any_element()
    }

    fn connection_content(&self, cx: &App) -> AnyElement {
        let percent = self.percentage();
        // No pretend connection exists for a queued/preparing item. Retained byte
        // telemetry or a completed result represents the one actual HTTP stream.
        let has_stream = self.snapshot.progress.is_some() || self.snapshot.status == "Finished";
        let body = TableBody::new().when(has_stream, |body| {
            body.child(
                TableRow::new()
                    .child(
                        TableCell::new()
                            .w(px(40.))
                            .min_w(px(40.))
                            .child(Label::new("1")),
                    )
                    .child(
                        TableCell::new().flex_1().child(Label::new(
                            self.snapshot
                                .progress
                                .map(|progress| formatting::bytes(progress.downloaded_bytes))
                                .unwrap_or_else(|| "Not available".into()),
                        )),
                    )
                    .child(
                        TableCell::new()
                            .flex_1()
                            .child(Label::new(self.connection_info())),
                    ),
            )
        });
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(Label::new("Connection progress"))
            .when(has_stream, |view| {
                view.child(
                    Progress::new("details-connection-progress")
                        .accessibility_label("Current HTTP stream progress")
                        .loading(self.snapshot.active && percent.is_none())
                        .value(percent.unwrap_or(0.0)),
                )
                .child(Label::new(
                    percent
                        .map(|percent| format!("{percent:.1}%"))
                        .unwrap_or_else(|| "Percentage unknown".into()),
                ))
            })
            .child(
                Table::new()
                    .accessibility_label("HTTP connection progress")
                    .child(
                        TableHeader::new().child(
                            TableRow::new()
                                .child(
                                    TableHead::new()
                                        .w(px(40.))
                                        .min_w(px(40.))
                                        .child(Label::new("N")),
                                )
                                .child(TableHead::new().flex_1().child(Label::new("Downloaded")))
                                .child(TableHead::new().flex_1().child(Label::new("Info"))),
                        ),
                    )
                    .child(body),
            )
            .when(!has_stream, |view| {
                view.child(Label::new("No connection progress yet"))
            })
            .child(
                Label::new("Single HTTP connection; segmented transfers are not implemented.")
                    .text_xs()
                    .text_color(cx.theme().muted_foreground),
            )
            .into_any_element()
    }
}

impl Render for DownloadDetails {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let percent = self.percentage();
        let tab_index = match self.tab {
            DetailsTab::Status => 0,
            DetailsTab::SpeedLimiter => 1,
            DetailsTab::Completion => 2,
        };
        let content = match self.tab {
            DetailsTab::Status => self.status_content(cx),
            DetailsTab::SpeedLimiter => Label::new(
                "Speed limiting is not implemented. Transfers currently use the available bandwidth.",
            )
            .into_any_element(),
            DetailsTab::Completion => Label::new(
                "Options on completion are not implemented. No automatic open, shutdown, or other completion action is performed.",
            )
            .into_any_element(),
        };
        div()
            .id("download-details")
            .w_full()
            .max_w(px(590.))
            .max_h(px(550.))
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap_3()
            .text_sm()
            .text_color(cx.theme().foreground)
            .child(Label::new(self.snapshot.name.clone()).font_weight(FontWeight::SEMIBOLD))
            .child(Label::new(format!("URL: {}", self.snapshot.source)))
            .child(
                TabBar::new("details-tabs")
                    .selected_index(tab_index)
                    .child(Tab::new().label("Download status"))
                    .child(Tab::new().label("Speed limiter"))
                    .child(Tab::new().label("Options on completion"))
                    .on_click(cx.listener(|view, index, _, cx| {
                        view.tab = match index {
                            1 => DetailsTab::SpeedLimiter,
                            2 => DetailsTab::Completion,
                            _ => DetailsTab::Status,
                        };
                        cx.notify();
                    })),
            )
            .child(content)
            .child(
                Progress::new("details-overall-progress")
                    .accessibility_label("Overall download progress")
                    .loading(self.snapshot.active && percent.is_none())
                    .value(percent.unwrap_or(0.0)),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .when(self.snapshot.active && percent.is_none(), |view| {
                        view.child(Spinner::new())
                    })
                    .child(Label::new(
                        percent
                            .map(|percent| format!("{percent:.1}%"))
                            .unwrap_or_else(|| "Percentage unknown".into()),
                    )),
            )
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(
                        Button::new("details-disclosure")
                            .label(if self.show_details {
                                "Hide details"
                            } else {
                                "Show details"
                            })
                            .on_click(cx.listener(|view, _, _, cx| {
                                view.show_details = !view.show_details;
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("details-pause")
                            .label("Pause")
                            .disabled(true)
                            .tooltip("Pause and resume are not implemented"),
                    )
                    .child(
                        Button::new("details-cancel")
                            .label(if self.snapshot.cancelling {
                                "Cancelling…"
                            } else {
                                "Cancel"
                            })
                            .disabled(!self.snapshot.active || self.snapshot.cancelling)
                            .on_click(cx.listener(|view, _, _, cx| {
                                if view.snapshot.active && !view.snapshot.cancelling {
                                    cx.emit(CancelDownload(view.snapshot.id));
                                }
                            })),
                    ),
            )
            .when(self.show_details, |view| {
                view.child(self.connection_content(cx))
            })
            .child(
                div().flex().justify_end().child(
                    Button::new("details-close")
                        .label("Close")
                        .on_click(cx.listener(|_, _, _, cx| cx.emit(CloseDetails))),
                ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::{DetailsSnapshot, DownloadDetails};
    use ulos_core::DownloadProgress;

    fn details(progress: Option<DownloadProgress>) -> DownloadDetails {
        DownloadDetails::new(DetailsSnapshot {
            id: 1,
            name: "file.bin".into(),
            source: "https://example.org/file.bin".into(),
            destination: "file.bin".into(),
            status: "Downloading".into(),
            progress,
            speed: None,
            eta: None,
            active: true,
            cancelling: false,
            error: None,
        })
    }

    #[test]
    fn missing_and_zero_totals_do_not_claim_completion() {
        for progress in [
            None,
            Some(DownloadProgress {
                downloaded_bytes: 12,
                total_bytes: None,
            }),
            Some(DownloadProgress {
                downloaded_bytes: 0,
                total_bytes: Some(0),
            }),
        ] {
            let details = details(progress);
            assert_eq!(details.percentage(), None);
            assert!(details.downloaded().contains("percentage unknown"));
        }
    }

    #[test]
    fn one_stream_uses_real_bytes_and_terminal_state() {
        let mut details = details(Some(DownloadProgress {
            downloaded_bytes: 50,
            total_bytes: Some(100),
        }));
        assert_eq!(details.percentage(), Some(50.0));
        assert!(details.downloaded().contains("50 B of 100 B (50.0%)"));
        assert_eq!(details.connection_info(), "Downloading");
        details.snapshot.active = false;
        details.snapshot.status = "Finished".into();
        assert_eq!(details.connection_info(), "Finished");
        details.snapshot.error = Some("Transfer failed".into());
        assert_eq!(details.connection_info(), "Error");
    }
}
