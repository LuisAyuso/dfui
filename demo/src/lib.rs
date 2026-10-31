use std::time::{Duration, Instant};

use dfui::anim::{Lerp, Morph, ease};
use dfui::taffy::{AlignItems, Dimension, JustifyContent, Size, Style};
use dfui::widgets::{
    Snapshot, Styled, button, column, container, future_builder, image, provide, radio, row,
    scroll, segmented, slider, spacer, stateful, stream_builder, tabs, text, text_input, toggle,
};
use dfui::{
    Animated, App, Bitmap, Color, Element, EventCx, Font, Handle, PaintCx, Path, Rect, Scope,
    Shadow, Widget,
};
use futures_channel::{mpsc, oneshot};

static BOLD: Font = Font(include_bytes!("../../assets/DejaVuSans-Bold.ttf"));

const BG: Color = Color::hex(0xf2f3f7);
const INK: Color = Color::hex(0x1d2030);
const MUTED: Color = Color::hex(0x6b7080);
const ACCENT: Color = Color::hex(0x3b6fe0);
const CHIPS: [u32; 5] = [0xe25c5c, 0xe0a23b, 0x4cae6e, 0x3b9ee0, 0x8a5ce2];

/// The app owns external services and hands them to the UI through providers.
#[derive(Default)]
pub struct Demo {
    jobs: Jobs,
}

impl App for Demo {
    fn view(&self) -> Element {
        // Grows to the window so each page's scroll is bounded.
        provide(self.jobs.clone(), stateful(Screen::view).grow(1.0)).into()
    }
}

/// App-wide UI state, owned by the root `stateful`.
struct Screen {
    count: i32,
    items: Vec<u32>,
    bench: bool,
    playing: bool,
    tab: usize,
}

impl Default for Screen {
    fn default() -> Self {
        Screen {
            count: 0,
            items: (1..=12).collect(),
            bench: false,
            playing: true,
            tab: 0,
        }
    }
}

/// Handler that applies `f` to the state behind `h`.
fn on<S: Default + 'static>(
    h: Handle<S>,
    f: impl Fn(&mut S) + Copy + 'static,
) -> impl Fn(&mut EventCx) + 'static {
    move |cx| cx.update(h, f)
}

const PAGES: [&str; 6] = ["Basics", "Controls", "Motion", "Cards", "Async", "Images"];

impl Screen {
    fn view(&self, h: Handle<Screen>) -> Element {
        if self.bench {
            return bench(h);
        }
        let header = row([
            text("dfui").font(BOLD).size(28.0).color(INK).into(),
            spacer().grow(1.0).into(),
            button(text("Bench").color(INK))
                .on_click(on(h, |s| s.bench = true))
                .color(Color::hex(0xe8e9ef))
                .into(),
        ])
        .align(AlignItems::CENTER)
        .gap(12.0);

        let page = match self.tab {
            0 => self.basics(h),
            1 => controls(),
            2 => self.motion(h),
            3 => self.cards(h),
            4 => jobs(),
            _ => gallery(),
        };
        let mut body = scroll(container(page).padding(4.0)).grow(1.0);
        // Zero basis, so the page's height doesn't push the screen past the window.
        body.style_mut().flex_basis = Dimension::length(0.0);
        column([
            header.into(),
            tabs(PAGES, self.tab)
                .accent(ACCENT)
                .on_change(move |cx, i| cx.update(h, move |s: &mut Screen| s.tab = i))
                .into(),
            // Keyed per page so each keeps its own scroll offset.
            Element::from(body).key(self.tab as u64),
        ])
        .padding(20.0)
        .gap(16.0)
        .grow(1.0)
        .bg(BG)
        .into()
    }

    fn basics(&self, h: Handle<Screen>) -> Element {
        let greeting = stateful(|name: &String, h| {
            let hi = if name.is_empty() {
                "Hi!".into()
            } else {
                format!("Hi, {name}!")
            };
            row([
                text_input(name.clone())
                    .placeholder("Your name")
                    .color(INK)
                    .accent(ACCENT)
                    .on_change(move |cx, v| cx.update(h, |n: &mut String| *n = v))
                    .grow(1.0)
                    .into(),
                text(hi).size(18.0).color(INK).into(),
            ])
            .align(AlignItems::CENTER)
            .gap(12.0)
        });

        let counter = row([
            button(text("−").size(22.0).color(Color::WHITE))
                .on_click(on(h, |s| s.count -= 1))
                .width(64.0)
                .into(),
            text(self.count.to_string()).size(22.0).color(INK).into(),
            button(text("+").size(22.0).color(Color::WHITE))
                .on_click(on(h, |s| s.count += 1))
                .width(64.0)
                .into(),
        ])
        .align(AlignItems::CENTER)
        .gap(16.0);

        let chips = row(CHIPS.iter().enumerate().map(|(i, &c)| {
            container(text(format!("chip {i}")).size(14.0).color(Color::WHITE))
                .bg(Color::hex(c))
                .radius(999.0)
                .padding_xy(12.0, 6.0)
                .into()
        }))
        .wrap()
        .gap(8.0);

        column([
            section("Text input", greeting.into()),
            section("Buttons", counter.into()),
            section("Wrapping row", chips.into()),
            text("Shapes are distance fields and glyphs are evaluated from their outlines, so both stay crisp at any size. Resize the window to watch the flex layout reflow.")
                .size(15.0)
                .color(MUTED)
                .into(),
        ])
        .gap(16.0)
        .into()
    }

    fn motion(&self, h: Handle<Screen>) -> Element {
        let shapes = row([
            MorphShape {
                playing: self.playing,
                shapes: rect_hexagon,
            }
            .into(),
            MorphShape {
                playing: self.playing,
                shapes: d_to_o,
            }
            .into(),
            Pulse {
                playing: self.playing,
            }
            .into(),
            button(text(if self.playing { "Pause" } else { "Resume" }).color(INK))
                .on_click(on(h, |s| s.playing = !s.playing))
                .color(Color::hex(0xe8e9ef))
                .width(96.0)
                .into(),
        ])
        .align(AlignItems::CENTER)
        .gap(12.0);

        let playing = self.playing;
        let zoom = stateful(move |typed: &String, h| {
            column([
                text_input(typed.clone())
                    .placeholder("Type a character")
                    .color(INK)
                    .accent(ACCENT)
                    .on_change(move |cx, v| cx.update(h, |t: &mut String| *t = v))
                    .into(),
                Zoom {
                    // The latest typed character.
                    ch: typed.chars().last().unwrap_or('g'),
                    playing,
                }
                .into(),
            ])
            .gap(12.0)
        });

        column([section("Morphs and easing", shapes.into()), zoom.into()])
            .gap(16.0)
            .into()
    }

    fn cards(&self, h: Handle<Screen>) -> Element {
        let add = button(text("Add card").color(Color::WHITE))
            .on_click(on(h, |s| s.items.push(s.items.last().map_or(1, |n| n + 1))))
            .color(Color::hex(0x2f9e6a));
        column(
            std::iter::once(add.into()).chain(self.items.iter().map(|&n| card(n, h).key(n as u64))),
        )
        .align(AlignItems::STRETCH)
        .gap(12.0)
        .into()
    }
}

/// External state: a job runner the app owns. Each job runs on its own
/// thread and reports entries over a channel, consumed as a `Stream`.
// ponytail: OS threads, so not on wasm; a timer future would serve the web port.
#[derive(Clone)]
struct Jobs {
    every: Duration,
    entries: u32,
}

impl Default for Jobs {
    fn default() -> Self {
        Jobs {
            every: Duration::from_millis(300),
            entries: 20,
        }
    }
}

impl Jobs {
    fn start(&self, run: u32) -> mpsc::UnboundedReceiver<String> {
        let (tx, rx) = mpsc::unbounded();
        let (every, entries) = (self.every, self.entries);
        std::thread::spawn(move || {
            let t0 = Instant::now();
            for i in 1..=entries {
                std::thread::sleep(every);
                let entry = format!(
                    "Job {run} · entry {i} · {:.1} s",
                    t0.elapsed().as_secs_f32()
                );
                // Fails once the list is gone, which ends the job.
                if tx.unbounded_send(entry).is_err() {
                    return;
                }
            }
        });
        rx
    }
}

/// A background job streams entries into a list; restarting or leaving the
/// page drops the stream, which stops the job.
fn jobs() -> Element {
    stateful(|&run: &u32, h| {
        let start =
            button(text(if run == 0 { "Start job" } else { "Restart job" }).color(Color::WHITE))
                .on_click(on(h, |run: &mut u32| *run += 1));
        let log: Element = if run == 0 {
            text("A background thread sends an entry every 300 ms; a stream builder collects them.")
                .size(15.0)
                .color(MUTED)
                .into()
        } else {
            stream_builder(
                run,
                move |scope: Scope| scope.get::<Jobs>().expect("Jobs is provided").start(run),
                |snap: &Snapshot<Vec<String>>| {
                    let n = snap.data.len();
                    let status = if snap.done {
                        format!("Done, {n} entries")
                    } else {
                        format!("Running… {n} entries")
                    };
                    let rows = snap.data.iter().rev().map(|e| {
                        container(text(e).size(15.0).color(INK))
                            .padding_xy(12.0, 8.0)
                            .bg(Color::WHITE)
                            .radius(8.0)
                            .into()
                    });
                    column([
                        text(status).size(14.0).color(MUTED).into(),
                        scroll(column(rows).gap(6.0)).height(320.0).into(),
                    ])
                    .gap(8.0)
                },
            )
            .into()
        };
        column([section("Background job", start.into()), log]).gap(16.0)
    })
    .into()
}

/// Downloads and decodes a PNG on its own thread; the UI only sees the `Bitmap`.
// ponytail: a thread per image; a small pool if pages ever load hundreds.
fn fetch(url: String) -> oneshot::Receiver<Result<Bitmap, String>> {
    let (tx, rx) = oneshot::channel();
    std::thread::spawn(move || {
        let load = || -> Result<Bitmap, Box<dyn std::error::Error>> {
            let png = ureq::get(&url).call()?.body_mut().read_to_vec()?;
            let img = image::load_from_memory_with_format(&png, image::ImageFormat::Png)?;
            let img = img.to_rgba8();
            Ok(Bitmap::from_rgba8(
                img.width(),
                img.height(),
                img.into_raw(),
            ))
        };
        // Fails once the tile is gone; nothing to do then.
        let _ = tx.send(load().map_err(|e| e.to_string()));
    });
    rx
}

/// Placeholder images from placehold.co, each downloaded in the background
/// and shown once decoded.
fn gallery() -> Element {
    let tiles = (1..=24u32).map(|n| {
        let [bg, fg] = [CHIPS[n as usize % CHIPS.len()], 0xffffff];
        let url = format!("https://placehold.co/600x400/{bg:06x}/{fg:06x}/png?text=Poster+{n}");
        future_builder(
            n,
            move |_| async move { fetch(url).await.unwrap_or_else(|e| Err(e.to_string())) },
            |snap: &Snapshot<Option<Result<Bitmap, String>>>| -> Element {
                match &snap.data {
                    Some(Ok(b)) => image(b.clone()).fill_width().radius(12.0).into(),
                    other => {
                        let msg = match other {
                            Some(Err(e)) => e.as_str(),
                            _ => "Loading…",
                        };
                        let mut tile = container(text(msg).size(14.0).color(MUTED))
                            .bg(Color::hex(0xe8e9ef))
                            .radius(12.0)
                            .fill_width()
                            .align(AlignItems::CENTER)
                            .justify(JustifyContent::CENTER);
                        tile.style_mut().aspect_ratio = Some(1.5);
                        tile.into()
                    }
                }
            },
        )
        .width(300.0)
        .into()
    });
    row(tiles).wrap().gap(12.0).into()
}

/// A titled group: muted heading over `body`.
fn section(title: &str, body: Element) -> Element {
    column([text(title).size(14.0).color(MUTED).into(), body])
        .gap(8.0)
        .fill_width()
        .into()
}

/// Worst case for outline text: the screen filled with large glyphs, redrawn every frame.
fn bench(h: Handle<Screen>) -> Element {
    let lines = (0..14).map(|_| text("Rag&@ %g1 WM").size(64.0).color(INK).into());
    column(
        [row([
            FpsMeter.into(),
            spacer().grow(1.0).into(),
            button(text("Back").color(Color::WHITE))
                .on_click(on(h, |s| s.bench = false))
                .into(),
        ])
        .align(AlignItems::CENTER)
        .into()]
        .into_iter()
        .chain(lines),
    )
    .padding(16.0)
    .bg(Color::WHITE)
    .into()
}

#[derive(Default)]
struct Fps {
    since: Option<Instant>,
    frames: u32,
    fps: f32,
}

/// Live frame-rate readout; keeps the app redrawing while it's on screen.
struct FpsMeter;

impl Widget for FpsMeter {
    fn style(&self) -> Style {
        Style {
            size: Size {
                width: Dimension::length(140.0),
                height: Dimension::length(28.0),
            },
            ..Style::default()
        }
    }

    fn paint(&self, cx: &mut PaintCx) {
        let s = cx.state.get::<Fps>();
        let since = *s.since.get_or_insert_with(Instant::now);
        s.frames += 1;
        if since.elapsed().as_secs_f32() >= 0.5 {
            s.fps = s.frames as f32 / since.elapsed().as_secs_f32();
            s.frames = 0;
            s.since = Some(Instant::now());
        }
        let label = format!("{:.0} fps", s.fps);
        let r = cx.rect;
        cx.text(&label, Font::DEFAULT, 22.0, ACCENT, None, (r.x, r.y));
        cx.request_frame();
    }
}

impl From<FpsMeter> for Element {
    fn from(w: FpsMeter) -> Self {
        Element::new(w)
    }
}

/// Seconds for one sweep of the dot across the track.
const SWEEP: f32 = 1.2;
/// Seconds for the zoom to grow or shrink once.
const ZOOM_SWEEP: f32 = 3.0;

#[derive(Default)]
struct PulseState {
    /// Sweeps travelled; accumulates frame time scaled by `speed`.
    phase: f32,
    last: Option<Instant>,
    /// 1 while playing, 0 while paused; eased so the dot glides to a stop.
    speed: Animated<f32>,
}

impl PulseState {
    /// Advances to `now` at one sweep per `sweep` seconds; returns the dot's
    /// position in 0..=1 and whether it still moves (and so needs another frame).
    fn advance(&mut self, playing: bool, now: Instant, sweep: f32) -> (f32, bool) {
        let target = if playing { 1.0 } else { 0.0 };
        self.speed
            .to(target, now, Duration::from_millis(600), ease::in_out_cubic);
        let speed = self.speed.get(now);
        if let Some(last) = self.last {
            self.phase += (now - last).as_secs_f32() * speed / sweep;
        }
        // Speed eases up from 0 on resume, so a long pause adds nothing.
        self.last = Some(now);
        let moving = speed > 0.0 || self.speed.running(now);
        let p = self.phase % 2.0;
        (
            ease::in_out_cubic(if p < 1.0 { p } else { 2.0 - p }),
            moving,
        )
    }
}

/// A dot swinging along a track; pausing eases it to a halt instead of freezing it.
struct Pulse {
    playing: bool,
}

impl Widget for Pulse {
    fn style(&self) -> Style {
        Style {
            size: Size {
                width: Dimension::auto(),
                height: Dimension::length(36.0),
            },
            flex_grow: 1.0,
            ..Style::default()
        }
    }

    fn paint(&self, cx: &mut PaintCx) {
        let now = cx.now;
        let (t, moving) = cx
            .state
            .get::<PulseState>()
            .advance(self.playing, now, SWEEP);
        if moving {
            cx.request_frame();
        }

        let r = cx.rect;
        cx.fill(r, r.h / 2.0, Color::hex(0xe8e9ef));
        let d = r.h - 8.0;
        let x = r.x + 4.0 + t * (r.w - d - 8.0);
        let dot = ACCENT.lerp(Color::hex(CHIPS[4]), t);
        cx.fill(Rect::new(x, r.y + 4.0, d, d), d / 2.0, dot);
    }
}

impl From<Pulse> for Element {
    fn from(w: Pulse) -> Self {
        Element::new(w)
    }
}

/// The two outlines a `MorphShape` swings between, for a `w`×`h` box.
type Shapes = fn(&PaintCx, f32, f32) -> (Path, Path);

#[derive(Default)]
struct MorphState {
    pulse: PulseState,
    /// Built for the box size it was made for.
    morph: Option<((f32, f32), Morph)>,
}

/// Morphs back and forth between two outlines, paused along with the pulse.
struct MorphShape {
    playing: bool,
    shapes: Shapes,
}

impl Widget for MorphShape {
    fn style(&self) -> Style {
        Style {
            size: Size {
                width: Dimension::length(48.0),
                height: Dimension::length(48.0),
            },
            flex_shrink: 0.0,
            ..Style::default()
        }
    }

    fn paint(&self, cx: &mut PaintCx) {
        let (now, r) = (cx.now, cx.rect);
        let size = (r.w, r.h);
        let fresh = match &cx.state.get::<MorphState>().morph {
            Some((s, _)) => *s != size,
            None => true,
        };
        if fresh {
            let (a, b) = (self.shapes)(cx, r.w, r.h);
            cx.state.get::<MorphState>().morph = Some((size, Morph::new(&a, &b)));
        }
        let s = cx.state.get::<MorphState>();
        let (t, moving) = s.pulse.advance(self.playing, now, SWEEP);
        let path = s.morph.as_ref().unwrap().1.at(t).offset(r.x, r.y);
        if moving {
            cx.request_frame();
        }
        cx.path(&path, ACCENT.lerp(Color::hex(CHIPS[4]), t));
    }
}

impl From<MorphShape> for Element {
    fn from(w: MorphShape) -> Self {
        Element::new(w)
    }
}

fn rect_hexagon(_: &PaintCx, w: f32, h: f32) -> (Path, Path) {
    let (cx, cy) = (w / 2.0, h / 2.0);
    let (rw, rh) = (w * 0.4, h * 0.28);
    let rect = Path::polygon(&[
        [cx - rw, cy - rh],
        [cx + rw, cy - rh],
        [cx + rw, cy + rh],
        [cx - rw, cy + rh],
    ]);
    let r = w.min(h) * 0.46;
    let corner = |k: usize| {
        // Pointy top: the rect's long edges are cut at the tips.
        let a = std::f32::consts::FRAC_PI_3 * k as f32 - std::f32::consts::FRAC_PI_2;
        [cx + r * a.cos(), cy + r * a.sin()]
    };
    (rect, Path::polygon(&(0..6).map(corner).collect::<Vec<_>>()))
}

fn d_to_o(cx: &PaintCx, w: f32, h: f32) -> (Path, Path) {
    let glyph = |c| {
        centered(
            cx.glyph_path(Font::DEFAULT, c, 44.0).unwrap_or_default(),
            w,
            h,
        )
    };
    (glyph('D'), glyph('O'))
}

/// Bounding box of `p` as (min, max) corners.
fn bounds(p: &Path) -> ([f32; 2], [f32; 2]) {
    let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
    for q in p.contours.iter().flatten().flatten() {
        lo = [lo[0].min(q[0]), lo[1].min(q[1])];
        hi = [hi[0].max(q[0]), hi[1].max(q[1])];
    }
    (lo, hi)
}

/// `p` moved so its bounding box is centred in a `w`×`h` box at the origin.
fn centered(p: Path, w: f32, h: f32) -> Path {
    let (lo, hi) = bounds(&p);
    p.offset((w - lo[0] - hi[0]) / 2.0, (h - lo[1] - hi[1]) / 2.0)
}

/// Smallest zoom, as a fraction of the size that fills the box.
const ZOOM_MIN: f32 = 0.01;

/// One glyph growing from a speck until it fills a square as wide as its
/// parent, and back; paused along with the pulse.
struct Zoom {
    ch: char,
    playing: bool,
}

impl Widget for Zoom {
    fn style(&self) -> Style {
        Style {
            size: Size {
                width: Dimension::percent(1.0),
                height: Dimension::auto(),
            },
            aspect_ratio: Some(1.0),
            ..Style::default()
        }
    }

    fn paint(&self, cx: &mut PaintCx) {
        let (now, r) = (cx.now, cx.rect);
        let (t, moving) = cx
            .state
            .get::<PulseState>()
            .advance(self.playing, now, ZOOM_SWEEP);
        if moving {
            cx.request_frame();
        }
        cx.fill(r, 14.0, Color::WHITE);
        let Some(unit) = cx.glyph_path(Font::DEFAULT, self.ch, 1.0) else {
            return;
        };
        let (lo, hi) = bounds(&unit);
        let (w, h) = (hi[0] - lo[0], hi[1] - lo[1]);
        if !(w > 0.0 && h > 0.0) {
            return;
        }
        // Geometric, so each step looks like the same amount of growth.
        let fit = 0.9 * (r.w / w).min(r.h / h);
        let size = fit * ZOOM_MIN.powf(1.0 - t);
        let Some(p) = cx.glyph_path(Font::DEFAULT, self.ch, size) else {
            return;
        };
        let p = centered(p, r.w, r.h).offset(r.x, r.y);
        cx.path(&p, ACCENT.lerp(Color::hex(CHIPS[4]), t));
    }
}

impl From<Zoom> for Element {
    fn from(w: Zoom) -> Self {
        Element::new(w)
    }
}

#[derive(Default)]
struct Settings {
    wifi: bool,
    volume: f32,
    size: usize,
    range: usize,
}

/// The simple controls; their values are local UI state.
fn controls() -> Element {
    stateful(|s: &Settings, h| {
        let label = |l: String| text(l).size(16.0).color(INK);
        let switches = column([
            row([
                label("Wi-Fi".into()).into(),
                spacer().grow(1.0).into(),
                toggle(s.wifi)
                    .accent(ACCENT)
                    .on_change(move |cx, on| cx.update(h, move |s: &mut Settings| s.wifi = on))
                    .into(),
            ])
            .align(AlignItems::CENTER)
            .into(),
            row([
                label(format!("Volume {:.0}", s.volume)).width(100.0).into(),
                slider(s.volume, 0.0..=100.0)
                    .accent(ACCENT)
                    .on_change(move |cx, v| cx.update(h, move |s: &mut Settings| s.volume = v))
                    .grow(1.0)
                    .into(),
            ])
            .align(AlignItems::CENTER)
            .into(),
        ])
        .gap(12.0);
        let sizes = column(
            ["Small", "Medium", "Large"]
                .into_iter()
                .enumerate()
                .map(|(i, l)| {
                    radio(s.size == i, label(l.into()))
                        .accent(ACCENT)
                        .on_select(move |cx| cx.update(h, move |s: &mut Settings| s.size = i))
                        .into()
                }),
        );
        let range = segmented(["Day", "Week", "Month"], s.range)
            .on_change(move |cx, i| cx.update(h, move |s: &mut Settings| s.range = i));
        column([
            section("Toggle and slider", panel(switches.into())),
            section("Radio buttons", panel(sizes.into())),
            section("Segmented button", range.into()),
        ])
        .gap(16.0)
    })
    .into()
}

/// White rounded panel, like the cards.
fn panel(body: Element) -> Element {
    container(body)
        .padding(14.0)
        .bg(Color::WHITE)
        .radius(14.0)
        .border(1.0, Color::hex(0xdfe1e8))
        .into()
}

/// Expanded/collapsed is ephemeral UI state, so it stays local to the card.
fn card(n: u32, root: Handle<Screen>) -> Element {
    stateful(move |&open: &bool, h| {
        let mut body = vec![text(format!("Card {n}")).size(18.0).color(INK).into()];
        if open {
            body.push(
                text("Rounded box SDF with border and soft shadow. This text lives in local widget state, toggled without an app message.")
                    .size(14.0)
                    .color(MUTED)
                    .into(),
            );
        }
        row([
            container(text(n.to_string()).size(20.0).color(Color::WHITE))
                .bg(ACCENT)
                .radius(24.0)
                .width(48.0)
                .height(48.0)
                .shrink(0.0)
                .align(AlignItems::CENTER)
                .justify(JustifyContent::CENTER)
                .into(),
            column(body).gap(4.0).grow(1.0).shrink(1.0).into(),
            button(text(if open { "less" } else { "more" }).size(14.0).color(INK))
                .on_click(move |cx| cx.update(h, |open: &mut bool| *open = !*open))
                .color(Color::hex(0xe8e9ef))
                .into(),
            button(text("×").size(20.0).color(INK))
                .on_click(on(root, move |s| s.items.retain(|&i| i != n)))
                .color(Color::hex(0xe8e9ef))
                .into(),
        ])
        .align(AlignItems::CENTER)
        .gap(14.0)
        .padding(14.0)
        .bg(Color::WHITE)
        .radius(14.0)
        .border(1.0, Color::hex(0xdfe1e8))
        .shadow(Shadow {
            color: Color::BLACK.alpha(0.12),
            offset: (0.0, 3.0),
            blur: 8.0,
        })
    })
    .into()
}

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
fn android_main(app: dfui::AndroidApp) {
    android_logger::init_once(
        android_logger::Config::default().with_max_level(log::LevelFilter::Info),
    );
    dfui::run_android(Demo::default(), app);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pulse_pauses_smoothly_and_resumes_without_jumping() {
        let t0 = Instant::now();
        let at = |ms: u64| t0 + Duration::from_millis(ms);
        let mut s = PulseState::default();
        s.advance(true, at(0), SWEEP);
        let (_, moving) = s.advance(true, at(300), SWEEP);
        assert!(moving);
        let before = s.phase;
        // Pausing glides on for a moment before stopping.
        s.advance(false, at(316), SWEEP);
        let (_, gliding) = s.advance(false, at(332), SWEEP);
        assert!(gliding && s.phase > before);
        let (_, moving) = s.advance(false, at(1000), SWEEP);
        assert!(!moving, "stopped once eased to zero");
        let stopped = s.phase;
        // A long pause adds nothing; resuming starts from rest.
        s.advance(true, at(9000), SWEEP);
        assert_eq!(s.phase, stopped);
        s.advance(true, at(9016), SWEEP);
        assert!(s.phase - stopped < 0.001, "eases back in");
    }

    #[test]
    #[ignore = "needs a GPU; writes target/demo.png for eyeballing"]
    fn snapshot() {
        let img = dfui::snapshot(super::Demo::default(), (800, 1000), 2.0);
        img.save(concat!(env!("CARGO_MANIFEST_DIR"), "/../target/demo.png"))
            .unwrap();
    }

    struct Ladder;

    impl dfui::App for Ladder {
        fn view(&self) -> dfui::Element {
            use dfui::widgets::{Styled, column, text};

            column([10.0, 13.0, 18.0, 32.0, 64.0, 240.0].map(|s| text("Rag&@ %g1").size(s).into()))
                .padding(8.0)
                .into()
        }
    }

    #[test]
    #[ignore = "needs a GPU; writes target/ladder.png"]
    fn ladder() {
        let img = dfui::snapshot(Ladder, (1200, 640), 1.0);
        img.save(concat!(env!("CARGO_MANIFEST_DIR"), "/../target/ladder.png"))
            .unwrap();
    }
}
