use super::*;
use crate::render::{damage, render_offscreen};
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll};

use crate::path::{Path, Shape};
use crate::text::Font;
use crate::widget::{Scope, Widget};
use crate::widgets::{
    Snapshot, Styled, button, column, dialog, dropdown, future_builder, icon_button, overlays,
    popup, provide, radio, row, scroll, segmented, slider, snackbar, spacer, stateful,
    stream_builder, text, text_input, toggle, tooltip, watch,
};

/// State of the root `stateful`.
fn root<S: Default + 'static, A: App>(rt: &mut Runtime<A>) -> &mut S {
    let id = rt.ids[0];
    rt.states.get_mut(&id).unwrap().get::<S>()
}

#[derive(Default)]
struct Counter {
    dark: Cell<bool>,
}

impl App for Counter {
    fn view(&self) -> Element {
        stateful(|_: &i32, h| {
            button(text("+"))
                .on_click(move |cx| cx.update(h, |n: &mut i32| *n += 1))
                .height(40.0)
        })
        .into()
    }
    fn background(&self) -> Color {
        if self.dark.get() {
            Color::BLACK
        } else {
            Color::WHITE
        }
    }
}

#[test]
fn button_click_updates_only_when_released_inside() {
    let mut rt = Runtime::new(Counter::default());
    rt.viewport = (200.0, 200.0);
    rt.relayout();
    rt.dispatch(Event::PointerDown { x: 10.0, y: 10.0 });
    rt.dispatch(Event::PointerUp { x: 10.0, y: 10.0 });
    assert_eq!(*root::<i32, _>(&mut rt), 1);
    rt.dispatch(Event::PointerDown { x: 10.0, y: 10.0 });
    rt.dispatch(Event::PointerUp { x: 10.0, y: 100.0 });
    assert_eq!(*root::<i32, _>(&mut rt), 1);
    rt.dispatch(Event::PointerDown { x: 10.0, y: 100.0 });
    rt.dispatch(Event::PointerUp { x: 10.0, y: 10.0 });
    assert_eq!(*root::<i32, _>(&mut rt), 1);
}

struct Field;

impl App for Field {
    fn view(&self) -> Element {
        stateful(|v: &String, h| {
            text_input(v.clone()).on_change(move |cx, v| cx.update(h, |s: &mut String| *s = v))
        })
        .into()
    }
}

fn field(rt: &mut Runtime<Field>) -> String {
    root::<String, _>(rt).clone()
}

#[test]
fn text_input_edits_only_while_focused() {
    let mut rt = Runtime::new(Field);
    rt.viewport = (200.0, 200.0);
    rt.relayout();
    rt.dispatch(Event::Text("x".into()));
    assert_eq!(field(&mut rt), "");

    rt.dispatch(Event::PointerDown { x: 10.0, y: 10.0 });
    rt.dispatch(Event::PointerUp { x: 10.0, y: 10.0 });
    for ev in [
        Event::Text("aé".into()),
        Event::Key(Key::Left),
        Event::Text("X".into()),
        Event::Key(Key::Home),
        Event::Key(Key::Delete),
        Event::Key(Key::End),
        Event::Key(Key::Backspace),
    ] {
        rt.dispatch(ev);
    }
    assert_eq!(field(&mut rt), "X");

    rt.dispatch(Event::PointerDown { x: 10.0, y: 190.0 });
    rt.dispatch(Event::Text("z".into()));
    assert_eq!(field(&mut rt), "X");
}

fn damage_after<A: App>(
    rt: &mut Runtime<A>,
    events: impl IntoIterator<Item = Event>,
) -> Option<Rect> {
    let t = task::now();
    task::set_clock(Some(t));
    let (a, na) = rt.scene(&mut false);
    for ev in events {
        rt.dispatch(ev);
    }
    // Transitions start at the first paint after the event; a second paint
    // a second later shows them settled.
    task::set_clock(Some(t + Duration::from_secs(1)));
    rt.scene(&mut false);
    task::set_clock(Some(t + Duration::from_secs(2)));
    let (b, nb) = rt.scene(&mut false);
    damage(&a, &na, &b, &nb)
}

#[test]
fn interactions_damage_only_the_widget() {
    let mut rt = Runtime::new(Counter::default());
    rt.viewport = (200.0, 200.0);
    rt.relayout();
    let hover = damage_after(&mut rt, [Event::PointerMove { x: 10.0, y: 10.0 }]).unwrap();
    // Button is 40px tall; its shadow reaches a few px further.
    assert!(hover.y + hover.h < 50.0, "{hover:?}");
    assert_eq!(
        damage_after(&mut rt, [Event::PointerMove { x: 12.0, y: 10.0 }]),
        None
    );

    let mut rt = Runtime::new(Field);
    rt.viewport = (200.0, 200.0);
    rt.relayout();
    rt.dispatch(Event::PointerDown { x: 10.0, y: 10.0 });
    let field = rt.placed[1].rect;
    let typed = damage_after(&mut rt, [Event::Text("hi".into())]).unwrap();
    assert!(
        typed.h <= field.h + 4.0 && typed.y >= field.y - 2.0,
        "{typed:?} {field:?}"
    );
}

#[test]
fn background_change_damages_everything() {
    let mut rt = Runtime::new(Counter::default());
    rt.viewport = (200.0, 200.0);
    rt.relayout();
    let (a, na) = rt.scene(&mut false);
    rt.app.dark.set(true);
    let (b, nb) = rt.scene(&mut false);
    let d = damage(&a, &na, &b, &nb).unwrap();
    assert!(d.w >= 200.0 && d.h >= 200.0, "{d:?}");
}

#[test]
#[ignore = "needs a GPU"]
fn new_device_gets_outlines_already_uploaded_elsewhere() {
    let mut rt = Runtime::new(Counter::default());
    rt.viewport = (40.0, 40.0);
    rt.relayout();
    let (scene, _) = rt.scene(&mut false);
    let draw = |text: &mut Text| render_offscreen(&[(&scene, None)], text, (40, 40), 1.0);
    let first = draw(&mut rt.text);
    // A second device, as after an Android resume, with the same `Text`.
    assert_eq!(draw(&mut rt.text), first);
}

#[derive(Default)]
struct Controls {
    on: bool,
    level: f32,
    picked: bool,
    seg: usize,
}

struct ControlsApp;

impl App for ControlsApp {
    fn view(&self) -> Element {
        stateful(|s: &Controls, h| {
            column([
                toggle(s.on)
                    .on_change(move |cx, b| cx.update(h, move |s: &mut Controls| s.on = b))
                    .into(),
                slider(s.level, 0.0..=10.0)
                    .on_change(move |cx, v| cx.update(h, move |s: &mut Controls| s.level = v))
                    .width(122.0)
                    .into(),
                radio(s.picked, text("pick"))
                    .on_select(move |cx| cx.update(h, |s: &mut Controls| s.picked = true))
                    .into(),
                segmented(["a", "b", "c"], s.seg)
                    .on_change(move |cx, i| cx.update(h, move |s: &mut Controls| s.seg = i))
                    .width(150.0)
                    .into(),
            ])
            .align(taffy::AlignItems::START)
        })
        .into()
    }
}

#[test]
fn controls_report_taps_and_drags() {
    let mut rt = Runtime::new(ControlsApp);
    rt.viewport = (300.0, 400.0);
    rt.relayout();
    let center = |rt: &Runtime<ControlsApp>, i: usize| {
        let r = rt.placed[i].rect;
        (r.x + r.w / 2.0, r.y + r.h / 2.0)
    };
    let tap = |rt: &mut Runtime<ControlsApp>, (x, y): (f32, f32)| {
        rt.dispatch(Event::PointerDown { x, y });
        rt.dispatch(Event::PointerUp { x, y });
    };
    // Pre-order: stateful, column, toggle, slider, radio, its label, segmented.
    let p = center(&rt, 2);
    tap(&mut rt, p);
    assert!(root::<Controls, _>(&mut rt).on);
    tap(&mut rt, p);
    assert!(!root::<Controls, _>(&mut rt).on);

    // 122px wide with a 22px thumb: 100px of travel from x = 11.
    let (_, y) = center(&rt, 3);
    rt.dispatch(Event::PointerDown { x: 61.0, y });
    assert_eq!(
        root::<Controls, _>(&mut rt).level,
        5.0,
        "tap jumps to the middle"
    );
    rt.dispatch(Event::PointerMove {
        x: 81.0,
        y: y + 300.0,
    });
    assert_eq!(
        root::<Controls, _>(&mut rt).level,
        7.0,
        "captured drag follows x only"
    );
    rt.dispatch(Event::PointerMove { x: 500.0, y });
    rt.dispatch(Event::PointerUp { x: 500.0, y });
    assert_eq!(
        root::<Controls, _>(&mut rt).level,
        10.0,
        "clamped to the range"
    );
    rt.dispatch(Event::PointerMove { x: 11.0, y });
    assert_eq!(root::<Controls, _>(&mut rt).level, 10.0, "released");

    let p = center(&rt, 5);
    tap(&mut rt, p);
    assert!(
        root::<Controls, _>(&mut rt).picked,
        "the label is part of the target"
    );

    let seg = rt.placed[6].rect;
    let y = seg.y + seg.h / 2.0;
    tap(&mut rt, (seg.x + seg.w - 5.0, y));
    assert_eq!(root::<Controls, _>(&mut rt).seg, 2);
    rt.dispatch(Event::PointerDown { x: seg.x + 5.0, y });
    rt.dispatch(Event::PointerUp {
        x: seg.x + seg.w / 2.0,
        y,
    });
    assert_eq!(
        root::<Controls, _>(&mut rt).seg,
        2,
        "released over another option"
    );
}

#[derive(Default)]
struct Layers {
    taps: u32,
    ok: u32,
    dismissed: u32,
}

struct LayersApp;

impl App for LayersApp {
    fn view(&self) -> Element {
        stateful(|_: &Layers, h| {
            let ok =
                button(text("ok")).on_click(move |cx| cx.update(h, |s: &mut Layers| s.ok += 1));
            let anchored = column([
                button(text("+"))
                    .on_click(move |cx| cx.update(h, |s: &mut Layers| s.taps += 1))
                    .height(40.0)
                    .into(),
                popup(column([]).width(50.0).height(10.0)).into(),
            ]);
            column([
                dialog(ok)
                    .on_dismiss(move |cx| cx.update(h, |s: &mut Layers| s.dismissed += 1))
                    .into(),
                scroll(anchored).height(60.0).into(),
            ])
            .padding(10.0)
        })
        .into()
    }
}

#[test]
fn layers_float_above_and_take_input_first() {
    let mut rt = Runtime::new(LayersApp);
    rt.viewport = (200.0, 200.0);
    rt.insets = [20.0, 0.0, 10.0, 0.0];
    let t = Instant::now();
    task::set_clock(Some(t));
    rt.relayout();
    // Past the dialog's slide in.
    task::set_clock(Some(t + Duration::from_secs(1)));
    rt.reposition();
    // Pre-order: stateful, column, dialog, sink, card, ok, its text,
    // scroll, anchor column, +, its text, popup, sink, popup content.
    assert_eq!(rt.order, [0, 1, 7, 8, 9, 10, 2, 3, 4, 5, 6, 11, 12, 13]);
    let full = Rect::new(0.0, 0.0, 200.0, 200.0);
    assert_eq!(
        rt.placed[2].rect,
        Rect::new(0.0, 20.0, 200.0, 170.0),
        "window minus bars"
    );
    let anchor = rt.placed[8].rect;
    assert_eq!(rt.placed[11].rect, anchor, "popup covers its parent");
    assert_eq!(rt.placed[12].rect.y, anchor.y + anchor.h + 4.0);
    assert_eq!(rt.placed[12].rect.w, anchor.w, "at least as wide");
    assert_eq!(rt.placed[13].clip, full, "escapes the scroll clip");
    assert_eq!(rt.placed[7].content.height, 40.0, "not scrollable content");

    let tap = |rt: &mut Runtime<LayersApp>, r: Rect, dx: f32, dy: f32| {
        let (x, y) = (r.x + dx, r.y + dy);
        rt.dispatch(Event::PointerDown { x, y });
        rt.dispatch(Event::PointerUp { x, y });
    };
    let (plus, card, ok) = (rt.placed[9].rect, rt.placed[4].rect, rt.placed[5].rect);
    tap(&mut rt, plus, 5.0, 5.0);
    assert_eq!(root::<Layers, _>(&mut rt).taps, 0, "blocked by the modal");
    assert_eq!(root::<Layers, _>(&mut rt).dismissed, 1);
    tap(&mut rt, card, 5.0, 5.0);
    assert_eq!(root::<Layers, _>(&mut rt).dismissed, 1, "inside the card");
    tap(&mut rt, ok, 5.0, 5.0);
    assert_eq!(root::<Layers, _>(&mut rt).ok, 1);
}

struct Host;

impl App for Host {
    fn view(&self) -> Element {
        overlays(|ov| {
            button(text("open")).height(40.0).on_click(move |cx| {
                ov.show(cx, |me| {
                    dialog(button(text("x")).on_click(move |cx| me.close(cx)))
                })
            })
        })
        .into()
    }
}

#[test]
fn controller_animates_overlays_out_then_removes_them() {
    let mut rt = Runtime::new(Host);
    rt.viewport = (200.0, 200.0);
    let t = Instant::now();
    task::set_clock(Some(t));
    rt.relayout();
    let tap = |rt: &mut Runtime<Host>, x: f32, y: f32| {
        rt.dispatch(Event::PointerDown { x, y });
        rt.dispatch(Event::PointerUp { x, y });
    };
    tap(&mut rt, 5.0, 5.0);
    // Pre-order: host, column, open, its text, then per entry: timer,
    // dialog, sink, card, x, its text.
    assert_eq!(rt.ids.len(), 10);
    assert!(rt.placed[7].rect.y >= 200.0, "starts below the window");
    task::set_clock(Some(t + Duration::from_secs(1)));
    rt.reposition();
    assert!(rt.placed[7].rect.y < 200.0, "slid in");

    tap(&mut rt, 5.0, 5.0);
    assert_eq!(rt.ids.len(), 10, "dismissed, still animating out");
    tap(&mut rt, 5.0, 5.0);
    assert_eq!(rt.ids.len(), 16, "a closing overlay lets input through");

    let due = task::fire_timers(task::now()).expect("removal timer");
    task::set_clock(Some(due));
    task::fire_timers(due);
    assert!(rt.drive());
    assert_eq!(rt.ids.len(), 4 + 2 + 6, "the first builds nothing");
}

#[test]
fn back_dismisses_the_topmost_overlay_only() {
    let mut rt = Runtime::new(Host);
    rt.viewport = (200.0, 200.0);
    rt.relayout();
    assert!(!rt.dispatch(Event::Back), "nothing to dismiss");
    rt.dispatch(Event::PointerDown { x: 5.0, y: 5.0 });
    rt.dispatch(Event::PointerUp { x: 5.0, y: 5.0 });
    assert!(rt.dispatch(Event::Back), "closes the dialog");
    assert!(
        !rt.dispatch(Event::Back),
        "a closing dialog lets it through"
    );
}

struct Toasts;

impl App for Toasts {
    fn view(&self) -> Element {
        overlays(|ov| {
            button(text("toast")).height(40.0).on_click(move |cx| {
                ov.show(cx, |_| snackbar("one"));
                ov.show(cx, |_| snackbar("two"));
            })
        })
        .into()
    }
}

#[test]
fn snackbars_queue_one_at_a_time() {
    let mut rt = Runtime::new(Toasts);
    rt.viewport = (200.0, 200.0);
    rt.relayout();
    rt.dispatch(Event::PointerDown { x: 5.0, y: 5.0 });
    rt.dispatch(Event::PointerUp { x: 5.0, y: 5.0 });
    task::fire_timers(Instant::now());
    rt.drive();
    // Host, column, button, its text; the first: queue, timer, overlay,
    // sink, bar, text; the second: queue, its empty column.
    assert_eq!(rt.ids.len(), 4 + 6 + 2, "the second waits for the first");
}

struct Anchored;

impl App for Anchored {
    fn view(&self) -> Element {
        let anchor = || column([popup(column([]).width(50.0).height(30.0)).into()]).height(20.0);
        let list = column([anchor().into(), column([]).height(300.0).into()]);
        column([
            scroll(list).height(100.0).into(),
            spacer().grow(1.0).into(),
            anchor().into(),
        ])
        .into()
    }
}

#[test]
fn popups_flip_into_the_window_and_hide_with_their_anchor() {
    let mut rt = Runtime::new(Anchored);
    rt.viewport = (200.0, 200.0);
    rt.relayout();
    let menus: Vec<usize> = (0..rt.placed.len())
        .filter(|&i| (rt.placed[i].rect.w, rt.placed[i].rect.h) == (50.0, 30.0))
        .collect();
    let [top, bottom] = menus[..] else {
        panic!("two menus, got {menus:?}")
    };
    assert_eq!(rt.placed[top].rect.y, 24.0, "below its anchor");
    assert_eq!(
        rt.placed[bottom].rect.y,
        180.0 - 4.0 - 30.0,
        "flipped above"
    );
    assert!(rt.placed[top].clip.contains(10.0, 30.0));
    rt.dispatch(Event::Wheel {
        x: 10.0,
        y: 10.0,
        dy: 50.0,
    });
    let clip = rt.placed[top].clip;
    assert!(clip.w * clip.h == 0.0, "anchor scrolled away: {clip:?}");
}

struct Tip;

impl App for Tip {
    fn view(&self) -> Element {
        tooltip(button(text("b")).height(40.0), "tip").into()
    }
}

#[test]
fn tooltip_shows_on_long_press_and_lingers() {
    let mut rt = Runtime::new(Tip);
    rt.viewport = (200.0, 200.0);
    let t = Instant::now();
    let ms = Duration::from_millis;
    task::set_clock(Some(t));
    rt.relayout();
    let tip = rt.placed.len() - 1;
    let at = |rt: &mut Runtime<Tip>, d: u64| {
        task::set_clock(Some(t + ms(d)));
        rt.reposition();
        rt.placed[tip].rect.y.abs() < 1e5
    };
    let lift = |rt: &mut Runtime<Tip>| {
        rt.dispatch(Event::PointerUp { x: 5.0, y: 5.0 });
        rt.dispatch(Event::Leave);
    };
    rt.dispatch(Event::PointerDown { x: 5.0, y: 5.0 });
    assert!(!at(&mut rt, 100), "not on a short press");
    lift(&mut rt);
    assert!(!at(&mut rt, 200), "nor after a tap");
    rt.dispatch(Event::PointerDown { x: 5.0, y: 5.0 });
    assert!(at(&mut rt, 800), "held long enough");
    lift(&mut rt);
    assert!(at(&mut rt, 1500), "lingers after release");
    assert!(!at(&mut rt, 2500), "then hides");
}

#[derive(Default)]
struct Pick {
    selected: usize,
    below: u32,
}

struct PickApp;

impl App for PickApp {
    fn view(&self) -> Element {
        stateful(|s: &Pick, h| {
            column([
                dropdown(["a", "b", "c"], s.selected)
                    .on_change(move |cx, i| cx.update(h, move |s: &mut Pick| s.selected = i))
                    .into(),
                button(text("below"))
                    .height(150.0)
                    .on_click(move |cx| cx.update(h, |s: &mut Pick| s.below += 1))
                    .into(),
            ])
        })
        .into()
    }
}

#[test]
fn closed_dropdown_menu_lets_taps_through() {
    let mut rt = Runtime::new(PickApp);
    rt.viewport = (200.0, 300.0);
    let t = Instant::now();
    task::set_clock(Some(t));
    rt.relayout();
    let tap = |rt: &mut Runtime<PickApp>, x: f32, y: f32| {
        rt.dispatch(Event::PointerDown { x, y });
        rt.dispatch(Event::PointerUp { x, y });
    };
    let field = rt.placed[3].rect;
    // Where the menu's second item rests when open.
    let (x, y) = (field.x + 5.0, field.y + field.h + 60.0);
    tap(&mut rt, x, y);
    assert_eq!(
        root::<Pick, _>(&mut rt).below,
        1,
        "no closed menu in the way"
    );
    tap(&mut rt, field.x + 5.0, field.y + 5.0);
    task::set_clock(Some(t + Duration::from_secs(1)));
    rt.reposition();
    tap(&mut rt, x, y);
    let s = root::<Pick, _>(&mut rt);
    assert_eq!((s.below, s.selected), (1, 1), "picked from the open menu");
}

/// Logs, per pointer event it receives, whether the pointer was over it.
struct Probe(Rc<RefCell<Vec<(char, bool)>>>, char);

impl Widget for Probe {
    fn style(&self) -> taffy::Style {
        taffy::Style {
            size: taffy::Size::from_lengths(50.0, 50.0),
            ..Default::default()
        }
    }
    fn event(&self, ev: &Event, cx: &mut EventCx) {
        if let Event::PointerDown { x, y }
        | Event::PointerMove { x, y }
        | Event::PointerUp { x, y } = *ev
        {
            self.0.borrow_mut().push((self.1, cx.hit(x, y)));
        }
    }
}

#[derive(Default)]
struct Probes {
    log: Rc<RefCell<Vec<(char, bool)>>>,
    shifted: Cell<bool>,
}

impl App for Probes {
    fn view(&self) -> Element {
        let probe = |c: char| Element::new(Probe(self.log.clone(), c)).key(c as u64);
        let lead = self.shifted.get().then(|| column([]).width(5.0).into());
        row(lead.into_iter().chain([probe('a'), probe('b')])).into()
    }
}

#[test]
fn pointer_events_reach_nodes_under_them_and_the_ones_they_left() {
    let mut rt = Runtime::new(Probes::default());
    rt.viewport = (200.0, 100.0);
    rt.relayout();
    let step = |rt: &mut Runtime<Probes>, ev: Event| {
        rt.dispatch(ev);
        std::mem::take(&mut *rt.app.log.borrow_mut())
    };
    let mv = |x| Event::PointerMove { x, y: 10.0 };
    assert_eq!(step(&mut rt, mv(10.0)), [('a', true)]);
    assert_eq!(
        step(&mut rt, mv(60.0)),
        [('b', true), ('a', false)],
        "a sees it leave"
    );
    assert_eq!(step(&mut rt, mv(70.0)), [('b', true)], "then a is skipped");

    // A press keeps reaching its node however far the pointer goes.
    step(&mut rt, Event::PointerDown { x: 10.0, y: 10.0 });
    assert_eq!(step(&mut rt, mv(150.0)), [('a', false)]);
    assert_eq!(step(&mut rt, mv(160.0)), [('a', false)]);
    let up = step(&mut rt, Event::PointerUp { x: 160.0, y: 10.0 });
    assert_eq!(up, [('a', false)]);
    assert_eq!(step(&mut rt, mv(170.0)), [], "released");

    // A node that moved in a rebuild still sees the pointer leave.
    step(&mut rt, mv(10.0));
    rt.app.shifted.set(true);
    rt.rebuild();
    rt.relayout();
    assert_eq!(step(&mut rt, mv(150.0)), [('a', false)]);
}

struct Long;

impl App for Long {
    fn view(&self) -> Element {
        let rows = (0..10).map(|_| column([]).height(50.0).into());
        column([scroll(column(rows)).grow(1.0).into()]).into()
    }
}

struct Tall;

impl App for Tall {
    fn view(&self) -> Element {
        let rows = (0..100).map(|_| column([]).height(50.0).bg(Color::BLACK).into());
        column([scroll(column(rows)).grow(1.0).into()]).into()
    }
}

#[test]
fn rows_outside_the_scroll_clip_are_not_painted() {
    let mut rt = Runtime::new(Tall);
    rt.viewport = (100.0, 200.0);
    rt.relayout();
    let painted = |rt: &mut Runtime<Tall>| rt.scene(&mut false).0.len();
    // Background plus the rows within the clip and its overdraw margin.
    assert_eq!(painted(&mut rt), 1 + 5);
    rt.dispatch(Event::Wheel {
        x: 10.0,
        y: 10.0,
        dy: 2000.0,
    });
    let (scene, _) = rt.scene(&mut false);
    assert_eq!(scene.len(), 1 + 6, "rows scrolled into view paint");
}

#[test]
fn wheel_scroll_moves_content_without_relayout() {
    let mut rt = Runtime::new(Long);
    rt.viewport = (100.0, 200.0);
    rt.relayout();
    let row = rt.placed[3].rect;
    rt.measured.clear();
    rt.dispatch(Event::Wheel {
        x: 10.0,
        y: 10.0,
        dy: 30.0,
    });
    // A cleared taffy result forces a full relayout, which must agree.
    assert_eq!(rt.placed[3].rect.y, row.y - 30.0);
    rt.dispatch(Event::Wheel {
        x: 10.0,
        y: 10.0,
        dy: 30.0,
    });
    assert_eq!(rt.placed[3].rect.y, row.y - 60.0);
}

#[test]
fn transitions_request_frames_until_settled() {
    let mut rt = Runtime::new(Counter::default());
    rt.viewport = (200.0, 200.0);
    rt.relayout();
    let t = Instant::now();
    let frame_at = |ms: u64, rt: &mut Runtime<Counter>| {
        task::set_clock(Some(t + Duration::from_millis(ms)));
        let mut frame = false;
        rt.scene(&mut frame);
        frame
    };
    assert!(!frame_at(0, &mut rt));
    rt.dispatch(Event::PointerMove { x: 10.0, y: 10.0 });
    assert!(frame_at(10, &mut rt), "hover fade starts");
    assert!(frame_at(60, &mut rt), "still fading");
    assert!(!frame_at(500, &mut rt), "settled, loop stops");
}

#[test]
fn caret_moves_and_deletes_whole_graphemes() {
    let mut rt = Runtime::new(Field);
    rt.viewport = (200.0, 200.0);
    rt.relayout();
    rt.dispatch(Event::PointerDown { x: 10.0, y: 10.0 });
    rt.dispatch(Event::PointerUp { x: 10.0, y: 10.0 });
    // "a", "e" + combining acute, a flag (two regional indicators), "b".
    rt.dispatch(Event::Text("ae\u{301}\u{1f1ea}\u{1f1f8}b".into()));
    let keys = |rt: &mut Runtime<Field>, keys: &[Key]| {
        for &k in keys {
            rt.dispatch(Event::Key(k));
        }
    };
    // Left twice from the end skips "b", then the whole flag.
    keys(&mut rt, &[Key::Left, Key::Left]);
    rt.dispatch(Event::Text("|".into()));
    assert_eq!(field(&mut rt), "ae\u{301}|\u{1f1ea}\u{1f1f8}b");
    // Backspace removes "|", then "e" together with its accent.
    keys(&mut rt, &[Key::Backspace, Key::Backspace]);
    assert_eq!(field(&mut rt), "a\u{1f1ea}\u{1f1f8}b");
    // Delete removes the whole flag.
    keys(&mut rt, &[Key::Delete]);
    assert_eq!(field(&mut rt), "ab");
}

#[test]
fn selection_extends_with_shift_and_is_replaced_by_edits() {
    let mut rt = Runtime::new(Field);
    rt.viewport = (200.0, 200.0);
    rt.relayout();
    rt.dispatch(Event::PointerDown { x: 10.0, y: 10.0 });
    rt.dispatch(Event::PointerUp { x: 10.0, y: 10.0 });
    rt.dispatch(Event::Text("hello world".into()));
    // Shift+Left x5 selects "world"; typing replaces it.
    rt.mods = ModifiersState::SHIFT;
    for _ in 0..5 {
        rt.dispatch(Event::Key(Key::Left));
    }
    rt.mods = ModifiersState::empty();
    rt.dispatch(Event::Text("there".into()));
    assert_eq!(field(&mut rt), "hello there");
    // Shift+Home selects back to the start; Left collapses to it.
    rt.mods = ModifiersState::SHIFT;
    rt.dispatch(Event::Key(Key::Home));
    rt.mods = ModifiersState::empty();
    rt.dispatch(Event::Key(Key::Left));
    rt.dispatch(Event::Text(">".into()));
    assert_eq!(field(&mut rt), ">hello there");
    // Select all, then Backspace clears it.
    rt.dispatch(Event::Key(Key::SelectAll));
    rt.dispatch(Event::Key(Key::Backspace));
    assert_eq!(field(&mut rt), "");
    // Dragging from the start of new text selects it.
    rt.dispatch(Event::Text("drag".into()));
    rt.dispatch(Event::PointerDown { x: 0.0, y: 10.0 });
    rt.dispatch(Event::PointerMove { x: 190.0, y: 10.0 });
    rt.dispatch(Event::PointerUp { x: 190.0, y: 10.0 });
    rt.dispatch(Event::Text("x".into()));
    assert_eq!(field(&mut rt), "x");
}

#[test]
fn copy_and_cut_take_the_selection() {
    let mut rt = Runtime::new(Field);
    rt.viewport = (200.0, 200.0);
    rt.relayout();
    rt.dispatch(Event::PointerDown { x: 10.0, y: 10.0 });
    rt.dispatch(Event::PointerUp { x: 10.0, y: 10.0 });
    rt.dispatch(Event::Text("héllo wörld".into()));
    rt.dispatch(Event::Key(Key::Copy));
    assert_eq!(rt.clipboard.get(), None, "nothing selected");
    rt.mods = ModifiersState::SHIFT;
    for _ in 0..5 {
        rt.dispatch(Event::Key(Key::Left));
    }
    rt.mods = ModifiersState::empty();
    rt.dispatch(Event::Key(Key::Cut));
    assert_eq!(rt.clipboard.get().as_deref(), Some("wörld"));
    assert_eq!(field(&mut rt), "héllo ");
    rt.dispatch(Event::Key(Key::SelectAll));
    rt.dispatch(Event::Key(Key::Copy));
    assert_eq!(rt.clipboard.get().as_deref(), Some("héllo "));
    assert_eq!(field(&mut rt), "héllo ");
}

#[test]
fn caret_blinks_on_timed_wakeups() {
    let mut rt = Runtime::new(Field);
    rt.viewport = (200.0, 200.0);
    rt.relayout();
    let t = Instant::now();
    let paint_at = |ms: u64, rt: &mut Runtime<Field>| {
        task::set_clock(Some(t + Duration::from_millis(ms)));
        let mut frame = false;
        let n = rt.scene(&mut frame).0.len();
        assert!(!frame, "a blink never asks for every frame");
        n
    };
    let idle = paint_at(0, &mut rt);
    assert_eq!(rt.wake, None, "unfocused fields don't wake");
    rt.dispatch(Event::PointerDown { x: 10.0, y: 10.0 });
    let on = paint_at(100, &mut rt);
    assert_eq!(on, idle + 1, "caret shown");
    assert_eq!(rt.wake, Some(t + Duration::from_millis(600)));
    assert_eq!(paint_at(600, &mut rt), idle, "caret hidden");
    assert_eq!(rt.wake, Some(t + Duration::from_millis(1100)));
    // Typing restarts the cycle with the caret shown.
    rt.dispatch(Event::Text("a".into()));
    let typed = paint_at(700, &mut rt);
    assert_eq!(rt.wake, Some(t + Duration::from_millis(1200)));
    assert!(typed > paint_at(1200, &mut rt), "hidden again 500ms later");
}

#[test]
fn released_drag_flings_and_settles_at_the_edge() {
    let mut rt = Runtime::new(Long);
    rt.viewport = (100.0, 200.0);
    rt.relayout();
    let t = Instant::now();
    let at = |ms: u64| t + Duration::from_millis(ms);
    let offset = |rt: &Runtime<Long>| -rt.placed[3].rect.y;
    let paint_at = |ms: u64, rt: &mut Runtime<Long>| {
        task::set_clock(Some(at(ms)));
        let mut frame = false;
        rt.scene(&mut frame);
        frame
    };
    // Drag up 80px over 64ms: 1250 px/s.
    task::set_clock(Some(at(0)));
    rt.dispatch(Event::PointerDown { x: 50.0, y: 150.0 });
    for i in 1..=4 {
        task::set_clock(Some(at(16 * i)));
        let y = 150.0 - 20.0 * i as f32;
        rt.dispatch(Event::PointerMove { x: 50.0, y });
    }
    rt.dispatch(Event::PointerUp { x: 50.0, y: 70.0 });
    assert_eq!(offset(&rt), 80.0);
    assert!(paint_at(64, &mut rt), "fling animates");
    // 80 + 1250·τ·(1 - e^(-0.1/τ)) at 100ms in, τ = 0.35s.
    let want = 80.0 + 1250.0 * 0.35 * (1.0 - (-0.1f32 / 0.35).exp());
    assert!(paint_at(164, &mut rt));
    assert!(
        (offset(&rt) - want).abs() < 0.5,
        "{} vs {want}",
        offset(&rt)
    );
    // Would travel past the end (300): stops there.
    assert!(!paint_at(2000, &mut rt), "settled");
    assert_eq!(offset(&rt), 300.0);

    // A drag that pauses before lifting doesn't fling.
    task::set_clock(Some(at(3000)));
    rt.dispatch(Event::PointerDown { x: 50.0, y: 50.0 });
    task::set_clock(Some(at(3016)));
    rt.dispatch(Event::PointerMove { x: 50.0, y: 100.0 });
    task::set_clock(Some(at(3300)));
    rt.dispatch(Event::PointerUp { x: 50.0, y: 100.0 });
    assert!(!paint_at(3300, &mut rt));
    assert_eq!(offset(&rt), 250.0);
}

/// A stream fed by hand: items queued, then closed.
#[derive(Clone, Default)]
struct Queue(Rc<RefCell<(VecDeque<u32>, bool)>>);

impl futures_core::Stream for Queue {
    type Item = u32;
    fn poll_next(self: Pin<&mut Self>, _: &mut Context) -> Poll<Option<u32>> {
        let mut q = self.0.borrow_mut();
        match q.0.pop_front() {
            Some(v) => Poll::Ready(Some(v)),
            None if q.1 => Poll::Ready(None),
            None => Poll::Pending,
        }
    }
}

#[derive(Default)]
struct Feed {
    queue: Queue,
    run: Cell<u32>,
    hidden: Cell<bool>,
}

impl App for Feed {
    fn view(&self) -> Element {
        if self.hidden.get() {
            return column([]).into();
        }
        let feed = stream_builder(
            self.run.get(),
            |s: Scope| s.get::<Queue>().unwrap().clone(),
            |snap: &Snapshot<Vec<u32>>| text(format!("{:?}", snap.data)),
        );
        provide(self.queue.clone(), feed).into()
    }
}

fn snap(rt: &mut Runtime<Feed>) -> (Vec<u32>, bool) {
    let id = rt.ids[1];
    let s = rt.states.get_mut(&id).unwrap().get::<Snapshot<Vec<u32>>>();
    (s.data.clone(), s.done)
}

#[test]
fn stream_builder_collects_restarts_and_cancels() {
    let mut rt = Runtime::new(Feed::default());
    assert!(!rt.drive(), "nothing queued yet");
    rt.app.queue.0.borrow_mut().0.extend([1, 2]);
    assert!(rt.drive());
    assert_eq!(snap(&mut rt), (vec![1, 2], false));

    rt.app.run.set(1);
    rt.rebuild();
    assert_eq!(snap(&mut rt), (vec![], false), "new deps start fresh");
    rt.app.queue.0.borrow_mut().0.push_back(3);
    rt.app.queue.0.borrow_mut().1 = true;
    assert!(rt.drive());
    assert_eq!(snap(&mut rt), (vec![3], true));
    rt.rebuild();
    assert!(!rt.drive(), "a finished stream isn't restarted");

    rt.app.hidden.set(true);
    rt.rebuild();
    assert!(rt.tasks.is_empty());
    assert_eq!(Rc::strong_count(&rt.app.queue.0), 1, "stream dropped");
}

struct Answer;

impl App for Answer {
    fn view(&self) -> Element {
        let answer = future_builder(
            (),
            |s: Scope| {
                let x = *s.get::<u32>().unwrap();
                async move { x * 2 }
            },
            |snap: &Snapshot<Option<u32>>| text(format!("{:?}", snap.data)),
        );
        provide(21u32, answer).into()
    }
}

#[test]
fn future_builder_reads_providers_and_resolves() {
    let mut rt = Runtime::new(Answer);
    assert!(rt.drive());
    let id = rt.ids[1];
    let s = rt
        .states
        .get_mut(&id)
        .unwrap()
        .get::<Snapshot<Option<u32>>>();
    assert_eq!((s.data, s.done), (Some(42), true));
}

struct Later;

impl App for Later {
    fn view(&self) -> Element {
        future_builder(
            (),
            |_| async {
                crate::sleep(Duration::from_millis(20)).await;
                7
            },
            |snap: &Snapshot<Option<u32>>| text(format!("{:?}", snap.data)),
        )
        .into()
    }
}

#[test]
fn sleep_resolves_after_its_deadline() {
    let mut rt = Runtime::new(Later);
    assert!(!rt.drive());
    let due = task::fire_timers(Instant::now()).expect("timer registered");
    assert!(!rt.drive(), "polling again doesn't finish early");
    std::thread::sleep(due - Instant::now());
    assert_eq!(task::fire_timers(Instant::now()), None, "fired and removed");
    assert!(rt.drive());
    let id = rt.ids[0];
    let s = rt
        .states
        .get_mut(&id)
        .unwrap()
        .get::<Snapshot<Option<u32>>>();
    assert_eq!(s.data, Some(7));
}

struct Secret;

impl App for Secret {
    fn view(&self) -> Element {
        stateful(|v: &String, h| {
            text_input(v.clone())
                .password()
                .on_change(move |cx, v| cx.update(h, |s: &mut String| *s = v))
        })
        .into()
    }
}

#[test]
fn password_keeps_the_value_but_doesnt_copy_it() {
    let mut rt = Runtime::new(Secret);
    rt.viewport = (200.0, 200.0);
    rt.relayout();
    rt.dispatch(Event::PointerDown { x: 10.0, y: 10.0 });
    rt.dispatch(Event::PointerUp { x: 10.0, y: 10.0 });
    rt.dispatch(Event::Text("hunter2".into()));
    assert_eq!(root::<String, _>(&mut rt), "hunter2");
    rt.dispatch(Event::Key(Key::SelectAll));
    rt.dispatch(Event::Key(Key::Copy));
    assert_eq!(rt.clipboard.get(), None);
}

struct Title;

impl App for Title {
    fn view(&self) -> Element {
        column([text("A title far too long for the window")
            .max_lines(1)
            .into()])
        .into()
    }
}

#[test]
fn max_lines_keeps_a_label_to_one_line() {
    let mut rt = Runtime::new(Title);
    rt.viewport = (100.0, 200.0);
    rt.relayout();
    let line = rt.text.line_height(Font::DEFAULT, 16.0);
    let r = rt.placed[1].rect;
    assert!(r.h <= line.ceil() && r.w <= 100.0, "{r:?}");
}

struct Tapper(Shape);

impl App for Tapper {
    fn view(&self) -> Element {
        column([icon_button(&self.0).on_click(|_| {}).into()]).into()
    }
}

#[test]
fn icon_button_shrinks_while_pressed_and_overshoots_on_release() {
    let sq = Path::polygon(&[[0.0, 0.0], [24.0, 0.0], [24.0, 24.0], [0.0, 24.0]]);
    let mut rt = Runtime::new(Tapper(Shape::new(sq, Rect::new(0.0, 0.0, 24.0, 24.0))));
    rt.viewport = (200.0, 200.0);
    rt.relayout();
    let t = Instant::now();
    let width = |rt: &mut Runtime<Tapper>, ms: u64| {
        task::set_clock(Some(t + Duration::from_millis(ms)));
        let (scene, _) = rt.scene(&mut false);
        scene.instances[1].rect[2]
    };
    let rest = width(&mut rt, 0);
    rt.dispatch(Event::PointerDown { x: 20.0, y: 20.0 });
    width(&mut rt, 10);
    assert!(width(&mut rt, 200) < rest * 0.85, "pressed");
    task::set_clock(Some(t + Duration::from_millis(200)));
    rt.dispatch(Event::PointerUp { x: 20.0, y: 20.0 });
    width(&mut rt, 210);
    assert!(width(&mut rt, 420) > rest * 1.04, "springs past its size");
    assert!((width(&mut rt, 2000) - rest).abs() < 0.01, "settles");
}

struct Watched {
    notifier: crate::Notifier,
    count: std::sync::Arc<std::sync::atomic::AtomicU32>,
}

impl App for Watched {
    fn view(&self) -> Element {
        let count = self.count.clone();
        watch(&self.notifier, move || {
            let n = count.load(std::sync::atomic::Ordering::Relaxed);
            text(n.to_string())
        })
        .into()
    }
}

#[test]
fn watch_rebuilds_when_another_thread_notifies() {
    use std::sync::atomic::Ordering::Relaxed;
    let app = Watched {
        notifier: crate::Notifier::new(),
        count: Default::default(),
    };
    let (n, count) = (app.notifier.clone(), app.count.clone());
    let mut rt = Runtime::new(app);
    assert!(!rt.drive(), "nothing to rebuild yet");
    for i in 1..=2 {
        let (n, count) = (n.clone(), count.clone());
        std::thread::spawn(move || {
            count.store(i, Relaxed);
            n.notify();
            n.notify();
        })
        .join()
        .unwrap();
        assert!(rt.drive(), "notify {i} rebuilds");
        assert!(!rt.drive(), "a burst is one rebuild");
    }
}
