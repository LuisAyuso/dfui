use std::time::{Duration, Instant};

use dfui::anim::{Lerp, ease};
use dfui::taffy::{AlignItems, Dimension, JustifyContent, Size, Style};
use dfui::widgets::{
    Styled, button, column, container, row, scroll, spacer, stateful, text, text_input,
};
use dfui::{Animated, App, Color, Element, Font, PaintCx, Rect, Shadow, Widget};

static BOLD: Font = Font(include_bytes!("../../assets/DejaVuSans-Bold.ttf"));

const BG: Color = Color::hex(0xf2f3f7);
const INK: Color = Color::hex(0x1d2030);
const MUTED: Color = Color::hex(0x6b7080);
const ACCENT: Color = Color::hex(0x3b6fe0);
const CHIPS: [u32; 5] = [0xe25c5c, 0xe0a23b, 0x4cae6e, 0x3b9ee0, 0x8a5ce2];

pub struct Demo {
    count: i32,
    items: Vec<u32>,
    bench: bool,
    playing: bool,
}

impl Default for Demo {
    fn default() -> Self {
        Demo {
            count: 0,
            items: (1..=12).collect(),
            bench: false,
            playing: true,
        }
    }
}

#[derive(Clone)]
pub enum Msg {
    Inc,
    Dec,
    Add,
    Remove(u32),
    Bench,
    Play,
}

impl App for Demo {
    type Msg = Msg;

    fn update(&mut self, msg: Msg) {
        match msg {
            Msg::Inc => self.count += 1,
            Msg::Dec => self.count -= 1,
            Msg::Add => self.items.push(self.items.last().map_or(1, |n| n + 1)),
            Msg::Remove(n) => self.items.retain(|&i| i != n),
            Msg::Bench => self.bench = !self.bench,
            Msg::Play => self.playing = !self.playing,
        }
    }

    fn view(&self) -> Element<Msg> {
        if self.bench {
            return bench();
        }
        let header = row([
            text("dfui").font(BOLD).size(28.0).color(INK).into(),
            spacer().grow(1.0).into(),
            text(format!("count {}", self.count))
                .size(18.0)
                .color(MUTED)
                .into(),
            button(text("Bench").color(INK))
                .on_press(Msg::Bench)
                .color(Color::hex(0xe8e9ef))
                .into(),
        ])
        .align(AlignItems::CENTER)
        .gap(12.0);

        let counter = row([
            button(text("−").size(22.0).color(Color::WHITE))
                .on_press(Msg::Dec)
                .width(64.0)
                .into(),
            button(text("+").size(22.0).color(Color::WHITE))
                .on_press(Msg::Inc)
                .width(64.0)
                .into(),
            spacer().grow(1.0).into(),
            button(text("Add card").color(Color::WHITE))
                .on_press(Msg::Add)
                .color(Color::hex(0x2f9e6a))
                .into(),
        ])
        .gap(12.0);

        let pulse = row([
            Pulse {
                playing: self.playing,
            }
            .into(),
            button(text(if self.playing { "Pause" } else { "Resume" }).color(INK))
                .on_press(Msg::Play)
                .color(Color::hex(0xe8e9ef))
                .width(96.0)
                .into(),
        ])
        .align(AlignItems::CENTER)
        .gap(12.0);

        let chips = row(CHIPS.iter().enumerate().map(|(i, &c)| {
            container(text(format!("chip {i}")).size(14.0).color(Color::WHITE))
                .bg(Color::hex(c))
                .radius(999.0)
                .padding_xy(12.0, 6.0)
                .into()
        }))
        .wrap()
        .gap(8.0);

        let cards = column(self.items.iter().map(|&n| card(n).key(n as u64)))
            .gap(12.0)
            .padding(4.0);

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

        column([
            header.into(),
            greeting.into(),
            counter.into(),
            pulse.into(),
            chips.into(),
            text("Shapes are distance fields and glyphs are evaluated from their outlines, so both stay crisp at any size. Resize the window to watch the flex layout reflow.")
                .size(15.0)
                .color(MUTED)
                .into(),
            scroll(cards).grow(1.0).into(),
        ])
        .padding(20.0)
        .gap(16.0)
        .bg(BG)
        .into()
    }
}

/// Worst case for outline text: the screen filled with large glyphs, redrawn every frame.
fn bench() -> Element<Msg> {
    let lines = (0..14).map(|_| text("Rag&@ %g1 WM").size(64.0).color(INK).into());
    column(
        [row([
            FpsMeter.into(),
            spacer().grow(1.0).into(),
            button(text("Back").color(Color::WHITE))
                .on_press(Msg::Bench)
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

impl<M> Widget<M> for FpsMeter {
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

impl<M: 'static> From<FpsMeter> for Element<M> {
    fn from(w: FpsMeter) -> Self {
        Element::new(w)
    }
}

/// Seconds for one sweep of the dot across the track.
const SWEEP: f32 = 1.2;

#[derive(Default)]
struct PulseState {
    /// Sweeps travelled; accumulates frame time scaled by `speed`.
    phase: f32,
    last: Option<Instant>,
    /// 1 while playing, 0 while paused; eased so the dot glides to a stop.
    speed: Animated<f32>,
}

impl PulseState {
    /// Advances to `now`; returns the dot's position in 0..=1 and whether it
    /// still moves (and so needs another frame).
    fn advance(&mut self, playing: bool, now: Instant) -> (f32, bool) {
        let target = if playing { 1.0 } else { 0.0 };
        self.speed
            .to(target, now, Duration::from_millis(600), ease::in_out_cubic);
        let speed = self.speed.get(now);
        if let Some(last) = self.last {
            self.phase += (now - last).as_secs_f32() * speed / SWEEP;
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

impl<M> Widget<M> for Pulse {
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
        let (t, moving) = cx.state.get::<PulseState>().advance(self.playing, now);
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

impl<M: 'static> From<Pulse> for Element<M> {
    fn from(w: Pulse) -> Self {
        Element::new(w)
    }
}

/// Expanded/collapsed is ephemeral UI state, so it stays local to the card.
fn card(n: u32) -> Element<Msg> {
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
                .on_press(Msg::Remove(n))
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
        s.advance(true, at(0));
        let (_, moving) = s.advance(true, at(300));
        assert!(moving);
        let before = s.phase;
        // Pausing glides on for a moment before stopping.
        s.advance(false, at(316));
        let (_, gliding) = s.advance(false, at(332));
        assert!(gliding && s.phase > before);
        let (_, moving) = s.advance(false, at(1000));
        assert!(!moving, "stopped once eased to zero");
        let stopped = s.phase;
        // A long pause adds nothing; resuming starts from rest.
        s.advance(true, at(9000));
        assert_eq!(s.phase, stopped);
        s.advance(true, at(9016));
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
        type Msg = ();
        fn update(&mut self, _: ()) {}
        fn view(&self) -> dfui::Element<()> {
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
