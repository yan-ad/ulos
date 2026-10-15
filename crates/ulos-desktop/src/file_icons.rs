use gpui_kit::{
    App, Hsla, Styled,
    assets::IconName,
    component::{ActiveTheme, Icon},
    rgb,
};

use crate::queue::Category;

pub fn navigation_color(cx: &App) -> Hsla {
    rgb(if cx.theme().is_dark() {
        0xfb923c
    } else {
        0xc2410c
    })
    .into()
}

pub fn category_color(category: Category, cx: &App) -> Hsla {
    let (light, dark) = match category {
        Category::Image => (0x0f766e, 0x2dd4bf),
        Category::Music => (0xbe185d, 0xf472b6),
        Category::Video => (0x7e22ce, 0xc084fc),
        Category::Apps => (0x1d4ed8, 0x60a5fa),
        Category::Document => (0x15803d, 0x4ade80),
        Category::Compressed => (0xa16207, 0xfbbf24),
        Category::Other => (0x475569, 0x94a3b8),
    };
    rgb(if cx.theme().is_dark() { dark } else { light }).into()
}

pub fn category_icon(category: Category, cx: &App) -> Icon {
    let glyph = match category {
        Category::Image => IconName::Image,
        Category::Music => IconName::Music,
        Category::Video => IconName::Film,
        Category::Apps => IconName::Package,
        Category::Document => IconName::FileText,
        Category::Compressed => IconName::Archive,
        Category::Other => IconName::File,
    };
    Icon::new(glyph).text_color(category_color(category, cx))
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::{
        Rgba,
        component::{Theme, ThemeMode},
    };

    fn luminance(color: Hsla) -> f64 {
        let color: Rgba = color.into();
        let linear = |channel: f32| {
            let value = f64::from(channel);
            if value <= 0.04045 {
                value / 12.92
            } else {
                ((value + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * linear(color.r) + 0.7152 * linear(color.g) + 0.0722 * linear(color.b)
    }

    fn contrast(first: Hsla, second: Hsla) -> f64 {
        let first = luminance(first);
        let second = luminance(second);
        (first.max(second) + 0.05) / (first.min(second) + 0.05)
    }

    #[gpui::test]
    fn icon_colors_are_distinct_and_contrast_with_stock_surfaces(
        cx: &mut gpui_kit::TestAppContext,
    ) {
        cx.update(gpui_kit::init);
        for mode in [ThemeMode::Light, ThemeMode::Dark] {
            cx.update(|cx| {
                Theme::change(mode, None, cx);
                let colors: Vec<_> = Category::ALL
                    .into_iter()
                    .map(|category| category_color(category, cx))
                    .collect();
                let navigation = navigation_color(cx);
                for (index, color) in colors.iter().enumerate() {
                    assert_ne!(*color, navigation);
                    assert!(!colors[..index].contains(color));
                }
                let theme = cx.theme();
                for color in colors.into_iter().chain([navigation]) {
                    for surface in [
                        theme.background,
                        theme.sidebar,
                        theme.sidebar_accent,
                        theme.accent,
                        theme.secondary,
                        theme.table_even,
                    ] {
                        assert!(
                            contrast(color, surface) >= 3.0,
                            "icon contrast below 3:1 in {mode:?}: {color:?} on {surface:?}"
                        );
                    }
                }
            });
        }
    }
}
