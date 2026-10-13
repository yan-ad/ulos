use gpui_kit::{component::ActiveTheme, *};

struct Ulos;

impl Render for Ulos {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .size_full()
            .p_8()
            .gap_4()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(div().text_2xl().child("Ulos"))
            .child("Desktop starter — downloads are not connected to this window yet.")
            .child("The CLI supports single-stream HTTP/HTTPS downloads.")
            .child("Run: cargo run -p ulos-cli -- <URL> --output <FILE>")
    }
}

fn main() {
    application().with_assets(assets::Assets).run(|cx| {
        init(cx);
        cx.on_window_closed(|cx, _| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                None,
                size(px(780.0), px(440.0)),
                cx,
            ))),
            titlebar: Some(TitlebarOptions {
                title: Some("Ulos".into()),
                ..Default::default()
            }),
            ..Default::default()
        };
        if let Err(error) = open_window(options, cx, |_, cx| cx.new(|_| Ulos)) {
            eprintln!("Could not open the Ulos window: {error}");
            cx.quit();
            return;
        }
        cx.activate(true);
    });
}
