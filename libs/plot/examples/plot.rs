//! Live plots fed from a background thread: `cargo run -p dfui-plot
//! --release --example plot`.

use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use dfui::widgets::{Styled, column, row, watch};
use dfui::{App, Element, Notifier, Role};
use dfui_plot::{Stat, bar, boxplot, candle, line, plot, point};

#[derive(Clone, Copy)]
struct Tick {
    t: f64,
    v: f64,
    series: u64,
}

/// One bucket of ticks as open, high, low, close.
struct Ohlc {
    bucket: f64,
    v: [f64; 4],
}

const BUCKET: f64 = 20.0;

struct Plots {
    ticks: Arc<Mutex<Vec<Tick>>>,
    notifier: Notifier,
}

impl App for Plots {
    fn view(&self) -> Element {
        let ticks = self.ticks.clone();
        watch(&self.notifier, move || {
            let data: Rc<[Tick]> = ticks.lock().unwrap().as_slice().into();
            let candles = ohlc(&data);
            let bucket = |r: &Tick| (r.t / BUCKET).floor();
            let series = |s: u64, name: &str| {
                line()
                    .x(|r: &Tick| r.t)
                    .y(|r| r.v)
                    .filter(move |r| r.series == s)
                    .name(name)
            };
            let sharp = plot(&data)
                .layer(series(0, "fast"))
                .layer(series(1, "slow"))
                .hline("zero", 0.0, Role::Muted)
                .y_label("sharp");
            let smooth = plot(&data)
                .layer(
                    line()
                        .x(|r: &Tick| r.t)
                        .y(|r| r.v)
                        .group(|r| r.series)
                        .smooth(),
                )
                .y_label("smooth");
            let bars = plot(&data)
                .layer(
                    bar()
                        .x(bucket)
                        .y(|r| r.v)
                        .group(|r| r.series)
                        .stat(Stat::Mean)
                        .radius(3.0),
                )
                .y_label("mean per bucket");
            let boxes = plot(&data)
                .layer(boxplot().x(bucket).y(|r: &Tick| r.v))
                .layer(line().x(bucket).y(|r: &Tick| r.v).stat(Stat::Mean).smooth())
                .y_label("spread per bucket")
                .x_label("bucket");
            let candles = plot(&candles)
                .layer(candle().x(|c: &Ohlc| c.bucket).ohlc(|c| c.v))
                .layer(point().x(|c: &Ohlc| c.bucket).y(|c| c.v[3]).radius(2.0))
                .y_label("ohlc")
                .x_label("bucket");
            let cell = |e: Element| column([e]).padding(12.0).fill().into();
            column([
                row([cell(sharp.into()), cell(smooth.into())]).fill().into(),
                row([cell(bars.into()), cell(boxes.into()), cell(candles.into())])
                    .fill()
                    .into(),
            ])
            .fill()
        })
        .into()
    }

    fn title(&self) -> String {
        "dfui plots".into()
    }
}

/// Series 0's ticks per bucket as candles.
fn ohlc(data: &[Tick]) -> Rc<[Ohlc]> {
    let s0: Vec<Tick> = data.iter().filter(|t| t.series == 0).copied().collect();
    s0.chunk_by(|a, b| (a.t / BUCKET).floor() == (b.t / BUCKET).floor())
        .map(|c| Ohlc {
            bucket: (c[0].t / BUCKET).floor(),
            v: [
                c[0].v,
                c.iter().map(|t| t.v).fold(f64::MIN, f64::max),
                c.iter().map(|t| t.v).fold(f64::MAX, f64::min),
                c[c.len() - 1].v,
            ],
        })
        .collect()
}

fn main() {
    let ticks = Arc::new(Mutex::new(Vec::new()));
    let notifier = Notifier::new();
    let (shared, n) = (ticks.clone(), notifier.clone());
    std::thread::spawn(move || {
        let mut seed = 0x2545_f491_u64;
        let mut noise = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % 1000) as f64 / 1000.0 - 0.5
        };
        for t in (0..).map(f64::from) {
            {
                let mut v = shared.lock().unwrap();
                v.push(Tick {
                    t,
                    v: (t / 15.0).sin() * 3.0 + noise(),
                    series: 0,
                });
                v.push(Tick {
                    t,
                    v: (t / 40.0).cos() * 2.0 + 1.0 + noise() * 0.5,
                    series: 1,
                });
                let excess = v.len().saturating_sub(400);
                v.drain(..excess);
            }
            n.notify();
            std::thread::sleep(Duration::from_millis(50));
        }
    });
    dfui::run(Plots { ticks, notifier });
}
