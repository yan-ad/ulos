use gpui_kit::{
    component::{Theme, ThemeMode},
    *,
};

pub fn init(cx: &mut App) {
    apply(ThemeMode::Light, cx);
}

pub fn apply(mode: ThemeMode, cx: &mut App) {
    Theme::change(mode, None, cx);
}
