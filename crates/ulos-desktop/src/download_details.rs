//! Presentation and intent events for a parent-owned transfer; this view never performs I/O.
use std::{path::PathBuf, time::Duration};

use gpui_kit::{
    assets::IconName as KitIcon,
    component::{
        ActiveTheme, Disableable, Icon, Sizable, Size,
        button::Button,
        checkbox::Checkbox,
        description_list::{DescriptionItem, DescriptionList},
        group_box::{GroupBox, GroupBoxVariants},
        h_flex,
        input::{Input, InputEvent, InputState},
        label::Label,
        progress::Progress,
        tab::{Tab, TabBar},
        table::{Table, TableBody, TableCell, TableHead, TableHeader, TableRow},
        toolbar::Toolbar,
        v_flex,
    },
    prelude::FluentBuilder as _,
    *,
};
use ulos_core::{
    DownloadProgress,
    transfer::{
        CollisionPolicy, ConnectionStatus, MAX_CONNECTIONS, ResumeCapability, TransferOptions,
        TransferTelemetry,
    },
};

use crate::{formatting, queue::DownloadId};

/// The parent sanitizes source credentials/query values and error text. Deliberately not Debug.
#[derive(Clone, PartialEq, Default)]
pub struct DetailsSnapshot {
    pub id: DownloadId,
    pub name: String,
    pub source: String,
    pub destination: PathBuf,
    /// Friendly folder label computed by the parent from cached platform paths.
    pub folder_display: String,
    pub status: String,
    pub progress: Option<DownloadProgress>,
    pub speed: Option<f64>,
    pub eta: Option<Duration>,
    pub active: bool,
    pub cancelling: bool,
    pub error: Option<String>,
    pub telemetry: TransferTelemetry,
    pub options: TransferOptions,
    pub can_retry: bool,
    pub shutdown_armed: bool,
    pub shutdown_remaining: Option<u64>,
    pub completion_error: Option<String>,
    pub paused: bool,
    pub is_torrent: bool,
    pub output_directory: bool,
    pub torrent: Option<ulos_core::torrent::TorrentTelemetry>,
    pub torrent_fresh: bool,
    pub torrent_settings: ulos_core::torrent::TorrentSettings,
}

pub struct CancelDownload(pub DownloadId);
pub struct CloseDetails;
pub struct TogglePause(pub DownloadId);
pub struct SetRateLimit {
    pub id: DownloadId,
    pub bytes_per_second: Option<u64>,
}
pub struct SetTorrentLimits {
    pub id: DownloadId,
    pub download: Option<u64>,
    pub upload: Option<u64>,
}
pub struct SetTorrentPeers {
    pub id: DownloadId,
    pub peers: usize,
}
pub struct SetConnections {
    pub id: DownloadId,
    pub connections: usize,
}
pub struct RetryDownload {
    pub id: DownloadId,
    pub collision: CollisionPolicy,
}
/// Intent only: the parent must obtain confirmation before arming shutdown.
pub struct RequestShutdown(pub DownloadId);
pub struct CancelShutdown(pub DownloadId);

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
    pub(crate) rate_input: Option<Entity<InputState>>,
    pub(crate) connections_input: Option<Entity<InputState>>,
    torrent_upload_input: Option<Entity<InputState>>,
    torrent_peers_input: Option<Entity<InputState>>,
    rate_enabled: bool,
    input_error: Option<String>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<CancelDownload> for DownloadDetails {}
impl EventEmitter<CloseDetails> for DownloadDetails {}
impl EventEmitter<TogglePause> for DownloadDetails {}
impl EventEmitter<SetRateLimit> for DownloadDetails {}
impl EventEmitter<SetConnections> for DownloadDetails {}
impl EventEmitter<SetTorrentLimits> for DownloadDetails {}
impl EventEmitter<SetTorrentPeers> for DownloadDetails {}
impl EventEmitter<RetryDownload> for DownloadDetails {}
impl EventEmitter<RequestShutdown> for DownloadDetails {}
impl EventEmitter<CancelShutdown> for DownloadDetails {}

/// Stock Kit glyphs paired with readable state labels in the table and status window.
pub(crate) fn status_icon(status: &str) -> KitIcon {
    match status {
        "Failed" | "Error" => KitIcon::CircleAlert,
        "Cancelled" | "Cancelling" | "Cancelling…" => KitIcon::CircleX,
        "Finished" | "Complete" => KitIcon::CircleCheck,
        "Paused" => KitIcon::Pause,
        "Connecting"
        | "Preparing torrent"
        | "Checking"
        | "Searching for peers"
        | "Connecting to peers"
        | "Waiting for peers" => KitIcon::LoaderCircle,
        "No peer discovery" | "Stalled" => KitIcon::CircleAlert,
        "Downloading" => KitIcon::Download,
        _ => KitIcon::Clock,
    }
}

/// Semantic status colors apply only to content Labels/Icons, never controls.
pub(crate) fn status_color(status: &str, cx: &App) -> Hsla {
    match status {
        "Finished" | "Complete" => cx.theme().success,
        "Failed" | "Error" => cx.theme().danger,
        _ => cx.theme().foreground,
    }
}

pub(crate) fn parse_optional_rate(text: &str) -> Result<Option<u64>, &'static str> {
    if text.trim().is_empty() {
        Ok(None)
    } else {
        parse_rate(text).map(Some)
    }
}
pub(crate) fn parse_peers(text: &str) -> Result<usize, &'static str> {
    text.trim()
        .parse::<usize>()
        .ok()
        .filter(|value| (1..=10000).contains(value))
        .ok_or("Peers must be a whole number from 1 to 10000.")
}
fn parse_rate(text: &str) -> Result<u64, &'static str> {
    let kib = text
        .trim()
        .parse::<u64>()
        .map_err(|_| "Enter a whole number of KiB/s (1–1048576).")?;
    if !(1..=1024 * 1024).contains(&kib) {
        return Err("Rate must be 1–1048576 KiB/s.");
    }
    kib.checked_mul(1024).ok_or("Rate is too large.")
}
fn parse_connections(text: &str) -> Result<usize, String> {
    let count = text
        .trim()
        .parse::<usize>()
        .map_err(|_| format!("Enter a whole number of connections (1–{MAX_CONNECTIONS})."))?;
    if !(1..=MAX_CONNECTIONS).contains(&count) {
        return Err(format!("Connections must be 1–{MAX_CONNECTIONS}."));
    }
    Ok(count)
}

impl DownloadDetails {
    /// Inputs are created once, on first render, to retain the window-free constructor.
    pub fn new(snapshot: DetailsSnapshot) -> Self {
        Self {
            rate_enabled: snapshot.options.bytes_per_second.is_some(),
            snapshot,
            show_details: false,
            tab: DetailsTab::Status,
            rate_input: None,
            connections_input: None,
            torrent_upload_input: None,
            torrent_peers_input: None,
            input_error: None,
            _subscriptions: Vec::new(),
        }
    }
    pub fn update(&mut self, snapshot: DetailsSnapshot, cx: &mut Context<Self>) {
        if self.snapshot != snapshot {
            self.snapshot = snapshot;
            cx.notify();
        }
    }
    pub fn selected_tab_index(&self) -> usize {
        match self.tab {
            DetailsTab::Status => 0,
            DetailsTab::SpeedLimiter => 1,
            DetailsTab::Completion => 2,
        }
    }
    pub fn select_tab(&mut self, index: usize, cx: &mut Context<Self>) {
        self.tab = match index {
            1 => DetailsTab::SpeedLimiter,
            2 => DetailsTab::Completion,
            _ => DetailsTab::Status,
        };
        cx.notify();
    }
    fn ensure_inputs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.snapshot.is_torrent
            && self.snapshot.torrent_settings.allow_upload
            && self.rate_input.is_none()
        {
            let settings = &self.snapshot.torrent_settings;
            let down = settings
                .download_bytes_per_second
                .map(|rate| (rate / 1024).to_string())
                .unwrap_or_default();
            let up = settings
                .upload_bytes_per_second
                .map(|rate| (rate / 1024).to_string())
                .unwrap_or_default();
            let peers = settings.peer_limit.to_string();
            self.rate_input = Some(cx.new(|cx| {
                let mut input =
                    InputState::new(window, cx).placeholder("Download KiB/s; blank = unlimited");
                input.set_value(down, window, cx);
                input
            }));
            self.torrent_upload_input = Some(cx.new(|cx| {
                let mut input =
                    InputState::new(window, cx).placeholder("Upload KiB/s; blank = unlimited");
                input.set_value(up, window, cx);
                input
            }));
            self.torrent_peers_input = Some(cx.new(|cx| {
                let mut input = InputState::new(window, cx).placeholder("1–10000 peers");
                input.set_value(peers, window, cx);
                input
            }));
        }
        if self.snapshot.is_torrent || self.rate_input.is_some() {
            return;
        }
        let rate = self
            .snapshot
            .options
            .bytes_per_second
            .map(|bps| bps / 1024)
            .unwrap_or(1024)
            .to_string();
        let count = self.snapshot.options.connections.to_string();
        let rate_input = cx.new(|cx| {
            let mut input = InputState::new(window, cx).placeholder("KiB/s");
            input.set_value(rate, window, cx);
            input
        });
        let connections_input = cx.new(|cx| {
            let mut input = InputState::new(window, cx).placeholder(format!("1–{MAX_CONNECTIONS}"));
            input.set_value(count, window, cx);
            input
        });
        self._subscriptions.push(
            cx.subscribe_in(&rate_input, window, |view, _, event, _, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    view.apply_rate(cx);
                }
            }),
        );
        self._subscriptions.push(cx.subscribe_in(
            &connections_input,
            window,
            |view, _, event, _, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    view.apply_connections(cx);
                }
            },
        ));
        self.rate_input = Some(rate_input);
        self.connections_input = Some(connections_input);
    }
    fn can_configure_torrent(&self) -> bool {
        self.snapshot.is_torrent
            && self.snapshot.torrent_settings.allow_upload
            && !self.snapshot.cancelling
            && (self.snapshot.active || self.snapshot.status == "Added" || self.snapshot.can_retry)
    }
    pub(crate) fn apply_torrent_limits(&mut self, cx: &mut Context<Self>) {
        if !self.can_configure_torrent() {
            return;
        }
        let (Some(down), Some(up)) = (&self.rate_input, &self.torrent_upload_input) else {
            return;
        };
        match (
            parse_optional_rate(down.read(cx).value().as_ref()),
            parse_optional_rate(up.read(cx).value().as_ref()),
        ) {
            (Ok(download), Ok(upload)) => {
                self.input_error = None;
                cx.emit(SetTorrentLimits {
                    id: self.snapshot.id,
                    download,
                    upload,
                });
            }
            (Err(error), _) | (_, Err(error)) => self.input_error = Some(error.into()),
        }
        cx.notify();
    }
    pub(crate) fn apply_torrent_peers(&mut self, cx: &mut Context<Self>) {
        if !self.can_configure_torrent() {
            return;
        }
        let Some(input) = &self.torrent_peers_input else {
            return;
        };
        match parse_peers(input.read(cx).value().as_ref()) {
            Ok(peers) => {
                self.input_error = None;
                cx.emit(SetTorrentPeers {
                    id: self.snapshot.id,
                    peers,
                });
            }
            Err(error) => self.input_error = Some(error.into()),
        }
        cx.notify();
    }
    pub(crate) fn apply_rate(&mut self, cx: &mut Context<Self>) {
        if !self.can_configure_rate() {
            return;
        }
        let result = if self.rate_enabled {
            let Some(input) = &self.rate_input else {
                return;
            };
            parse_rate(input.read(cx).value().as_ref()).map(Some)
        } else {
            Ok(None)
        };
        match result {
            Ok(bytes_per_second) => {
                self.input_error = None;
                cx.emit(SetRateLimit {
                    id: self.snapshot.id,
                    bytes_per_second,
                });
            }
            Err(error) => self.input_error = Some(error.into()),
        }
        cx.notify();
    }
    fn can_configure_rate(&self) -> bool {
        !self.snapshot.is_torrent && self.snapshot.active && !self.snapshot.cancelling
    }
    fn can_pause(&self) -> bool {
        self.snapshot.active
            && !self.snapshot.cancelling
            && if self.snapshot.is_torrent {
                self.snapshot.torrent.as_ref().is_some_and(|telemetry| {
                    matches!(
                        telemetry.phase,
                        ulos_core::torrent::TorrentPhase::Downloading
                            | ulos_core::torrent::TorrentPhase::Paused
                    )
                })
            } else {
                self.snapshot.telemetry.resume == ResumeCapability::Supported
            }
    }
    fn can_configure_connections(&self) -> bool {
        !self.snapshot.is_torrent
            && !self.snapshot.active
            && self.snapshot.status != "Finished"
            && (self.snapshot.status == "Added" || self.snapshot.can_retry)
    }
    pub(crate) fn apply_connections(&mut self, cx: &mut Context<Self>) {
        if !self.can_configure_connections() {
            return;
        }
        let Some(input) = &self.connections_input else {
            return;
        };
        match parse_connections(input.read(cx).value().as_ref()) {
            Ok(connections) => {
                self.input_error = None;
                cx.emit(SetConnections {
                    id: self.snapshot.id,
                    connections,
                });
            }
            Err(error) => self.input_error = Some(error),
        }
        cx.notify();
    }
    fn percentage(&self) -> Option<f32> {
        self.snapshot.progress.and_then(|progress| {
            if self.snapshot.status == "Finished"
                && progress.total_bytes == Some(0)
                && progress.downloaded_bytes == 0
            {
                return Some(100.0);
            }
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
        } else if self.snapshot.paused {
            "Paused"
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
    fn folder_display(&self) -> String {
        if !self.snapshot.folder_display.is_empty() {
            return self.snapshot.folder_display.clone();
        }
        let folder = if self.snapshot.output_directory {
            self.snapshot.destination.as_path()
        } else {
            self.snapshot
                .destination
                .parent()
                .unwrap_or(&self.snapshot.destination)
        };
        crate::platform_paths::folder_label(folder, None, None)
    }
    fn status_content(&self, cx: &App) -> AnyElement {
        let s = &self.snapshot;
        let rate = s
            .speed
            .filter(|rate| rate.is_finite() && *rate >= 0.0)
            .map(|rate| format!("{}/s", formatting::bytes(rate.min(u64::MAX as f64) as u64)))
            .unwrap_or_else(|| "Not available".into());
        let mut fields = vec![
            ("Downloaded", KitIcon::Download, self.downloaded()),
            ("Rate", KitIcon::Gauge, rate),
            (
                "Left",
                KitIcon::Timer,
                s.eta
                    .map(formatting::duration)
                    .unwrap_or_else(|| "Unknown".into()),
            ),
            (
                "Resume",
                KitIcon::RotateCw,
                s.telemetry.resume.label().into(),
            ),
            ("Folder", KitIcon::Folder, self.folder_display()),
            ("Source", KitIcon::Link, s.source.clone()),
        ];
        if s.is_torrent {
            fields.retain(|(label, _, _)| !matches!(*label, "Resume" | "Rate"));
            fields.insert(0, ("Protocol", KitIcon::Link, "Torrent".into()));
            let metrics = self.torrent_summary_fields();
            for (label, value) in metrics.into_iter().rev() {
                fields.insert(2, (label, KitIcon::Gauge, value));
            }
        }
        GroupBox::new()
            .outline()
            .child(
                DescriptionList::horizontal()
                    .columns(1)
                    .bordered(false)
                    .children(fields.into_iter().map(|(label, icon, value)| {
                        // The readable Kit Label names the stock metadata glyph;
                        // no OS accent or default-opener identity is inferred.
                        DescriptionItem::new(
                            h_flex()
                                .gap_2()
                                .child(Icon::new(icon))
                                .child(Label::new(label))
                                .into_any_element(),
                        )
                        .value(
                            h_flex()
                                .min_w_0()
                                .debug_selector(move || {
                                    format!(
                                        "status-metric-{}",
                                        label.to_lowercase().replace(' ', "-")
                                    )
                                })
                                .child(Label::new(value).truncate())
                                .into_any_element(),
                        )
                    }))
                    .when_some(s.error.clone(), |list, error| {
                        list.item(
                            Label::new("Error").into_any_element(),
                            Label::new(error)
                                .text_color(cx.theme().danger)
                                .into_any_element(),
                            1,
                        )
                    }),
            )
            .when_some(
                s.torrent
                    .as_ref()
                    .filter(|_| s.is_torrent && s.active && s.torrent_fresh)
                    .and_then(|telemetry| telemetry.diagnostic()),
                |panel, message| panel.child(Label::new(message)),
            )
            .into_any_element()
    }
    fn torrent_summary_fields(&self) -> Vec<(&'static str, String)> {
        let Some(telemetry) = &self.snapshot.torrent else {
            return vec![
                ("Peers", "Not available".into()),
                ("Download speed", "Not available".into()),
                ("Upload speed", "Not available".into()),
            ];
        };
        let transferring = self.snapshot.active
            && telemetry.phase == ulos_core::torrent::TorrentPhase::Downloading;
        let live = transferring && self.snapshot.torrent_fresh;
        let unavailable = "Not available (waiting for an update)";
        vec![
            (
                "Peers",
                if live {
                    format!(
                        "{} connected, {} connecting, {} queued",
                        telemetry.peers, telemetry.connecting_peers, telemetry.queued_peers
                    )
                } else if transferring {
                    unavailable.into()
                } else {
                    "0 connected".into()
                },
            ),
            (
                "Download speed",
                if transferring && !live {
                    unavailable.into()
                } else {
                    format!(
                        "{}/s",
                        formatting::bytes(if live {
                            telemetry.download_bytes_per_second
                        } else {
                            0
                        })
                    )
                },
            ),
            (
                "Upload speed",
                if transferring && !live {
                    format!(
                        "{unavailable}{}",
                        if telemetry.upload_enabled {
                            ""
                        } else {
                            " (payload upload disabled)"
                        }
                    )
                } else {
                    format!(
                        "{}/s{}",
                        formatting::bytes(if live {
                            telemetry.upload_bytes_per_second
                        } else {
                            0
                        }),
                        if telemetry.upload_enabled {
                            ""
                        } else {
                            " (payload upload disabled)"
                        }
                    )
                },
            ),
            (
                "Trackers",
                format!(
                    "{} configured ({})",
                    telemetry.tracker_count,
                    if telemetry.dht_enabled {
                        "DHT enabled"
                    } else {
                        "DHT disabled"
                    }
                ),
            ),
        ]
    }
    fn torrent_fields(&self) -> Vec<(&'static str, String)> {
        let Some(telemetry) = &self.snapshot.torrent else {
            return vec![("Telemetry", "Not available".into())];
        };
        let active = self.snapshot.active;
        vec![
            (
                "Phase",
                if !active {
                    self.snapshot.status.clone()
                } else if telemetry.phase == ulos_core::torrent::TorrentPhase::Downloading
                    && !self.snapshot.torrent_fresh
                {
                    "Waiting for telemetry".into()
                } else {
                    telemetry.activity_label().into()
                },
            ),
            (
                "Peers",
                if !active {
                    "0".into()
                } else if self.snapshot.torrent_fresh {
                    telemetry.peers.to_string()
                } else {
                    "Not available".into()
                },
            ),
            ("Peers seen", telemetry.seen_peers.to_string()),
            (
                "No data for",
                if active && self.snapshot.torrent_fresh {
                    formatting::duration(Duration::from_secs(telemetry.stalled_for_secs))
                } else {
                    "Not applicable".into()
                },
            ),
            (
                "Verified bytes",
                format!(
                    "{} of {}",
                    formatting::bytes(telemetry.downloaded_bytes),
                    formatting::bytes(telemetry.total_bytes)
                ),
            ),
            ("Uploaded", formatting::bytes(telemetry.uploaded_bytes)),
        ]
    }
    fn connection_content(&self) -> AnyElement {
        if self.snapshot.is_torrent {
            return GroupBox::new()
                .outline()
                .title(Label::new("Torrent"))
                .child(
                    DescriptionList::horizontal()
                        .columns(1)
                        .bordered(false)
                        .children(self.torrent_fields().into_iter().map(|(label, value)| {
                            DescriptionItem::new(Label::new(label).into_any_element())
                                .value(Label::new(value).into_any_element())
                        })),
                )
                .into_any_element();
        }
        let body = TableBody::new().children(self.snapshot.telemetry.connections.iter().map(
            |connection| {
                let status = match connection.status {
                    ConnectionStatus::Connecting => "Connecting",
                    ConnectionStatus::Downloading => "Downloading",
                    ConnectionStatus::Paused => "Paused",
                    ConnectionStatus::Complete => "Complete",
                };
                TableRow::new()
                    .child(TableCell::new().child(Label::new(connection.id.to_string())))
                    .child(
                        TableCell::new()
                            .child(Label::new(formatting::bytes(connection.downloaded))),
                    )
                    .child(TableCell::new().child(Label::new(status)))
            },
        ));
        GroupBox::new()
            .outline()
            .title(Label::new("HTTP connections"))
            .child(
                Table::new()
                    .accessibility_label("Actual HTTP connection progress")
                    .child(
                        TableHeader::new().child(
                            TableRow::new()
                                .child(TableHead::new().child(Label::new("N")))
                                .child(TableHead::new().child(Label::new("Downloaded")))
                                .child(TableHead::new().child(Label::new("State"))),
                        ),
                    )
                    .child(body),
            )
            .when(self.snapshot.telemetry.connections.is_empty(), |view| {
                view.child(Label::new("No connection progress yet"))
            })
            .into_any_element()
    }
    fn speed_content(&self, cx: &mut Context<Self>) -> AnyElement {
        if self.snapshot.is_torrent {
            let enabled = self.can_configure_torrent();
            let mut group = GroupBox::new()
                .outline()
                .title(Label::new("Torrent limits"));
            if let (Some(down), Some(up), Some(peers)) = (
                &self.rate_input,
                &self.torrent_upload_input,
                &self.torrent_peers_input,
            ) {
                group = group
                    .child(Label::new(
                        "Download / upload KiB/s (blank = unlimited; 1–1048576)",
                    ))
                    .child(
                        h_flex()
                            .gap_2()
                            .child(Input::new(down).disabled(!enabled))
                            .child(Input::new(up).disabled(!enabled))
                            .child(
                                Button::new("torrent-rates-apply")
                                    .label("Apply rates")
                                    .disabled(!enabled)
                                    .on_click(
                                        cx.listener(|view, _, _, cx| view.apply_torrent_limits(cx)),
                                    ),
                            ),
                    )
                    .child(Label::new("Peer limit (1–10000)"))
                    .child(
                        h_flex()
                            .gap_2()
                            .child(Input::new(peers).disabled(!enabled))
                            .child(
                                Button::new("torrent-peers-apply")
                                    .label("Apply peers")
                                    .disabled(!enabled)
                                    .on_click(
                                        cx.listener(|view, _, _, cx| view.apply_torrent_peers(cx)),
                                    ),
                            ),
                    );
            }
            return group.child(Label::new(format!("Applied download: {} · upload: {} · peers: {}", self.snapshot.torrent_settings.download_bytes_per_second.map(|r| format!("{}/s", formatting::bytes(r))).unwrap_or_else(|| "unlimited".into()), self.snapshot.torrent_settings.upload_bytes_per_second.map(|r| format!("{}/s", formatting::bytes(r))).unwrap_or_else(|| "unlimited".into()), self.snapshot.torrent_settings.peer_limit)))
                .child(Label::new("Limits require native peer-upload consent. Paused transfers accept changes; queued/retry changes apply next start. No post-completion seeding or magnet links."))
                .when_some(self.input_error.clone(), |group, error| group.child(Label::new(error)))
                .into_any_element();
        }
        let enabled = self.can_configure_rate();
        v_flex().gap_3()
            .child(GroupBox::new().outline().title(Label::new("Speed limit"))
                .child(Checkbox::new("rate-enabled").label("Limit transfer rate").checked(self.rate_enabled).disabled(!enabled)
                    .on_click(cx.listener(|view, checked, _, cx| {
                        view.rate_enabled = *checked;
                        if !*checked { view.apply_rate(cx); }
                        cx.notify();
                    })))
                .child(h_flex().gap_2().child(Input::new(self.rate_input.as_ref().expect("inputs initialized")).disabled(!enabled || !self.rate_enabled))
                    .child(Label::new("KiB/s"))
                    .child(Button::new("rate-apply").label("Apply").disabled(!enabled).on_click(cx.listener(|view, _, _, cx| view.apply_rate(cx)))))
                .child(Label::new(self.snapshot.options.bytes_per_second.map(|bps| format!("Applied: {}/s", formatting::bytes(bps))).unwrap_or_else(|| "Applied: unlimited".into()))))
            .child(GroupBox::new().outline().title(Label::new("Connections for next start"))
                .child(h_flex().gap_2().child(Input::new(self.connections_input.as_ref().expect("inputs initialized")).disabled(!self.can_configure_connections()))
                    .child(Button::new("connections-apply").label("Apply").disabled(!self.can_configure_connections()).on_click(cx.listener(|view, _, _, cx| view.apply_connections(cx)))))
                .child(Label::new(format!("Configured: {} · Actual: {}", self.snapshot.options.connections, self.snapshot.telemetry.connections.len())))
                .footer(Label::new(format!("1–{MAX_CONNECTIONS} connections. Verified byte ranges and a strong validator are required; other servers use one stream. Changes apply on the next start, not during a transfer."))))
            .when_some(self.input_error.clone(), |view, error| view.child(Label::new(error)))
            .into_any_element()
    }
    fn completion_content(&self, cx: &mut Context<Self>) -> AnyElement {
        GroupBox::new().outline().title(Label::new("On completion"))
            .child(Checkbox::new("shutdown-enabled").label("Shut down after successful completion").checked(self.snapshot.shutdown_armed)
                .disabled(!self.snapshot.active && !self.snapshot.shutdown_armed)
                .on_click(cx.listener(|view, checked, _, cx| {
                    if *checked && view.snapshot.active { cx.emit(RequestShutdown(view.snapshot.id)); }
                    else if !*checked { cx.emit(CancelShutdown(view.snapshot.id)); }
                })))
            .child(Label::new("Requires confirmation. Shutdown follows a cancellable countdown and is blocked while another transfer is queued or active."))
            .when_some(self.snapshot.shutdown_remaining, |view, remaining| view.child(Label::new(format!("Shutdown in {remaining} seconds"))))
            .when(self.snapshot.shutdown_armed, |view| view.child(Button::new("shutdown-cancel").label("Cancel shutdown").on_click(cx.listener(|view, _, _, cx| cx.emit(CancelShutdown(view.snapshot.id))))))
            .when_some(self.snapshot.completion_error.clone(), |view, error| view.child(Label::new(error)))
            .into_any_element()
    }
}

impl Render for DownloadDetails {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.ensure_inputs(window, cx);
        let percent = self.percentage();
        let content = match self.tab {
            DetailsTab::Status => self.status_content(cx),
            DetailsTab::SpeedLimiter => self.speed_content(cx),
            DetailsTab::Completion => self.completion_content(cx),
        };
        let can_pause = self.can_pause();
        let conflict = self.snapshot.can_retry
            && self
                .snapshot
                .error
                .as_ref()
                .is_some_and(|error| error.to_lowercase().contains("already exists"));
        let tabs = h_flex()
            .debug_selector(|| "status-tabs".into())
            .flex_1()
            .min_w_0()
            .child(
                TabBar::new("details-tabs")
                    .segmented()
                    .selected_index(self.selected_tab_index())
                    .child(Tab::new().label("Status").icon(KitIcon::Download))
                    .child(Tab::new().label("Speed").icon(KitIcon::Gauge))
                    .child(Tab::new().label("Completion").icon(KitIcon::Power))
                    .on_click(cx.listener(|view, index, _, cx| view.select_tab(*index, cx))),
            );
        #[cfg(target_os = "macos")]
        let tabs = gpui_kit::component::TitleBar::new()
            .h(px(48.))
            .bg(cx.theme().background)
            .child(tabs)
            .into_any_element();
        #[cfg(not(target_os = "macos"))]
        let tabs = h_flex().p_4().child(tabs).into_any_element();
        let body = v_flex()
            .p_4()
            .gap_3()
            .flex_1()
            .min_h_0()
            .child(
                v_flex()
                    .id("download-details-content")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .gap_3()
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                Icon::new(status_icon(self.connection_info()))
                                    .text_color(status_color(self.connection_info(), cx)),
                            )
                            .child(
                                Label::new(if self.snapshot.cancelling {
                                    "Cancelling".to_owned()
                                } else {
                                    self.snapshot.status.clone()
                                })
                                .text_color(status_color(self.connection_info(), cx)),
                            )
                            .child(Label::new(if self.snapshot.is_torrent {
                                "Torrent".to_owned()
                            } else {
                                format!("{} connections", self.snapshot.telemetry.connections.len())
                            })),
                    )
                    .child(content)
                    .when(
                        self.show_details && self.tab == DetailsTab::Status,
                        |view| view.child(self.connection_content()),
                    )
                    .when(conflict, |view| {
                        view.child(
                            GroupBox::new()
                                .outline()
                                .title(Label::new("Destination already exists"))
                                .child(Label::new("Choose how to publish the retried download."))
                                .child(
                                    h_flex()
                                        .gap_2()
                                        .when(!self.snapshot.is_torrent, |row| {
                                            row.child(
                                                Button::new("retry-overwrite")
                                                    .label("Override existing")
                                                    .on_click(cx.listener(|view, _, _, cx| {
                                                        if view.snapshot.can_retry
                                                            && !view.snapshot.is_torrent
                                                        {
                                                            cx.emit(RetryDownload {
                                                                id: view.snapshot.id,
                                                                collision:
                                                                    CollisionPolicy::Overwrite,
                                                            });
                                                        }
                                                    })),
                                            )
                                        })
                                        .child(
                                            Button::new("retry-rename")
                                                .label("Add another file")
                                                .on_click(cx.listener(|view, _, _, cx| {
                                                    if view.snapshot.can_retry {
                                                        cx.emit(RetryDownload {
                                                            id: view.snapshot.id,
                                                            collision: CollisionPolicy::Rename,
                                                        });
                                                    }
                                                })),
                                        ),
                                ),
                        )
                    }),
            )
            .child(
                v_flex()
                    .id("status-progress")
                    .debug_selector(|| "status-progress".into())
                    .relative()
                    .flex_shrink_0()
                    .child(
                        Progress::new("details-overall-progress")
                            .accessibility_label("Overall download progress")
                            .loading(
                                self.snapshot.active && !self.snapshot.paused && percent.is_none(),
                            )
                            .value(percent.unwrap_or(0.0))
                            .with_size(Size::Size(px(24.))),
                    )
                    .child(
                        h_flex()
                            .debug_selector(|| "status-percentage".into())
                            .absolute()
                            .inset_0()
                            .items_center()
                            .justify_center()
                            .child(Label::new(
                                percent
                                    .map(|percent| format!("{percent:.1}%"))
                                    .unwrap_or_else(|| "Percentage unknown".into()),
                            )),
                    ),
            )
            .when(self.snapshot.shutdown_armed, |body| {
                body.child(
                    Toolbar::new("shared-completion-warning")
                        .h_auto()
                        .flex_shrink_0()
                        .content(Label::new(match self.snapshot.shutdown_remaining {
                            Some(seconds) => format!("Shutdown in {seconds}s"),
                            None => "Shutdown armed after successful completion".into(),
                        }))
                        .content(
                            Button::new("shared-cancel-shutdown")
                                .debug_selector(|| "shared-cancel-shutdown".into())
                                .icon(KitIcon::Power)
                                .label("Cancel shutdown")
                                .on_click(cx.listener(|view, _, _, cx| {
                                    cx.emit(CancelShutdown(view.snapshot.id))
                                })),
                        ),
                )
            })
            .child(
                Toolbar::new("details-transfer-actions")
                    .h_auto()
                    .flex_shrink_0()
                    .content(
                        Button::new("details-disclosure")
                            .icon(if self.show_details {
                                KitIcon::ChevronUp
                            } else {
                                KitIcon::ChevronDown
                            })
                            .label(if self.show_details {
                                "Hide details"
                            } else {
                                "Show details"
                            })
                            .disabled(self.tab != DetailsTab::Status)
                            .on_click(cx.listener(|view, _, _, cx| {
                                view.show_details = !view.show_details;
                                cx.notify();
                            })),
                    )
                    .content(
                        Button::new("details-pause")
                            .icon(if self.snapshot.paused {
                                KitIcon::Play
                            } else {
                                KitIcon::Pause
                            })
                            .label(if self.snapshot.paused {
                                "Resume"
                            } else {
                                "Pause"
                            })
                            .disabled(!can_pause)
                            .tooltip(if self.snapshot.is_torrent {
                                "Pause or resume while downloading or paused"
                            } else {
                                "Pause requires verified byte ranges and a strong validator"
                            })
                            .on_click(cx.listener(|view, _, _, cx| {
                                if view.can_pause() {
                                    cx.emit(TogglePause(view.snapshot.id));
                                }
                            })),
                    )
                    .content(
                        Button::new("details-cancel")
                            .icon(KitIcon::X)
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
                    )
                    .when(self.snapshot.can_retry && !conflict, |toolbar| {
                        toolbar.content(
                            Button::new("details-retry")
                                .icon(KitIcon::RotateCw)
                                .label("Retry")
                                .on_click(cx.listener(|view, _, _, cx| {
                                    if view.snapshot.can_retry {
                                        cx.emit(RetryDownload {
                                            id: view.snapshot.id,
                                            collision: CollisionPolicy::Reject,
                                        });
                                    }
                                })),
                        )
                    })
                    .content(
                        Button::new("details-close")
                            .label("Close")
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(CloseDetails))),
                    ),
            );
        v_flex()
            .id("download-details")
            .size_full()
            .child(tabs)
            .child(body)
    }
}
#[cfg(test)]
mod tests {
    use super::{DetailsSnapshot, DownloadDetails};
    use ulos_core::DownloadProgress;

    #[test]
    fn status_starts_compact_with_friendly_folder_metadata() {
        let mut view = details(None);
        assert!(!view.show_details);
        view.snapshot.destination = "/Volumes/External/Archives/file.bin".into();
        assert_eq!(view.folder_display(), "Archives");
        view.snapshot.folder_display = "Downloads/Archives".into();
        assert_eq!(view.folder_display(), "Downloads/Archives");
        // Display abbreviations never alter the original destination.
        assert_eq!(
            view.snapshot.destination.to_string_lossy(),
            "/Volumes/External/Archives/file.bin"
        );
    }

    #[test]
    fn source_less_history_is_read_only_when_parent_gates_it() {
        let view = DownloadDetails::new(DetailsSnapshot {
            source: "Source not retained".into(),
            status: "Failed".into(),
            active: false,
            can_retry: false,
            ..DetailsSnapshot::default()
        });
        assert_eq!(view.snapshot.source, "Source not retained");
        assert!(!view.can_configure_connections());
        assert!(!view.snapshot.active);
        assert!(!view.snapshot.can_retry);
    }

    fn torrent_details() -> DownloadDetails {
        let mut view = details(None);
        view.snapshot.is_torrent = true;
        view.snapshot.torrent_fresh = true;
        view.snapshot.source = "Local torrent metadata".into();
        view.snapshot.torrent = Some(ulos_core::torrent::TorrentTelemetry {
            downloaded_bytes: 42,
            total_bytes: 100,
            peers: 3,
            uploaded_bytes: 0,
            phase: ulos_core::torrent::TorrentPhase::Downloading,
            download_bytes_per_second: 2048,
            tracker_count: 2,
            seen_peers: 5,
            ..ulos_core::torrent::TorrentTelemetry::default()
        });
        view
    }

    #[test]
    fn torrent_status_uses_actual_swarm_metrics_not_http_ranges() {
        let mut view = torrent_details();
        assert_eq!(
            view.torrent_fields(),
            vec![
                ("Phase", "Downloading".into()),
                ("Peers", "3".into()),
                ("Peers seen", "5".into()),
                ("No data for", "0s".into()),
                ("Verified bytes", "42 B of 100 B".into()),
                ("Uploaded", "0 B".into()),
            ]
        );
        // Torrent details never fall through to the HTTP table, even without telemetry.
        let _ = view.connection_content();
        view.snapshot.torrent = None;
        assert_eq!(
            view.torrent_fields(),
            vec![("Telemetry", "Not available".into())]
        );
        let _ = view.connection_content();
        assert!(!view.can_pause());
    }

    #[test]
    fn stale_and_terminal_torrents_never_reuse_live_rates_or_publishing_labels() {
        let mut view = torrent_details();
        view.snapshot.torrent_fresh = false;
        let fields = view.torrent_summary_fields();
        for label in ["Peers", "Download speed", "Upload speed"] {
            assert!(
                fields
                    .iter()
                    .find(|(name, _)| *name == label)
                    .unwrap()
                    .1
                    .contains("Not available")
            );
        }
        assert_eq!(
            view.torrent_fields()[0],
            ("Phase", "Waiting for telemetry".into())
        );
        for status in ["Finished", "Failed", "Cancelled"] {
            view.snapshot.active = false;
            view.snapshot.status = status.into();
            view.snapshot.torrent.as_mut().unwrap().phase =
                ulos_core::torrent::TorrentPhase::Publishing;
            assert_eq!(view.torrent_fields()[0], ("Phase", status.into()));
            assert_eq!(view.torrent_fields()[1], ("Peers", "0".into()));
            assert_eq!(
                view.torrent_fields()[3],
                ("No data for", "Not applicable".into())
            );
            assert_eq!(
                view.torrent_summary_fields()[1],
                ("Download speed", "0 B/s".into())
            );
            assert!(view.torrent_summary_fields()[2].1.starts_with("0 B/s"));
        }
    }

    #[gpui::test]
    fn torrent_live_metrics_are_visible_without_expanding_details(
        cx: &mut gpui_kit::TestAppContext,
    ) {
        use gpui_kit::{AppContext as _, component::Root};
        cx.update(gpui_kit::init);
        let mut content = None;
        let (_root, cx) = cx.add_window_view(|window, cx| {
            let view = cx.new(|_| torrent_details());
            content = Some(view.clone());
            Root::new(view, window, cx)
        });
        let content = content.unwrap();
        cx.update(|window, cx| window.draw(cx).clear(cx));
        for selector in [
            "status-metric-peers",
            "status-metric-download-speed",
            "status-metric-upload-speed",
            "status-metric-trackers",
        ] {
            assert!(
                cx.debug_bounds(selector).is_some(),
                "missing always-visible {selector}"
            );
        }
        content.read_with(cx, |view, _| {
            assert!(!view.show_details);
            assert_eq!(
                view.torrent_summary_fields()[1],
                ("Download speed", "2.0 KiB/s".into())
            );
            assert!(view.torrent_summary_fields()[2].1.contains("0 B/s"));
            assert!(view.torrent_summary_fields()[2].1.contains("disabled"));
        });
        content.update(cx, |view, cx| {
            let telemetry = view.snapshot.torrent.as_mut().unwrap();
            telemetry.peers = 0;
            telemetry.seen_peers = 0;
            telemetry.downloaded_bytes = 0;
            telemetry.download_bytes_per_second = 0;
            assert_eq!(telemetry.activity_label(), "Searching for peers");
            assert!(
                telemetry
                    .diagnostic()
                    .unwrap()
                    .contains("no peer candidates")
            );
            telemetry.tracker_count = 0;
            assert_eq!(telemetry.activity_label(), "No peer discovery");
            assert!(telemetry.diagnostic().unwrap().contains("No trackers"));
            cx.notify();
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));
        assert!(cx.debug_bounds("status-metric-peers").is_some());
        content.update(cx, |view, cx| {
            view.snapshot.paused = true;
            view.snapshot.torrent.as_mut().unwrap().phase =
                ulos_core::torrent::TorrentPhase::Paused;
            assert_eq!(
                view.torrent_summary_fields()[1],
                ("Download speed", "0 B/s".into())
            );
            view.snapshot.active = false;
            view.snapshot
                .torrent
                .as_mut()
                .unwrap()
                .download_bytes_per_second = 4096;
            assert_eq!(
                view.torrent_summary_fields()[1],
                ("Download speed", "0 B/s".into())
            );
            cx.notify();
        });
    }

    #[test]
    fn torrent_paused_slot_displays_paused_and_remains_resumable() {
        let mut view = torrent_details();
        assert!(view.can_pause()); // No HTTP resume capability is needed.
        view.snapshot.paused = true;
        view.snapshot.status = "Paused".into();
        view.snapshot.torrent.as_mut().unwrap().phase = ulos_core::torrent::TorrentPhase::Paused;
        assert_eq!(view.connection_info(), "Paused");
        assert_eq!(view.torrent_fields()[0], ("Phase", "Paused".into()));
        assert!(view.can_pause());
        view.snapshot.torrent.as_mut().unwrap().phase =
            ulos_core::torrent::TorrentPhase::Publishing;
        assert!(!view.can_pause());
        view.snapshot.active = false;
        assert!(!view.can_pause());
    }

    #[gpui::test]
    fn torrent_live_controls_emit_no_http_configuration_requests(
        cx: &mut gpui_kit::TestAppContext,
    ) {
        use gpui_kit::AppContext as _;
        use std::{cell::Cell, rc::Rc};
        let events = Rc::new(Cell::new(0));
        let (view, _rate, _connections) = cx.update(|cx| {
            let view = cx.new(|_| torrent_details());
            let count = events.clone();
            let rate = cx.subscribe(&view, move |_, _: &super::SetRateLimit, _| {
                count.set(count.get() + 1);
            });
            let count = events.clone();
            let connections = cx.subscribe(&view, move |_, _: &super::SetConnections, _| {
                count.set(count.get() + 1);
            });
            (view, rate, connections)
        });
        cx.update(|cx| {
            view.update(cx, |view, cx| {
                assert!(!view.can_configure_rate());
                assert!(!view.can_configure_connections());
                view.apply_rate(cx);
                // Unsupported speed settings render without even creating inputs.
                let _ = view.speed_content(cx);
                assert!(view.rate_input.is_none());
                assert!(view.connections_input.is_none());
                view.snapshot.active = false;
                view.snapshot.can_retry = true;
                view.snapshot.status = "Failed".into();
                assert!(!view.can_configure_connections());
                view.apply_connections(cx);
            })
        });
        cx.run_until_parked();
        assert_eq!(events.get(), 0);
    }

    #[test]
    fn torrent_limits_parse_unlimited_and_bounds() {
        assert_eq!(super::parse_optional_rate(" "), Ok(None));
        assert_eq!(
            super::parse_optional_rate("1048576"),
            Ok(Some(1_073_741_824))
        );
        for value in ["0", "-1", "1.5", "1048577"] {
            assert!(super::parse_optional_rate(value).is_err());
        }
        assert_eq!(super::parse_peers("1"), Ok(1));
        assert_eq!(super::parse_peers("10000"), Ok(10000));
        for value in ["", "0", "10001", "1.5"] {
            assert!(super::parse_peers(value).is_err());
        }
    }

    #[gpui::test]
    fn native_torrent_controls_emit_bounded_intents_and_honor_live_upload_telemetry(
        cx: &mut gpui_kit::TestAppContext,
    ) {
        use gpui_kit::{AppContext as _, component::Root};
        use std::{cell::RefCell, rc::Rc};
        cx.update(gpui_kit::init);
        let mut content = None;
        let (_root, cx) = cx.add_window_view(|window, cx| {
            let view = cx.new(|_| {
                let mut view = torrent_details();
                view.snapshot.torrent_settings.allow_upload = true;
                view.snapshot.torrent.as_mut().unwrap().upload_enabled = true;
                view
            });
            content = Some(view.clone());
            Root::new(view, window, cx)
        });
        let view = content.unwrap();
        let rates = Rc::new(RefCell::new(Vec::new()));
        let peers = Rc::new(RefCell::new(Vec::new()));
        let rates_out = rates.clone();
        let peers_out = peers.clone();
        let _rates = cx.update(|_, cx| {
            cx.subscribe(&view, move |_, event: &super::SetTorrentLimits, _| {
                rates_out
                    .borrow_mut()
                    .push((event.id, event.download, event.upload))
            })
        });
        let _peers = cx.update(|_, cx| {
            cx.subscribe(&view, move |_, event: &super::SetTorrentPeers, _| {
                peers_out.borrow_mut().push((event.id, event.peers))
            })
        });
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.ensure_inputs(window, cx);
                assert!(view.can_configure_torrent());
                assert!(!view.torrent_summary_fields()[2].1.contains("disabled"));
                assert!(view.connections_input.is_none());
                view.rate_input
                    .as_ref()
                    .unwrap()
                    .update(cx, |input, cx| input.set_value("1024", window, cx));
                view.torrent_upload_input
                    .as_ref()
                    .unwrap()
                    .update(cx, |input, cx| input.set_value("", window, cx));
                view.torrent_peers_input
                    .as_ref()
                    .unwrap()
                    .update(cx, |input, cx| input.set_value("200", window, cx));
                view.apply_torrent_limits(cx);
                view.apply_torrent_peers(cx);
                view.rate_input
                    .as_ref()
                    .unwrap()
                    .update(cx, |input, cx| input.set_value("0", window, cx));
                view.apply_torrent_limits(cx);
                view.snapshot.cancelling = true;
                view.apply_torrent_peers(cx);
            })
        });
        cx.run_until_parked();
        assert_eq!(rates.borrow().len(), 1);
        assert_eq!(rates.borrow()[0].1, Some(1_048_576));
        assert_eq!(rates.borrow()[0].2, None);
        assert_eq!(peers.borrow().len(), 1);
        assert_eq!(peers.borrow()[0].1, 200);
    }

    #[test]
    fn rate_limit_accepts_only_bounded_positive_whole_kib() {
        assert_eq!(super::parse_rate(" 1 "), Ok(1024));
        assert_eq!(super::parse_rate("1048576"), Ok(1_073_741_824));
        for value in [
            "",
            "0",
            "-1",
            "1.5",
            "NaN",
            "1048577",
            "18446744073709551616",
        ] {
            assert!(super::parse_rate(value).is_err(), "accepted {value}");
        }
    }

    #[test]
    fn connections_accept_only_one_through_shared_maximum() {
        assert_eq!(super::parse_connections(" 1 "), Ok(1));
        assert_eq!(
            super::parse_connections(&super::MAX_CONNECTIONS.to_string()),
            Ok(super::MAX_CONNECTIONS)
        );
        assert!(super::parse_connections(&(super::MAX_CONNECTIONS + 1).to_string()).is_err());
        for value in ["", "0", "-1", "1.5", "infinity"] {
            assert!(super::parse_connections(value).is_err(), "accepted {value}");
        }
    }

    #[test]
    fn next_start_connections_cannot_change_during_transfer_or_after_success() {
        let mut view = details(None);
        assert!(!view.can_configure_connections());
        view.snapshot.active = false;
        view.snapshot.status = "Failed".into();
        view.snapshot.can_retry = true;
        assert!(view.can_configure_connections());
        view.snapshot.status = "Finished".into();
        assert!(!view.can_configure_connections());
    }

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
            ..DetailsSnapshot::default()
        })
    }

    #[gpui::test]
    fn telemetry_updates_preserve_view_state_and_skip_unchanged_notifications(
        cx: &mut gpui_kit::TestAppContext,
    ) {
        use std::{cell::Cell, rc::Rc};

        use gpui_kit::AppContext as _;

        let notifications = Rc::new(Cell::new(0));
        let (view, _subscription) = cx.update(|cx| {
            let view = cx.new(|_| {
                let mut view = details(None);
                view.tab = super::DetailsTab::Completion;
                view.show_details = false;
                view
            });
            let count = notifications.clone();
            let subscription = cx.observe(&view, move |_, _| count.set(count.get() + 1));
            (view, subscription)
        });
        let original = cx.update(|cx| view.read(cx).snapshot.clone());
        cx.update(|cx| view.update(cx, |view, cx| view.update(original.clone(), cx)));
        cx.run_until_parked();
        assert_eq!(notifications.get(), 0);

        let mut changed = original;
        changed.progress = Some(DownloadProgress {
            downloaded_bytes: 50,
            total_bytes: Some(100),
        });
        changed.speed = Some(25.0);
        changed.telemetry.resume = ulos_core::transfer::ResumeCapability::Supported;
        changed.telemetry.paused = true;
        changed.paused = true;
        changed.shutdown_armed = true;
        changed.shutdown_remaining = Some(12);
        cx.update(|cx| view.update(cx, |view, cx| view.update(changed.clone(), cx)));
        cx.run_until_parked();
        assert_eq!(notifications.get(), 1);
        cx.update(|cx| view.update(cx, |view, cx| view.update(changed, cx)));
        cx.run_until_parked();
        assert_eq!(notifications.get(), 1);
        cx.update(|cx| {
            let view = view.read(cx);
            assert!(view.tab == super::DetailsTab::Completion);
            assert!(!view.show_details);
            assert_eq!(view.percentage(), Some(50.0));
            assert!(view.snapshot.paused);
            assert!(view.snapshot.shutdown_armed);
            assert_eq!(view.snapshot.shutdown_remaining, Some(12));
        });
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
    fn finished_empty_download_has_complete_progress() {
        let mut view = details(Some(DownloadProgress {
            downloaded_bytes: 0,
            total_bytes: Some(0),
        }));
        view.snapshot.active = false;
        view.snapshot.status = "Finished".into();
        assert_eq!(view.percentage(), Some(100.0));
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
