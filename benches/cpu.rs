use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use dfui::taffy::AlignItems;
use dfui::widgets::{Styled, button, column, container, row, scroll, text};
use dfui::{App, Bench, Color, Element, Shadow, Text};

const PARAGRAPH: &str = "Shapes are distance fields and glyphs are evaluated from their \
    outlines, so both stay crisp at any size. Resize the window to watch the flex layout \
    reflow. Every frame rebuilds the tree, lays it out with taffy and paints a flat list \
    of instances that the GPU draws in a single call.";

/// A scrolling list of demo-style cards: the common shape of a real screen.
struct Cards(u32);

impl App for Cards {
    type Msg = ();
    fn update(&mut self, _: ()) {}
    fn view(&self) -> Element<()> {
        let cards = (1..=self.0).map(|n| {
            let card: Element<()> = row([
                container(text(n.to_string()).size(20.0).color(Color::WHITE))
                    .bg(Color::hex(0x3b6fe0))
                    .radius(24.0)
                    .width(48.0)
                    .height(48.0)
                    .into(),
                column([
                    text(format!("Card {n}")).size(18.0).into(),
                    text("Rounded box SDF with border and soft shadow.")
                        .size(14.0)
                        .into(),
                ])
                .grow(1.0)
                .into(),
                button(text("×")).on_press(()).into(),
            ])
            .align(AlignItems::CENTER)
            .gap(14.0)
            .padding(14.0)
            .bg(Color::WHITE)
            .radius(14.0)
            .shadow(Shadow {
                color: Color::BLACK.alpha(0.12),
                offset: (0.0, 3.0),
                blur: 8.0,
            })
            .into();
            card.key(n as u64)
        });
        column([scroll(column(cards).gap(12.0)).grow(1.0).into()])
            .padding(20.0)
            .into()
    }
}

const VIEWPORT: (f32, f32) = (400.0, 800.0);

fn text_layout(c: &mut Criterion) {
    let t = Text::default();
    c.bench_function("text/layout_paragraph", |b| {
        b.iter(|| t.layout(PARAGRAPH, 15.0, Some(320.0)))
    });
}

fn frame(c: &mut Criterion) {
    let mut g = c.benchmark_group("frame");
    for n in [10, 100, 1000] {
        let mut bench = Bench::new(Cards(n), VIEWPORT);
        g.bench_with_input(BenchmarkId::new("cards", n), &n, |b, _| {
            b.iter(|| bench.frame())
        });
    }
    g.finish();
}

fn startup(c: &mut Criterion) {
    c.bench_function("startup/cards_10", |b| {
        b.iter(|| Bench::new(Cards(10), VIEWPORT))
    });
}

criterion_group!(benches, text_layout, frame, startup);
criterion_main!(benches);
