# dfui-plot (experiment)

[![crates.io](https://img.shields.io/crates/v/dfui-plot.svg)](https://crates.io/crates/dfui-plot)
[![docs.rs](https://img.shields.io/docsrs/dfui-plot)](https://docs.rs/dfui-plot)
[![examples](https://img.shields.io/badge/examples-live-blue)](https://luisayuso.github.io/dfui/)
[![tests](https://github.com/LuisAyuso/dfui/actions/workflows/test.yml/badge.svg)](https://github.com/LuisAyuso/dfui/actions/workflows/test.yml)

Plot widgets for [dfui](https://github.com/LuisAyuso/dfui) in the spirit of
ggplot2, built only on dfui's public API. dfui itself doesn't depend on it.

A `plot` takes rows of any type and stacks layers. Each layer maps rows to
aesthetics with closures, summarizes them with a `Stat` and draws a geom:

```rust,ignore
plot(&rows)
    .layer(line().x(|r: &Tick| r.t).y(|r| r.v).group(|r| r.series).smooth())
    .layer(bar().x(|r: &Tick| r.day).y(|r| r.vol).stat(Stat::Sum))
    .layer(boxplot().x(|r: &Tick| r.day).y(|r| r.v))
    .layer(candle().x(|r: &Tick| r.t).ohlc(|r| [r.o, r.h, r.l, r.c]))
    .y_label("price")
```

- Geoms: `line` (sharp, or `.smooth()` along a monotone cubic that passes
  through every sample without overshooting), `point`, `bar` (side by side
  per group, optional corner `.radius`), `boxplot`, `candle`.
- Stats: `Identity`, `Mean`, `Sum`, `Count`, `Min`, `Max`, `Box`, over the
  rows that share a group and x.
- Layers: `.name(..)` lists one in the legend, `.filter(..)` keeps some
  rows, so several layers can share one data set; ungrouped layers take
  palette colors in layer order. `vline`/`hline` draw reference lines
  across the plot, named in the legend too.
- Data changes: build the plot from fresh rows on each rebuild, e.g. inside
  `watch(&notifier, ..)`.
- Long lines are thinned to each pixel column's envelope before stroking.

```sh
cargo run -p dfui-plot --release --example plot
```
