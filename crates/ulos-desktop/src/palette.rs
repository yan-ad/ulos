use gpui_kit::{
    component::{Theme, ThemeMode},
    *,
};

pub fn init(cx: &mut App) {
    apply(ThemeMode::Light, cx);
}

pub fn apply(mode: ThemeMode, cx: &mut App) {
    Theme::change(mode, None, cx);
    Theme::update(cx, |theme| {
        theme.font_size = px(13.);
        theme.radius = px(2.);
        theme.radius_lg = px(2.);
        if mode == ThemeMode::Dark {
            theme.background = rgb(0x17191f).into();
            theme.foreground = rgb(0xe7e9ef).into();
            theme.muted_foreground = rgb(0xaeb5c4).into();
            theme.border = rgb(0x444c5c).into();
            theme.sidebar = rgb(0x1b1e26).into();
            theme.sidebar_foreground = theme.foreground;
            theme.sidebar_border = rgb(0x333947).into();
            theme.sidebar_accent = rgb(0x2a3448).into();
            theme.sidebar_accent_foreground = theme.foreground;
            theme.table = theme.background;
            theme.table_head = rgb(0x222630).into();
            theme.table_head_foreground = theme.muted_foreground;
            theme.table_hover = rgb(0x222a38).into();
            theme.table_active = rgb(0x293549).into();
            theme.table_row_border = rgb(0x2b303c).into();
            theme.status_bar = rgb(0x1b1e26).into();
            theme.primary = rgb(0x79b3ef).into();
            theme.primary_foreground = rgb(0x101927).into();
            theme.primary_hover = rgb(0x91c3f7).into();
            theme.primary_active = rgb(0x659fdd).into();
            theme.progress_bar = theme.primary;
            theme.ring = theme.primary;
        } else {
            theme.background = rgb(0xffffff).into();
            theme.foreground = rgb(0x1b2430).into();
            theme.muted_foreground = rgb(0x465365).into();
            theme.border = rgb(0xced2d7).into();
            theme.sidebar = rgb(0xfafafa).into();
            theme.sidebar_foreground = theme.foreground;
            theme.sidebar_border = theme.border;
            theme.sidebar_accent = rgb(0xe5effb).into();
            theme.sidebar_accent_foreground = theme.foreground;
            theme.table = theme.background;
            theme.table_head = rgb(0xf1f2f4).into();
            theme.table_head_foreground = theme.foreground;
            theme.table_hover = rgb(0xf0f5fb).into();
            theme.table_active = rgb(0xe1edfb).into();
            theme.table_active_border = rgb(0x2465ad).into();
            theme.table_row_border = rgb(0xe5e7eb).into();
            theme.status_bar = rgb(0xf5f6f8).into();
            theme.primary = rgb(0x2465ad).into();
            theme.primary_foreground = rgb(0xffffff).into();
            theme.primary_hover = rgb(0x1f5899).into();
            theme.primary_active = rgb(0x194c86).into();
            theme.progress_bar = theme.primary;
            theme.ring = theme.primary;
        }
        theme.status_bar_border = theme.border;
        theme.button_primary = theme.primary;
        theme.button_primary_foreground = theme.primary_foreground;
        theme.button_primary_hover = theme.primary_hover;
        theme.button_primary_active = theme.primary_active;
    });
}
