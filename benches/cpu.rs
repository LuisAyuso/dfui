use criterion::{BatchSize, BenchmarkId, Criterion, criterion_group, criterion_main};
use dfui::{Bench, Text};

mod common;
use common::Cards;

const PARAGRAPH: &str = "Shapes are distance fields and glyphs are evaluated from their \
    outlines, so both stay crisp at any size. Resize the window to watch the flex layout \
    reflow. Every frame rebuilds the tree, lays it out with taffy and paints a flat list \
    of instances that the GPU draws in a single call.";

const VIEWPORT: (f32, f32) = (400.0, 800.0);

fn text_layout(c: &mut Criterion) {
    let t = Text::default();
    c.bench_function("text/layout_paragraph", |b| {
        // Two frame ends evict everything, so each layout is a cache miss.
        let evict = || {
            t.end_frame();
            t.end_frame();
        };
        b.iter_batched(
            evict,
            |_| t.layout(PARAGRAPH, 15.0, Some(320.0)),
            BatchSize::PerIteration,
        )
    });
    c.bench_function("text/layout_paragraph_cached", |b| {
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
