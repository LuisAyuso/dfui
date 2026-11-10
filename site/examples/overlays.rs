use dfui::taffy::AlignItems;
use dfui::widgets::{
    Overlay, Overlays, Shown, Styled, bottom_sheet, button, column, dialog, drawer, dropdown,
    overlays, row, snackbar, stateful, text, tooltip,
};
use dfui::{Color, Element, EventCx, Handle};

fn panel(title: &str, me: Shown) -> Element {
    column([
        text(title).size(20.0).into(),
        button(text("Close").color(Color::WHITE))
            .on_click(move |cx| me.close(cx))
            .into(),
    ])
    .gap(16.0)
    .into()
}

fn content(ov: Handle<Overlays>) -> Element {
    let open = move |label: &str, show: fn(Shown) -> Overlay| -> Element {
        button(text(label).color(Color::WHITE))
            .on_click(move |cx| ov.show(cx, show))
            .into()
    };
    stateful(move |&size: &usize, h| {
        column([
            row([
                open("Dialog", |me| dialog(panel("A dialog", me))),
                open("Drawer", |me| drawer(panel("A drawer", me))),
                open("Sheet", |me| bottom_sheet(panel("A bottom sheet", me))),
                button(text("Delete").color(Color::WHITE))
                    .on_click(move |cx| {
                        let done = move |cx: &mut EventCx| ov.show(cx, |_| snackbar("Deleted"));
                        ov.confirm(cx, "Delete it?", "This can't be undone.", "Delete", done);
                    })
                    .into(),
            ])
            .gap(8.0)
            .wrap()
            .into(),
            row([
                dropdown(["Small", "Medium", "Large"], size)
                    .on_change(move |cx, i| cx.update(h, move |s: &mut usize| *s = i))
                    .into(),
                tooltip(open("Snack", |_| snackbar("Saved")), "Shows a snackbar").into(),
            ])
            .gap(8.0)
            .align(AlignItems::CENTER)
            .into(),
        ])
        .gap(16.0)
    })
    .padding(16.0)
    .into()
}

fn view() -> Element {
    overlays(content).into()
}

fn main() {
    dfui::run(view);
}
