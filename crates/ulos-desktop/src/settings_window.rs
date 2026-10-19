//! A window-local draft. Only a successful explicit Save publishes settings.
use gpui_kit::{component::Disableable, prelude::FluentBuilder};

use crate::config::{ConfigSnapshot, LoadedSettings, ThemePreference};
use gpui_kit::{
    component::{
        ActiveTheme,
        button::Button,
        checkbox::Checkbox,
        h_flex,
        input::{Input, InputState},
        label::Label,
        setting::{SettingField, SettingGroup, SettingItem, SettingPage, Settings},
        v_flex,
    },
    *,
};

pub struct SettingsSaved {
    pub loaded: LoadedSettings,
    pub clipboard_enabled: bool,
}
pub struct SettingsDismiss;
pub struct SettingsWindow {
    baseline: LoadedSettings,
    pub(crate) busy: bool,
    theme: ThemePreference,
    directory: Entity<InputState>,
    connections: Entity<InputState>,
    http_rate: Entity<InputState>,
    down_rate: Entity<InputState>,
    up_rate: Entity<InputState>,
    peers: Entity<InputState>,
    dht: bool,
    dht_confirmed: bool,
    clipboard_enabled: bool,
    error: Option<String>,
    runtime: tokio::runtime::Handle,
    _save: Option<Task<()>>,
}
impl EventEmitter<SettingsSaved> for SettingsWindow {}
impl EventEmitter<SettingsDismiss> for SettingsWindow {}
impl SettingsWindow {
    pub fn new(
        baseline: LoadedSettings,
        clipboard_enabled: bool,
        runtime: tokio::runtime::Handle,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let c = &baseline.snapshot;
        let mut input =
            |value: String| cx.new(|cx| InputState::new(window, cx).default_value(value));
        Self {
            theme: c.theme,
            directory: input(
                c.default_directory
                    .as_ref()
                    .map(|p| p.to_string_lossy().into_owned())
                    .unwrap_or_default(),
            ),
            connections: input(c.http_connections.to_string()),
            http_rate: input(rate_text(c.http_rate_limit)),
            down_rate: input(rate_text(c.torrent_download_limit)),
            up_rate: input(rate_text(c.torrent_upload_limit)),
            peers: input(c.torrent_peer_limit.to_string()),
            dht: c.dht_enabled,
            dht_confirmed: false,
            clipboard_enabled,
            error: baseline.error.clone(),
            baseline,
            runtime,
            busy: false,
            _save: None,
        }
    }
    pub(crate) fn accepts_saved(&self, event: &SettingsSaved) -> bool {
        !self.busy
            && self.baseline.snapshot == event.loaded.snapshot
            && self.baseline.path == event.loaded.path
            && event.loaded.error.is_none()
            && self.clipboard_enabled == event.clipboard_enabled
    }

    fn draft(&self, cx: &App) -> Result<ConfigSnapshot, String> {
        let directory = self.directory.read(cx).value();
        let c = ConfigSnapshot {
            version: 1,
            theme: self.theme,
            default_directory: (!directory.trim().is_empty())
                .then(|| std::path::PathBuf::from(directory.trim())),
            http_connections: self
                .connections
                .read(cx)
                .value()
                .trim()
                .parse()
                .map_err(|_| "HTTP connections must be a whole number.".to_string())?,
            http_rate_limit: parse_rate(self.http_rate.read(cx).value().as_ref())?,
            torrent_download_limit: parse_rate(self.down_rate.read(cx).value().as_ref())?,
            torrent_upload_limit: parse_rate(self.up_rate.read(cx).value().as_ref())?,
            torrent_peer_limit: self
                .peers
                .read(cx)
                .value()
                .trim()
                .parse()
                .map_err(|_| "Peer limit must be a whole number.".to_string())?,
            dht_enabled: self.dht,
        };
        c.validate()?;
        if c.dht_enabled && !self.dht_confirmed {
            return Err(
                "Confirm the public-DHT warning before saving an enabled preference.".into(),
            );
        }
        Ok(c)
    }
    fn save(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let snapshot = match self.draft(cx) {
            Ok(c) => c,
            Err(e) => {
                self.error = Some(e);
                cx.notify();
                return;
            }
        };
        let baseline = self.baseline.clone();
        let runtime = self.runtime.clone();
        self.busy = true;
        self.error = None;
        self._save = Some(cx.spawn(async move |view, cx| {
            let result = runtime
                .spawn_blocking(move || baseline.save(snapshot))
                .await
                .unwrap_or_else(|_| Err("Settings worker stopped unexpectedly.".into()));
            let _ = view.update(cx, |view, cx| {
                view.busy = false;
                match result {
                    Ok(loaded) => {
                        view.baseline = loaded.clone();
                        cx.emit(SettingsSaved {
                            loaded,
                            clipboard_enabled: view.clipboard_enabled,
                        });
                    }
                    Err(error) => view.error = Some(error),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }
    fn input_item(title: &'static str, input: &Entity<InputState>, disabled: bool) -> SettingItem {
        let input = input.clone();
        SettingItem::new(
            title,
            SettingField::render(move |_, _, _| Input::new(&input).disabled(disabled)),
        )
        .layout(Axis::Vertical)
    }
}

fn rate_text(value: Option<u64>) -> String {
    value.map(|v| v.to_string()).unwrap_or_default()
}
fn parse_rate(text: &str) -> Result<Option<u64>, String> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    text.parse()
        .map(Some)
        .map_err(|_| "Rate limits must be whole bytes per second, or blank for unlimited.".into())
}
impl Render for SettingsWindow {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let busy = self.busy;
        let owner = cx.entity().downgrade();
        let theme = self.theme;
        let theme_item = SettingItem::new(
            "Theme",
            SettingField::render(move |_, _, _| {
                let mut row = h_flex().gap_2();
                for (value, label) in [
                    (ThemePreference::System, "System"),
                    (ThemePreference::Light, "Light"),
                    (ThemePreference::Dark, "Dark"),
                ] {
                    let owner = owner.clone();
                    row = row.child(
                        Button::new(label)
                            .label(label)
                            .disabled(busy || theme == value)
                            .on_click(move |_, _, cx| {
                                let _ = owner.update(cx, |view, cx| {
                                    view.theme = value;
                                    cx.notify();
                                });
                            }),
                    );
                }
                row
            }),
        );
        let owner = cx.weak_entity();
        let dht = self.dht;
        let dht_item=SettingItem::new("Public DHT preference",SettingField::render(move|_,_,_|{let owner=owner.clone(); Checkbox::new("settings-dht").label("Use public DHT after per-download upload consent").checked(dht).disabled(busy).on_click(move|checked,_,cx|{let _=owner.update(cx,|view,cx|{view.dht = *checked;view.dht_confirmed=false;cx.notify();});})})).description("A preference is not network or upload consent. Each new download still requires review and upload consent; the visible DHT checkbox can be switched off before Start. Private torrents never use DHT.");
        let owner = cx.weak_entity();
        let confirmed = self.dht_confirmed;
        let confirm_item = SettingItem::new(
            "DHT warning",
            SettingField::render(move |_, _, _| {
                let owner = owner.clone();
                Checkbox::new("settings-dht-confirm")
                    .label("I understand public DHT exposes my IP to peers")
                    .checked(confirmed)
                    .disabled(busy || !dht)
                    .on_click(move |checked, _, cx| {
                        let _ = owner.update(cx, |view, cx| {
                            view.dht_confirmed = *checked;
                            cx.notify();
                        });
                    })
            }),
        );
        let owner = cx.weak_entity();
        let clipboard = self.clipboard_enabled;
        let clipboard_item=SettingItem::new("Clipboard — this session only",SettingField::render(move|_,_,_|{let owner=owner.clone();Checkbox::new("settings-clipboard").label("Detect copied HTTP / HTTPS links").checked(clipboard).disabled(busy).on_click(move|checked,_,cx|{let _=owner.update(cx,|view,cx|{view.clipboard_enabled = *checked;cx.notify();});})})).description("Not saved in ulos.conf. Off on every launch. Only while the main window is active; confirmation only, never automatic downloads. Apply with Save.");
        v_flex().size_full().bg(cx.theme().background).gap_2().p_4()
      .child(Label::new("Settings apply to new download drafts only. Existing downloads are unchanged. Blank limits mean unlimited; byte-rate defaults round up to whole KiB/s in torrent reviews. Upload consent is never saved."))
      .child(v_flex().flex_1().min_h_0().child(Settings::new("ulos-settings").pages([
        SettingPage::new("General").default_open(true).group(SettingGroup::new().items(vec![theme_item,Self::input_item("Default directory (absolute existing folder; blank uses system default)",&self.directory,busy),clipboard_item])),
        SettingPage::new("HTTP").group(SettingGroup::new().items(vec![Self::input_item("Default connections (1–16)",&self.connections,busy),Self::input_item("Default rate limit (bytes/second)",&self.http_rate,busy)])),
        SettingPage::new("Torrents").group(SettingGroup::new().items(vec![Self::input_item("Download rate limit (bytes/second)",&self.down_rate,busy),Self::input_item("Upload rate limit (bytes/second)",&self.up_rate,busy),Self::input_item("Peer limit (1–10,000)",&self.peers,busy),dht_item,confirm_item]))
      ])))
      .when_some(self.error.clone(),|layout,error|layout.child(Label::new(error)))
      .child(h_flex().gap_2().child(Button::new("settings-save").debug_selector(|| "settings-save".into()).label(if busy{"Saving…"}else{"Save"}).disabled(busy||!self.baseline.can_save()).on_click(cx.listener(|view, _, window, cx| view.save(window, cx)))).child(Button::new("settings-cancel").debug_selector(|| "settings-cancel".into()).label("Cancel").disabled(busy).on_click(cx.listener(|_,_,_,cx|cx.emit(SettingsDismiss)))))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[::core::prelude::v1::test]
    fn rates() {
        assert_eq!(parse_rate("").unwrap(), None);
        assert_eq!(parse_rate(" 1024 ").unwrap(), Some(1024));
        assert!(parse_rate("-1").is_err());
        assert!(parse_rate("1.5").is_err());
    }
}
