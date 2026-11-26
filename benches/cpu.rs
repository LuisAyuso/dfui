use criterion::{BatchSize, BenchmarkId, Criterion, criterion_group, criterion_main};
use dfui::{App, Bench, Event, Font, Key, Text};

mod common;
use common::{Cards, Rows, Table, Typing};

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
            |_| t.layout(PARAGRAPH, Font::DEFAULT, 15.0, Some(320.0)),
            BatchSize::PerIteration,
        )
    });
    // A label that changes every frame: only one word is new.
    let mut n = 0u64;
    let changing = Text::default();
    c.bench_function("text/layout_changing_word", |b| {
        b.iter(|| {
            n += 1;
            changing.end_frame();
            changing.layout(
                &format!("The quick brown fox jumps over the lazy dog {n} times today"),
                Font::DEFAULT,
                16.0,
                Some(300.0),
            )
        })
    });
    c.bench_function("text/layout_paragraph_cached", |b| {
        b.iter(|| t.layout(PARAGRAPH, Font::DEFAULT, 15.0, Some(320.0)))
    });
}

fn frame(c: &mut Criterion) {
    let mut g = c.benchmark_group("frame");
    for n in [10, 100, 1000] {
        let mut bench = Bench::new(Cards::new(n), VIEWPORT);
        g.bench_with_input(BenchmarkId::new("cards", n), &n, |b, _| {
            b.iter(|| bench.frame())
        });
    }
    // One label changes: a rebuild where most of the tree is unchanged.
    for n in [10, 100, 1000] {
        let mut bench = Bench::new(Cards::new(n), VIEWPORT);
        g.bench_with_input(BenchmarkId::new("edit", n), &n, |b, _| {
            b.iter(|| bench.update(Cards::edit))
        });
    }
    // A wheel tick: no rebuild, so taffy's result is reused.
    for n in [10, 100, 1000] {
        let mut bench = Bench::new(Cards::new(n), VIEWPORT);
        let mut dy = 40.0;
        g.bench_with_input(BenchmarkId::new("scroll", n), &n, |b, _| {
            b.iter(|| {
                dy = -dy;
                bench.event(Event::Wheel {
                    x: 200.0,
                    y: 400.0,
                    dx: 0.0,
                    dy,
                });
                bench.paint()
            })
        });
    }
    // Hover over a gap between cards: dispatch only, nothing changes.
    for n in [10, 100, 1000] {
        let mut bench = Bench::new(Cards::new(n), VIEWPORT);
        g.bench_with_input(BenchmarkId::new("pointer_move", n), &n, |b, _| {
            b.iter(|| bench.event(Event::PointerMove { x: 10.0, y: 10.0 }))
        });
    }
    g.finish();
}

/// Wheel ticks down for 4000px, then back up: rows are built and dropped
/// as the list scrolls past them, so some ticks rebuild.
fn scroll_ticks<A: App>(c: &mut Criterion, name: &str, n: u32, app: A) {
    let mut bench = Bench::new(app, VIEWPORT);
    let mut tick = 0u32;
    c.bench_with_input(BenchmarkId::new(name, n), &n, |b, _| {
        b.iter(|| {
            tick += 1;
            let dy = if (tick / 100).is_multiple_of(2) { 40.0 } else { -40.0 };
            bench.event(Event::Wheel {
                x: 200.0,
                y: 400.0,
                dx: 0.0,
                dy,
            });
            bench.paint()
        })
    });
}

fn list(c: &mut Criterion) {
    for n in [1000, 100_000] {
        let mut bench = Bench::new(Rows::new(n), VIEWPORT);
        c.bench_with_input(BenchmarkId::new("list/frame", n), &n, |b, _| {
            b.iter(|| bench.frame())
        });
        scroll_ticks(c, "list/scroll", n, Rows::new(n));
    }
}

fn table(c: &mut Criterion) {
    // Every rebuild sorts the whole model.
    for n in [1000, 100_000] {
        let mut bench = Bench::new(Table::new(n), VIEWPORT);
        c.bench_with_input(BenchmarkId::new("table/frame", n), &n, |b, _| {
            b.iter(|| bench.frame())
        });
        let mut bench = Bench::new(Table::new(n), VIEWPORT);
        let (x, y) = Table::HEADER;
        bench.event(Event::PointerDown { x, y });
        bench.event(Event::PointerUp { x, y });
        c.bench_with_input(BenchmarkId::new("table/frame_sorted", n), &n, |b, _| {
            b.iter(|| bench.frame())
        });
        scroll_ticks(c, "table/scroll", n, Table::new(n));
    }
}

/// One keystroke into a focused field: typed then erased, alternately.
fn typing(c: &mut Criterion) {
    for n in [0, 100] {
        let mut bench = Bench::new(Typing::new(n), VIEWPORT);
        let (x, y) = Typing::FIELD;
        bench.event(Event::PointerDown { x, y });
        bench.event(Event::PointerUp { x, y });
        let before = bench.paint();
        bench.event(Event::Text("a".into()));
        assert_ne!(bench.paint(), before, "the field took the keystroke");
        let mut typed = true;
        c.bench_with_input(BenchmarkId::new("typing/cards", n), &n, |b, _| {
            b.iter(|| {
                typed = !typed;
                bench.event(match typed {
                    true => Event::Text("a".into()),
                    false => Event::Key(Key::Backspace),
                });
                bench.paint()
            })
        });
    }
}

fn startup(c: &mut Criterion) {
    c.bench_function("startup/cards_10", |b| {
        b.iter(|| Bench::new(Cards::new(10), VIEWPORT))
    });
}

criterion_group!(benches, text_layout, frame, list, table, typing, startup);
criterion_main!(benches);
