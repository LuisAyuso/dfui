use criterion::{Criterion, criterion_group, criterion_main};
use dfui::{GpuBench, Rect};

mod common;
use common::{Cards, TextWall};

/// Phone-like target: 1080x2400 physical px at 2.625 scale.
const SIZE: (u32, u32) = (1080, 2400);
const SCALE: f32 = 2.625;

/// Reports GPU time per frame (timestamp queries), not CPU wall time.
fn gpu_time(c: &mut Criterion, name: &str, mut g: GpuBench, damage: Option<Rect>) {
    c.bench_function(name, |b| {
        b.iter_custom(|iters| (0..iters).map(|_| g.draw(damage)).sum())
    });
}

fn frames(c: &mut Criterion) {
    gpu_time(
        c,
        "gpu/cards_10",
        GpuBench::new(Cards(10), SIZE, SCALE),
        None,
    );
    gpu_time(
        c,
        "gpu/text_wall",
        GpuBench::new(TextWall, SIZE, SCALE),
        None,
    );
    // The FPS meter's area on the Bench screen: what a partial frame redraws.
    let meter = Rect::new(16.0, 16.0, 140.0, 28.0);
    gpu_time(
        c,
        "gpu/text_wall_partial",
        GpuBench::new(TextWall, SIZE, SCALE),
        Some(meter),
    );
}

criterion_group!(benches, frames);
criterion_main!(benches);
