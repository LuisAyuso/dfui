use std::rc::Rc;

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use dfui::{App, Bench, Element};
use dfui_plot::{Layer, Stat, bar, boxplot, line, plot};

#[derive(Clone, Copy)]
struct Row {
    t: f64,
    v: f64,
    g: u64,
}

struct One {
    rows: Rc<[Row]>,
    layer: fn() -> Layer<Row>,
}

impl App for One {
    fn view(&self) -> Element {
        plot(&self.rows).layer((self.layer)()).into()
    }
}

fn rows(n: usize) -> Rc<[Row]> {
    (0..n)
        .map(|i| Row {
            t: (i / 2) as f64,
            v: (i as f64 / 50.0).sin() * 3.0 + (i % 7) as f64 * 0.1,
            g: (i % 2) as u64,
        })
        .collect()
}

type Case = (&'static str, fn() -> Layer<Row>);

fn layers() -> [Case; 4] {
    [
        ("line", || line().x(|r: &Row| r.t).y(|r| r.v).group(|r| r.g)),
        ("smooth", || {
            line().x(|r: &Row| r.t).y(|r| r.v).group(|r| r.g).smooth()
        }),
        ("bar_sum", || {
            bar()
                .x(|r: &Row| (r.t / 100.0).floor())
                .y(|r| r.v)
                .stat(Stat::Sum)
        }),
        ("boxplot", || {
            boxplot().x(|r: &Row| (r.t / 1000.0).floor()).y(|r| r.v)
        }),
    ]
}

fn plots(c: &mut Criterion) {
    let mut g = c.benchmark_group("plot");
    for n in [1_000, 10_000, 100_000] {
        for (name, layer) in layers() {
            let mut b = Bench::new(
                One {
                    rows: rows(n),
                    layer,
                },
                (800.0, 500.0),
            );
            g.bench_with_input(BenchmarkId::new(format!("{name}/frame"), n), &n, |bn, _| {
                bn.iter(|| b.frame())
            });
            g.bench_with_input(BenchmarkId::new(format!("{name}/paint"), n), &n, |bn, _| {
                bn.iter(|| b.paint())
            });
        }
    }
}

criterion_group!(benches, plots);
criterion_main!(benches);
