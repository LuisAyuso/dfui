use super::*;
use crate::render::{damage, render_offscreen};
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll};

use crate::Theme;
use crate::path::{Path, Shape};
use crate::text::Font;
use crate::widget::{Handle, PaintCx, Scope, Widget};
use crate::widgets::{
    Snapshot, Styled, button, column, dialog, dropdown, future_builder, icon, icon_button, on_back,
    on_key, overlays, popup, provide, radio, row, scroll, segmented, slider, snackbar, spacer,
    stateful, stream_builder, text, text_input, toggle, tooltip, watch,
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

/// Two screens whose fields land in the same tree slot, so share an id.
struct Screens;

#[derive(Default)]
struct TwoFields {
    second: bool,
    a: String,
    b: String,
}

impl App for Screens {
    fn view(&self) -> Element {
        stateful(|s: &TwoFields, h| match s.second {
            false => text_input(s.a.clone())
                .on_change(move |cx, v| cx.update(h, |s: &mut TwoFields| s.a = v)),
            true => text_input(s.b.clone())
                .on_change(move |cx, v| cx.update(h, |s: &mut TwoFields| s.b = v)),
        })
        .into()
    }
}

#[test]
fn text_input_types_at_the_end_of_text_it_did_not_write() {
    let mut rt = Runtime::new(Screens);
    rt.viewport = (200.0, 200.0);
    rt.relayout();
    rt.dispatch(Event::PointerDown { x: 10.0, y: 10.0 });
    rt.dispatch(Event::PointerUp { x: 10.0, y: 10.0 });
    rt.dispatch(Event::Text("abc".into()));
    rt.dispatch(Event::Key(Key::Home));
    let s = root::<TwoFields, _>(&mut rt);
    (s.second, s.b) = (true, "hello".into());
    rt.rebuild();
    rt.dispatch(Event::Text("x".into()));
    assert_eq!(root::<TwoFields, _>(&mut rt).b, "hellox");
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

#[test]
fn tapping_the_focused_input_requests_the_keyboard_again() {
    let mut rt = Runtime::new(Field);
    rt.viewport = (200.0, 200.0);
    rt.relayout();
    for y in [10.0, 10.0, 190.0, 190.0] {
        rt.dispatch(Event::PointerDown { x: 10.0, y });
    }
    let text = Some(Keyboard::Text);
    assert_eq!(rt.ime_requests, [text, text, None]);
}

struct ScrollInOverlays;

impl App for ScrollInOverlays {
    fn view(&self) -> Element {
        overlays(|_| {
            column([
                scroll(spacer().height(1000.0)).grow(1.0).into(),
                button(text("after")).height(40.0).into(),
            ])
            .fill()
        })
        .fill()
        .into()
    }
}

#[test]
fn scroll_inside_overlays_keeps_later_widgets_on_screen() {
    let mut rt = Runtime::new(ScrollInOverlays);
    rt.viewport = (200.0, 200.0);
    rt.relayout();
    let last = rt.placed.last().unwrap().rect;
    assert!(last.y + last.h <= 200.0, "pushed off screen: {last:?}");
}

struct EqualRow;

impl App for EqualRow {
    fn view(&self) -> Element {
        row([
            button(text("a")).fill().into(),
            button(text("a longer one")).fill().into(),
        ])
        .into()
    }
}

#[test]
fn filling_siblings_in_a_row_get_equal_widths() {
    let mut rt = Runtime::new(EqualRow);
    rt.viewport = (300.0, 100.0);
    rt.relayout();
    let w: Vec<f32> = rt.placed.iter().map(|p| p.rect.w).collect();
    assert_eq!((w[1], w[3]), (150.0, 150.0), "{w:?}");
}

struct Menu;

impl App for Menu {
    fn view(&self) -> Element {
        stateful(|&v: &usize, h| {
            dropdown((0..30).map(|i| i.to_string()), v)
                .on_change(move |cx, i| cx.update(h, move |s: &mut usize| *s = i))
        })
        .into()
    }
}

#[test]
fn long_dropdown_scrolls_to_its_last_option() {
    let mut rt = Runtime::new(Menu);
    rt.viewport = (200.0, 800.0);
    let t = Instant::now();
    task::set_clock(Some(t));
    rt.relayout();
    rt.dispatch(Event::PointerDown { x: 10.0, y: 10.0 });
    rt.dispatch(Event::PointerUp { x: 10.0, y: 10.0 });
    task::set_clock(Some(t + Duration::from_secs(1)));
    rt.reposition();
    let menu = rt.placed.iter().find(|p| p.rect.h == 320.0).unwrap().rect;
    let (x, y) = (menu.x + 10.0, menu.y + menu.h - 10.0);
    rt.dispatch(Event::PointerDown { x, y });
    for dy in [50.0, 5000.0] {
        rt.dispatch(Event::PointerMove { x, y: y - dy });
    }
    rt.dispatch(Event::PointerUp { x, y: y - 5000.0 });
    assert_eq!(*root::<usize, _>(&mut rt), 0, "a drag doesn't choose");
    rt.dispatch(Event::PointerDown { x, y });
    rt.dispatch(Event::PointerUp { x, y });
    task::set_clock(None);
    assert_eq!(*root::<usize, _>(&mut rt), 29);
}

struct Disabled;

impl App for Disabled {
    fn view(&self) -> Element {
        stateful(|_: &i32, h| {
            let bump = move |cx: &mut EventCx| cx.update(h, |n: &mut i32| *n += 1);
            column([
                button(text("+"))
                    .enabled(false)
                    .on_click(bump)
                    .height(40.0)
                    .into(),
                icon(&Shape::new(
                    Path::polygon(&[[0.0, 0.0], [24.0, 0.0], [0.0, 24.0]]),
                    Rect::new(0.0, 0.0, 24.0, 24.0),
                ))
                .enabled(false)
                .on_click(bump)
                .into(),
            ])
        })
        .into()
    }
}

#[test]
fn disabled_button_and_icon_ignore_taps() {
    let mut rt = Runtime::new(Disabled);
    rt.viewport = (200.0, 200.0);
    rt.relayout();
    for y in [10.0, 50.0] {
        rt.dispatch(Event::PointerDown { x: 10.0, y });
        rt.dispatch(Event::PointerUp { x: 10.0, y });
    }
    assert_eq!(*root::<i32, _>(&mut rt), 0);
}

struct BackCounter;

impl App for BackCounter {
    fn view(&self) -> Element {
        stateful(|_: &i32, h| {
            column([on_back(move |cx| cx.update(h, |n: &mut i32| *n += 1)).into()])
        })
        .into()
    }
}

#[test]
fn on_back_runs_and_keeps_the_app_open() {
    let mut rt = Runtime::new(BackCounter);
    rt.viewport = (200.0, 200.0);
    rt.relayout();
    assert!(rt.dispatch(Event::Back), "handled, so Android doesn't exit");
    assert_eq!(*root::<i32, _>(&mut rt), 1);
}

struct Keys;

impl App for Keys {
    fn view(&self) -> Element {
        stateful(|_: &Vec<Key>, h| {
            column([
                on_key(move |cx, k| cx.update(h, move |v: &mut Vec<Key>| v.push(k))).into(),
                text_input("").into(),
            ])
        })
        .into()
    }
}

#[test]
fn keys_reach_on_key_unless_a_field_is_focused() {
    let mut rt = Runtime::new(Keys);
    rt.viewport = (200.0, 200.0);
    rt.relayout();
    rt.dispatch(Event::Key(Key::Down));
    rt.key_pressed(&WKey::Named(NamedKey::Enter), None);
    rt.key_pressed(&WKey::Character("q".into()), Some("q"));
    assert_eq!(
        *root::<Vec<Key>, _>(&mut rt),
        [Key::Down, Key::Enter, Key::Char('q')]
    );
    let field = rt.placed.last().unwrap().rect;
    let (x, y) = (field.x + 5.0, field.y + 5.0);
    rt.dispatch(Event::PointerDown { x, y });
    rt.dispatch(Event::PointerUp { x, y });
    rt.dispatch(Event::Key(Key::Left));
    rt.key_pressed(&WKey::Character("q".into()), Some("q"));
    assert_eq!(
        root::<Vec<Key>, _>(&mut rt).len(),
        3,
        "the focused field took them"
    );
    rt.dispatch(Event::Key(Key::Up));
    assert_eq!(
        root::<Vec<Key>, _>(&mut rt)[3],
        Key::Up,
        "but lets Up through"
    );
}

struct TwoButtons;

impl App for TwoButtons {
    fn view(&self) -> Element {
        stateful(|_: &Vec<u8>, h| {
            let press =
                move |n: u8| move |cx: &mut EventCx| cx.update(h, move |v: &mut Vec<u8>| v.push(n));
            column([
                button(text("a")).on_click(press(1)).into(),
                text("not focusable").into(),
                button(text("b")).on_click(press(2)).into(),
            ])
        })
        .into()
    }
}

#[test]
fn tab_walks_focusable_widgets_and_enter_or_space_presses_them() {
    let mut rt = Runtime::new(TwoButtons);
    rt.viewport = (200.0, 200.0);
    rt.relayout();
    let tab = |rt: &mut Runtime<TwoButtons>| rt.key_pressed(&WKey::Named(NamedKey::Tab), None);
    let enter = WKey::Named(NamedKey::Enter);
    tab(&mut rt);
    rt.key_pressed(&enter, None);
    tab(&mut rt);
    rt.key_pressed(&WKey::Character(" ".into()), Some(" "));
    // Past the last one it wraps; Shift+Tab goes back.
    tab(&mut rt);
    rt.key_pressed(&enter, None);
    rt.mods = ModifiersState::SHIFT;
    tab(&mut rt);
    rt.key_pressed(&enter, None);
    assert_eq!(*root::<Vec<u8>, _>(&mut rt), [1, 2, 1, 2]);
    assert!(rt.focus_visible, "the ring shows after Tab");
    rt.dispatch(Event::PointerDown { x: 199.0, y: 199.0 });
    assert_eq!(rt.focused, None, "a tap elsewhere drops focus");
}

struct Log;

impl App for Log {
    fn view(&self) -> Element {
        stateful(|&extra: &f32, _| scroll(spacer().height(1000.0 + extra)).stick_to_end()).into()
    }
}

#[test]
fn stick_to_end_follows_until_scrolled_away() {
    let mut rt = Runtime::new(Log);
    rt.viewport = (200.0, 200.0);
    rt.relayout();
    let grow = |rt: &mut Runtime<Log>| {
        *root::<f32, _>(rt) += 100.0;
        rt.rebuild();
        rt.relayout();
        let last = rt.placed.last().unwrap().rect;
        last.y + last.h
    };
    let wheel = |rt: &mut Runtime<Log>, dy| {
        rt.dispatch(Event::Wheel {
            x: 10.0,
            y: 10.0,
            dx: 0.0,
            dy,
        });
    };
    assert_eq!(grow(&mut rt), 200.0, "follows");
    wheel(&mut rt, -50.0);
    assert_eq!(grow(&mut rt), 350.0, "stays where the user went");
    wheel(&mut rt, 1e5);
    assert_eq!(grow(&mut rt), 200.0, "follows again");
}

struct SelectedMenu;

impl App for SelectedMenu {
    fn view(&self) -> Element {
        dropdown((0..30).map(|i| i.to_string()), 25).into()
    }
}

#[test]
fn long_dropdown_opens_at_the_selected_option() {
    let mut rt = Runtime::new(SelectedMenu);
    rt.viewport = (200.0, 800.0);
    let t = Instant::now();
    task::set_clock(Some(t));
    rt.relayout();
    rt.dispatch(Event::PointerDown { x: 10.0, y: 10.0 });
    rt.dispatch(Event::PointerUp { x: 10.0, y: 10.0 });
    task::set_clock(Some(t + Duration::from_secs(1)));
    rt.reposition();
    task::set_clock(None);
    let k = rt.placed.iter().position(|p| p.rect.h == 320.0).unwrap();
    let (menu, row) = (rt.placed[k].rect, rt.placed[k + 2 + 2 * 25].rect);
    assert!(
        row.y >= menu.y && row.y + row.h <= menu.y + menu.h,
        "{row:?} outside {menu:?}"
    );
}

#[derive(Clone, Default)]
struct AccentProbe(Rc<Cell<Color>>);

impl Widget for AccentProbe {
    fn style(&self) -> taffy::Style {
        taffy::Style {
            size: taffy::Size::from_lengths(10.0, 10.0),
            ..taffy::Style::default()
        }
    }
    fn paint(&self, cx: &mut PaintCx) {
        self.0.set(cx.theme.accent);
    }
}

struct Themed([AccentProbe; 3]);

const APP_ACCENT: Color = Color::hex(0x112233);
const INNER_ACCENT: Color = Color::hex(0x445566);

impl App for Themed {
    fn view(&self) -> Element {
        let [a, b, c] = self.0.clone();
        let inner = Theme {
            accent: INNER_ACCENT,
            ..Theme::default()
        };
        column([
            Element::new(a),
            provide(inner, column([Element::new(b)])).into(),
            Element::new(c),
        ])
        .into()
    }
    fn theme(&self) -> Theme {
        Theme {
            accent: APP_ACCENT,
            ..Theme::default()
        }
    }
}

#[test]
fn a_provided_theme_applies_only_inside_its_subtree() {
    let probes: [AccentProbe; 3] = Default::default();
    let mut rt = Runtime::new(Themed(probes.clone()));
    rt.viewport = (200.0, 200.0);
    rt.relayout();
    rt.scene(&mut false);
    let seen = probes.map(|p| p.0.get());
    assert_eq!(seen, [APP_ACCENT, INNER_ACCENT, APP_ACCENT]);
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

struct Label(Cell<&'static str>);

impl App for Label {
    fn view(&self) -> Element {
        text(self.0.get()).into()
    }
}

#[test]
#[ignore = "needs a GPU"]
fn outlines_stored_after_the_first_frame_reach_the_gpu() {
    let mut rt = Runtime::new(Label(Cell::new("a")));
    rt.viewport = (40.0, 40.0);
    rt.relayout();
    let mut off = crate::render::Offscreen::new((40, 40));
    let (a, _) = rt.scene(&mut false);
    off.draw(&a, &mut rt.text, 1.0, None);
    let first = off.read();
    rt.app.0.set("bq");
    rt.rebuild();
    let (b, _) = rt.scene(&mut false);
    off.draw(&b, &mut rt.text, 1.0, None);
    let fresh = render_offscreen(&[(&b, None)], &mut rt.text, (40, 40), 1.0);
    assert!(off.read() != first, "the new glyphs drew");
    assert!(off.read() == fresh, "same as uploading everything");
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

struct ScreenApp;

impl App for ScreenApp {
    fn view(&self) -> Element {
        column([
            crate::widgets::screen(column([]).height(10.0)).into(),
            crate::widgets::overlay(column([]).height(10.0)).into(),
            crate::widgets::backdrop(column([]).grow(1.0)).into(),
        ])
        .into()
    }
}

#[test]
fn screen_layers_reach_under_the_bars() {
    let mut rt = Runtime::new(ScreenApp);
    rt.viewport = (200.0, 200.0);
    rt.insets = [20.0, 0.0, 10.0, 0.0];
    rt.relayout();
    // Pre-order: column, screen, its sink, content, overlay, sink, content.
    assert_eq!(rt.placed[1].rect, Rect::new(0.0, 0.0, 200.0, 200.0));
    assert_eq!(rt.placed[4].rect, Rect::new(0.0, 20.0, 200.0, 170.0));
    assert_eq!(rt.placed[3].rect.h, 10.0, "screen content hugs itself");
    assert_eq!(
        rt.placed[9].rect,
        Rect::new(0.0, 0.0, 200.0, 200.0),
        "backdrop fills"
    );
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
        dx: 0.0,
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

#[test]
fn cancelled_touch_ends_a_tooltip_press() {
    let mut rt = Runtime::new(Tip);
    rt.viewport = (200.0, 200.0);
    let t = Instant::now();
    task::set_clock(Some(t));
    rt.relayout();
    let tip = rt.placed.len() - 1;
    let at = |rt: &mut Runtime<Tip>, d: u64| {
        task::set_clock(Some(t + Duration::from_millis(d)));
        rt.reposition();
        rt.placed[tip].rect.y.abs() < 1e5
    };
    rt.touch(1, TouchPhase::Started, 5.0, 5.0);
    assert!(!at(&mut rt, 100), "not held long enough yet");
    rt.touch(1, TouchPhase::Cancelled, 5.0, 5.0);
    assert!(!at(&mut rt, 800), "a cancelled press never shows it");

    rt.touch(1, TouchPhase::Started, 5.0, 5.0);
    assert!(at(&mut rt, 1600), "held long enough");
    rt.touch(1, TouchPhase::Cancelled, 5.0, 5.0);
    assert!(!at(&mut rt, 60_000), "hides without another touch");
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
        dx: 0.0,
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
        dx: 0.0,
        dy: 30.0,
    });
    // A cleared taffy result forces a full relayout, which must agree.
    assert_eq!(rt.placed[3].rect.y, row.y - 30.0);
    rt.dispatch(Event::Wheel {
        x: 10.0,
        y: 10.0,
        dx: 0.0,
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

/// A fling over rows from `row`, caught by a press; returns the state after.
struct FlingOver(fn(Handle<i32>) -> Element);

impl App for FlingOver {
    fn view(&self) -> Element {
        let row = self.0;
        stateful(move |_: &i32, h| scroll(column((0..20).map(|_| row(h)))).height(200.0)).into()
    }
}

fn flinging(row: fn(Handle<i32>) -> Element) -> Runtime<FlingOver> {
    let mut rt = Runtime::new(FlingOver(row));
    rt.viewport = (100.0, 200.0);
    rt.relayout();
    let t = Instant::now();
    task::set_clock(Some(t));
    rt.dispatch(Event::PointerDown { x: 50.0, y: 150.0 });
    for i in 1..=4 {
        task::set_clock(Some(t + Duration::from_millis(16 * i)));
        let y = 150.0 - 20.0 * i as f32;
        rt.dispatch(Event::PointerMove { x: 50.0, y });
    }
    rt.dispatch(Event::PointerUp { x: 50.0, y: 70.0 });
    rt
}

#[test]
fn press_that_catches_a_fling_does_not_tap() {
    let mut rt = flinging(|h| {
        button(text("x"))
            .on_click(move |cx| cx.update(h, |n: &mut i32| *n += 1))
            .height(50.0)
            .into()
    });
    rt.dispatch(Event::PointerDown { x: 50.0, y: 100.0 });
    rt.dispatch(Event::PointerUp { x: 50.0, y: 100.0 });
    assert_eq!(
        *root::<i32, _>(&mut rt),
        0,
        "the press only stops the fling"
    );
    rt.dispatch(Event::PointerDown { x: 50.0, y: 100.0 });
    rt.dispatch(Event::PointerUp { x: 50.0, y: 100.0 });
    assert_eq!(*root::<i32, _>(&mut rt), 1, "the next tap lands");
}

#[test]
fn press_that_catches_a_fling_does_not_focus() {
    let mut rt = flinging(|_| text_input("x").height(50.0).into());
    rt.dispatch(Event::PointerDown { x: 50.0, y: 100.0 });
    assert_eq!(rt.focused, None);
}

#[test]
fn delete_before_counts_utf16_units_and_takes_whole_graphemes() {
    let mut rt = Runtime::new(Field);
    rt.viewport = (200.0, 200.0);
    rt.relayout();
    rt.dispatch(Event::PointerDown { x: 10.0, y: 10.0 });
    rt.dispatch(Event::PointerUp { x: 10.0, y: 10.0 });
    rt.dispatch(Event::Text("abe\u{301}\u{1f600}".into()));
    rt.dispatch(Event::Key(Key::DeleteBefore(2)));
    assert_eq!(
        root::<String, _>(&mut rt),
        "abe\u{301}",
        "one emoji, two units"
    );
    rt.dispatch(Event::Key(Key::DeleteBefore(1)));
    assert_eq!(
        root::<String, _>(&mut rt),
        "ab",
        "the whole accented letter"
    );
    rt.dispatch(Event::Key(Key::DeleteBefore(9)));
    assert_eq!(root::<String, _>(&mut rt), "", "stops at the start");
}

mod fast_sources {
    use super::*;
    use std::sync::mpsc::channel;
    use std::time::Duration;

    #[derive(Default)]
    struct Discard;

    impl Extend<()> for Discard {
        fn extend<I: IntoIterator<Item = ()>>(&mut self, items: I) {
            items.into_iter().for_each(drop);
        }
    }

    struct AlwaysReady;

    impl futures_core::Stream for AlwaysReady {
        type Item = ();
        fn poll_next(self: Pin<&mut Self>, _: &mut Context) -> Poll<Option<()>> {
            Poll::Ready(Some(()))
        }
    }

    #[derive(Default)]
    struct SelfWaking(bool);

    impl futures_core::Stream for SelfWaking {
        type Item = ();
        fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Option<()>> {
            self.0 = !self.0;
            if self.0 {
                return Poll::Ready(Some(()));
            }
            cx.waker().wake_by_ref();
            Poll::Pending
        }
    }

    struct Streamed<St>(fn() -> St);

    impl<St: futures_core::Stream<Item = ()> + 'static> App for Streamed<St> {
        fn view(&self) -> Element {
            let start = self.0;
            stream_builder(0, move |_| start(), |_: &Snapshot<Discard>| text("x")).into()
        }
    }

    /// Notified during every rebuild, like a producer faster than `view`.
    struct NotifiedInView(crate::Notifier);

    impl App for NotifiedInView {
        fn view(&self) -> Element {
            self.0.notify();
            watch(&self.0, || text("x")).into()
        }
    }

    fn drive_returns<A: App>(make: impl FnOnce() -> A + Send + 'static) -> bool {
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            Runtime::new(make()).drive();
            let _ = tx.send(());
        });
        rx.recv_timeout(Duration::from_secs(10)).is_ok()
    }

    #[test]
    fn always_ready_stream_returns() {
        assert!(drive_returns(|| Streamed(|| AlwaysReady)));
    }

    #[test]
    fn self_waking_stream_returns() {
        assert!(drive_returns(|| Streamed(SelfWaking::default)));
    }

    #[test]
    fn updates_during_a_rebuild_wait_for_the_next_wake() {
        assert!(drive_returns(|| NotifiedInView(crate::Notifier::new())));
    }
}

thread_local! {
    static SEEN: RefCell<Vec<Event>> = const { RefCell::new(Vec::new()) };
}

struct Recorder;

impl Widget for Recorder {
    fn style(&self) -> taffy::Style {
        taffy::Style {
            size: taffy::Size::from_lengths(200.0, 200.0),
            ..Default::default()
        }
    }
    fn event(&self, ev: &Event, _: &mut EventCx) {
        SEEN.with(|s| s.borrow_mut().push(ev.clone()));
    }
}

struct Pinchy;

impl App for Pinchy {
    fn view(&self) -> Element {
        Element::new(Recorder)
    }
}

#[test]
fn two_fingers_pinch_and_keep_the_pointer_steady() {
    let mut rt = Runtime::new(Pinchy);
    rt.viewport = (200.0, 200.0);
    rt.relayout();
    rt.touch(1, TouchPhase::Started, 50.0, 50.0);
    rt.touch(2, TouchPhase::Started, 150.0, 50.0);
    rt.touch(2, TouchPhase::Moved, 250.0, 50.0);
    rt.touch(1, TouchPhase::Ended, 50.0, 50.0);
    rt.touch(2, TouchPhase::Moved, 260.0, 50.0);
    rt.touch(2, TouchPhase::Ended, 260.0, 50.0);
    let seen = SEEN.with(|s| s.take());
    assert_eq!(
        seen,
        [
            Event::PointerDown { x: 50.0, y: 50.0 },
            // Midpoint moved 50 px: the pointer follows by as much.
            Event::PointerMove { x: 100.0, y: 50.0 },
            Event::Pinch {
                x: 150.0,
                y: 50.0,
                scale: 2.0
            },
            Event::PointerMove { x: 110.0, y: 50.0 },
            Event::PointerUp { x: 110.0, y: 50.0 },
            Event::Leave,
        ]
    );
}

thread_local! {
    /// The item `Shelves` keeps in view, by index; none when negative.
    static REVEAL: Cell<i32> = const { Cell::new(-1) };
}

/// A page that scrolls down, with a row of ten 80 px items that scrolls sideways.
struct Shelves;

impl App for Shelves {
    fn view(&self) -> Element {
        let items = (0..10).map(|_| {
            column([])
                .width(80.0)
                .height(50.0)
                .shrink(0.0)
                .bg(Color::BLACK)
                .into()
        });
        let mut shelf = scroll(row(items)).horizontal();
        if let Ok(i) = u32::try_from(REVEAL.get()) {
            let x = i as f32 * 80.0;
            shelf = shelf.reveal((x, x + 80.0));
        }
        let rows = (0..20).map(|_| column([]).height(50.0).into());
        let page = column(std::iter::once(shelf.height(50.0).into()).chain(rows));
        column([scroll(page).grow(1.0).into()]).into()
    }
}

/// The first shelf item's top-left corner.
fn first_item(rt: &Runtime<Shelves>) -> (f32, f32) {
    let r = rt
        .placed
        .iter()
        .map(|p| p.rect)
        .find(|r| (r.w, r.h) == (80.0, 50.0))
        .unwrap();
    (r.x, r.y)
}

fn wheel(dx: f32, dy: f32) -> Event {
    Event::Wheel {
        x: 10.0,
        y: 10.0,
        dx,
        dy,
    }
}

#[test]
fn a_horizontal_scroll_takes_sideways_wheels_and_passes_vertical_ones_on() {
    REVEAL.set(-1);
    let mut rt = Runtime::new(Shelves);
    rt.viewport = (200.0, 200.0);
    rt.relayout();
    assert_eq!(first_item(&rt), (0.0, 0.0));
    rt.dispatch(wheel(100.0, 0.0));
    assert_eq!(
        first_item(&rt),
        (-100.0, 0.0),
        "the row moved, not the page"
    );
    rt.dispatch(wheel(0.0, 30.0));
    assert_eq!(
        first_item(&rt),
        (-100.0, -30.0),
        "the page moved, not the row"
    );
    // Past the end stops at the last item: 800 px of items in a 200 px box.
    rt.dispatch(wheel(5000.0, 0.0));
    assert_eq!(first_item(&rt).0, -600.0);
}

#[test]
fn dragging_a_horizontal_scroll_moves_only_the_row() {
    REVEAL.set(-1);
    let mut rt = Runtime::new(Shelves);
    rt.viewport = (200.0, 200.0);
    rt.relayout();
    rt.dispatch(Event::PointerDown { x: 150.0, y: 25.0 });
    for x in [140.0, 120.0, 100.0, 80.0] {
        rt.dispatch(Event::PointerMove { x, y: 25.0 });
    }
    rt.dispatch(Event::PointerUp { x: 80.0, y: 25.0 });
    let (x, y) = first_item(&rt);
    assert!(x < -40.0, "followed the finger: {x}");
    assert_eq!(y, 0.0);
}

#[test]
fn reveal_glides_to_the_focused_item_and_then_lets_go() {
    let t = Instant::now();
    task::set_clock(Some(t));
    REVEAL.set(0);
    let mut rt = Runtime::new(Shelves);
    rt.viewport = (200.0, 200.0);
    rt.relayout();
    assert_eq!(first_item(&rt).0, 0.0);
    // Item 5 spans 400..480: the least move that shows it puts its end at 200.
    REVEAL.set(5);
    rt.rebuild();
    rt.relayout();
    task::set_clock(Some(t + Duration::from_millis(500)));
    rt.reposition();
    assert_eq!(first_item(&rt).0, -280.0);
    // The user's own scrolling stays while the focus doesn't move.
    rt.dispatch(wheel(-100.0, 0.0));
    rt.rebuild();
    rt.relayout();
    assert_eq!(first_item(&rt).0, -180.0);
    task::set_clock(None);
}
