use gpui_kit::{
    component::{Theme, ThemeMode},
    *,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ThemePreference {
    #[default]
    System,
    Light,
    Dark,
}

impl ThemePreference {
    pub const ALL: [Self; 3] = [Self::System, Self::Light, Self::Dark];

    pub fn label(self) -> &'static str {
        match self {
            Self::System => "Follow system",
            Self::Light => "Light",
            Self::Dark => "Dark",
        }
    }

    fn resolve(self, appearance: WindowAppearance) -> ThemeMode {
        match self {
            Self::System => appearance.into(),
            Self::Light => ThemeMode::Light,
            Self::Dark => ThemeMode::Dark,
        }
    }
}

#[derive(Default)]
struct ThemeSettings {
    preference: ThemePreference,
}
impl Global for ThemeSettings {}

pub fn init(cx: &mut App) {
    choose(ThemePreference::System, cx);
}

pub fn preference(cx: &App) -> ThemePreference {
    cx.try_global::<ThemeSettings>()
        .map(|settings| settings.preference)
        .unwrap_or_default()
}

pub fn choose(preference: ThemePreference, cx: &mut App) {
    cx.set_global(ThemeSettings { preference });
    // Clear a previous application override before resolving System appearance.
    cx.set_window_appearance(match preference {
        ThemePreference::System => None,
        ThemePreference::Light => Some(WindowAppearance::Light),
        ThemePreference::Dark => Some(WindowAppearance::Dark),
    });
    Theme::change(preference.resolve(cx.window_appearance()), None, cx);
}

#[cfg(test)]
pub fn apply(mode: ThemeMode, cx: &mut App) {
    choose(
        if mode.is_dark() {
            ThemePreference::Dark
        } else {
            ThemePreference::Light
        },
        cx,
    );
}

pub(crate) fn appearance_changed(appearance: WindowAppearance, cx: &mut App) {
    if preference(cx) == ThemePreference::System {
        let mode = ThemePreference::System.resolve(appearance);
        if Theme::global(cx).mode != mode {
            Theme::change(mode, None, cx);
        }
    }
}

pub fn observe_window(window: &mut Window, cx: &mut App) -> Subscription {
    appearance_changed(window.appearance(), cx);
    window.observe_window_appearance(|window, cx| appearance_changed(window.appearance(), cx))
}

pub fn observe_preference(cx: &mut App, callback: impl FnMut(&mut App) + 'static) -> Subscription {
    cx.observe_global::<ThemeSettings>(callback)
}

#[cfg(test)]
mod tests {
    use super::{ThemePreference, appearance_changed, choose, init, preference};
    use gpui_kit::{
        WindowAppearance,
        component::{Theme, ThemeMode},
    };

    #[test]
    fn system_resolves_light_dark_and_vibrant_appearances() {
        for (appearance, expected) in [
            (WindowAppearance::Light, ThemeMode::Light),
            (WindowAppearance::VibrantLight, ThemeMode::Light),
            (WindowAppearance::Dark, ThemeMode::Dark),
            (WindowAppearance::VibrantDark, ThemeMode::Dark),
        ] {
            assert_eq!(ThemePreference::System.resolve(appearance), expected);
            assert_eq!(ThemePreference::Light.resolve(appearance), ThemeMode::Light);
            assert_eq!(ThemePreference::Dark.resolve(appearance), ThemeMode::Dark);
        }
    }

    #[gpui::test]
    fn system_tracks_changes_but_manual_modes_remain_fixed(cx: &mut gpui_kit::TestAppContext) {
        cx.update(gpui_kit::init);
        cx.update(init);
        cx.update(|cx| {
            assert_eq!(preference(cx), ThemePreference::System);
            appearance_changed(WindowAppearance::Dark, cx);
            assert_eq!(Theme::global(cx).mode, ThemeMode::Dark);
            assert_eq!(preference(cx), ThemePreference::System);
            appearance_changed(WindowAppearance::Light, cx);
            assert_eq!(Theme::global(cx).mode, ThemeMode::Light);
            choose(ThemePreference::Dark, cx);
            appearance_changed(WindowAppearance::Light, cx);
            assert_eq!(Theme::global(cx).mode, ThemeMode::Dark);
            choose(ThemePreference::Light, cx);
            appearance_changed(WindowAppearance::Dark, cx);
            assert_eq!(Theme::global(cx).mode, ThemeMode::Light);
            choose(ThemePreference::System, cx);
            assert_eq!(preference(cx), ThemePreference::System);
            appearance_changed(WindowAppearance::Dark, cx);
            assert_eq!(Theme::global(cx).mode, ThemeMode::Dark);
        });
    }
}
