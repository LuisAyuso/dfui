//! The patterns apps are built from, driven through input events like a user.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use dfui::model::Keyed;
use dfui::widgets::{
    Overlays, Styled, button, column, dialog, dropdown, future_builder, list, on_back, overlays,
    row, scroll, stateful, stateful_with, text, text_input, watch,
};
use dfui::{App, Bench, Element, Event, EventCx, Handle, Notifier, sleep};

thread_local! {
    static LOG: RefCell<Vec<String>> = RefCell::default();
}

fn log(s: impl Into<String>) {
    LOG.with(|l| l.borrow_mut().push(s.into()));
}

fn logged() -> Vec<String> {
    LOG.with(|l| l.take())
}

/// Longer than any transition. `settle` never returns with a focused input,
/// whose caret keeps blinking.
const SETTLED: Duration = Duration::from_millis(400);

struct View(Box<dyn Fn() -> Element>);

impl App for View {
    fn view(&self) -> Element {
        (self.0)()
    }
}

fn run(view: impl Fn() -> Element + 'static, size: (f32, f32)) -> Bench<View> {
    let mut b = Bench::new(View(Box::new(view)), size);
    b.settle();
    b
}

fn tap(b: &mut Bench<View>, x: f32, y: f32) {
    b.event(Event::PointerDown { x, y });
    b.event(Event::PointerUp { x, y });
    b.advance(SETTLED);
}

fn tall(label: &str, f: impl Fn(&mut EventCx) + 'static) -> Element {
    button(text(label)).on_click(f).height(40.0).into()
}

#[test]
fn counter_rebuilds_from_its_state() {
    let mut b = run(
        || {
            stateful(|n: &i32, h| {
                log(format!("count {n}"));
                tall("+", move |cx| cx.update(h, |n: &mut i32| *n += 1))
            })
            .into()
        },
        (200.0, 100.0),
    );
    logged();
    tap(&mut b, 20.0, 20.0);
    tap(&mut b, 20.0, 20.0);
    assert_eq!(logged().last().unwrap(), "count 2");
}

struct Form {
    name: String,
}

#[test]
fn form_starts_from_data_and_saves_only_when_valid() {
    let view = || -> Element {
        stateful_with(
            || Form { name: "Ann".into() },
            |f: &Form, h| {
                let name = f.name.clone();
                column([
                    text_input(f.name.clone())
                        .on_change(move |cx, v| cx.update(h, move |f: &mut Form| f.name = v))
                        .height(40.0)
                        .into(),
                    button(text("Save"))
                        .enabled(!f.name.is_empty())
                        .on_click(move |_| log(format!("saved {name}")))
                        .height(40.0)
                        .into(),
                ])
            },
        )
        .into()
    };
    let mut b = run(view, (200.0, 100.0));
    tap(&mut b, 20.0, 20.0);
    b.event(Event::Key(dfui::Key::End));
    b.event(Event::Text("a".into()));
    b.advance(SETTLED);
    tap(&mut b, 20.0, 60.0);
    assert_eq!(logged(), ["saved Anna"]);
    tap(&mut b, 20.0, 20.0);
    b.event(Event::Key(dfui::Key::SelectAll));
    b.event(Event::Text(String::new()));
    b.advance(SETTLED);
    tap(&mut b, 20.0, 60.0);
    assert!(logged().is_empty(), "an empty name doesn't save");
}

#[test]
fn confirm_blocks_the_page_and_runs_on_ok() {
    let page = |ov: Handle<Overlays>| -> Element {
        tall("Delete", move |cx| {
            log("asked");
            ov.confirm(cx, "Delete it?", "This can't be undone.", "Delete", |_| {
                log("deleted")
            });
        })
    };
    let mut b = run(move || overlays(page).into(), (280.0, 240.0));
    tap(&mut b, 20.0, 20.0);
    assert_eq!(logged(), ["asked"]);
    tap(&mut b, 20.0, 20.0);
    assert!(logged().is_empty(), "a tap outside only dismisses");
    tap(&mut b, 20.0, 20.0);
    assert_eq!(logged(), ["asked"], "the page works again");
    tap(&mut b, 190.0, 150.0);
    assert_eq!(logged(), ["deleted"]);
}

#[test]
fn back_closes_the_overlay_before_the_screen() {
    let page = |ov: Handle<Overlays>| -> Element {
        column([
            on_back(|_| log("screen back")).into(),
            tall("Open", move |cx| ov.show(cx, |_| dialog(text("Hello")))),
        ])
        .into()
    };
    let mut b = run(move || overlays(page).into(), (280.0, 240.0));
    tap(&mut b, 20.0, 20.0);
    b.event(Event::Back);
    b.settle();
    assert!(logged().is_empty(), "the dialog took it");
    b.event(Event::Back);
    b.settle();
    assert_eq!(logged(), ["screen back"]);
}

#[test]
fn typing_reaches_an_input_inside_a_dialog() {
    let page = |ov: Handle<Overlays>| -> Element {
        tall("Open", move |cx| {
            ov.show(cx, |_| {
                dialog(stateful(|v: &String, h| {
                    log(format!("value {v}"));
                    text_input(v.clone())
                        .on_change(move |cx, v| cx.update(h, move |s: &mut String| *s = v))
                        .height(40.0)
                        .width(120.0)
                }))
            })
        })
    };
    let mut b = run(move || overlays(page).into(), (280.0, 240.0));
    tap(&mut b, 20.0, 20.0);
    tap(&mut b, 140.0, 120.0);
    b.event(Event::Text("hi".into()));
    b.advance(SETTLED);
    assert_eq!(logged().last().unwrap(), "value hi");
}

#[derive(Clone)]
struct Item(u64);

impl Keyed for Item {
    fn key(&self) -> u64 {
        self.0
    }
}

#[test]
fn list_rows_keep_their_state_when_an_item_above_goes() {
    let items = Rc::new(RefCell::new((0..5).map(Item).collect::<Vec<_>>()));
    let model = items.clone();
    let mut b = run(
        move || {
            let rows = list(model.clone(), |it: &Item| -> Element {
                let k = it.0;
                stateful(move |n: &i32, h| {
                    log(format!("row {k} = {n}"));
                    tall("+", move |cx| cx.update(h, |n: &mut i32| *n += 1))
                })
                .into()
            });
            column([rows.fill().into()]).fill().into()
        },
        (200.0, 200.0),
    );
    tap(&mut b, 20.0, 60.0);
    items.borrow_mut().remove(0);
    b.frame();
    b.settle();
    let log = logged();
    let last = log.iter().rfind(|l| l.starts_with("row 1")).unwrap();
    assert_eq!(last, "row 1 = 1");
}

#[test]
fn a_tap_after_scrolling_hits_the_row_on_screen_not_a_drag() {
    let mut b = run(
        || {
            let rows = (0..20).map(|i| tall("row", move |_| log(format!("row {i}"))));
            column([scroll(column(rows)).fill().into()]).fill().into()
        },
        (200.0, 200.0),
    );
    b.event(Event::Wheel {
        x: 20.0,
        y: 20.0,
        dy: 400.0,
    });
    b.settle();
    tap(&mut b, 20.0, 20.0);
    assert_eq!(logged(), ["row 10"]);

    b.event(Event::PointerDown { x: 20.0, y: 100.0 });
    b.event(Event::PointerMove { x: 20.0, y: 60.0 });
    b.event(Event::PointerUp { x: 20.0, y: 60.0 });
    b.settle();
    assert!(logged().is_empty(), "a drag doesn't tap");
}

#[test]
fn scroll_position_survives_a_rebuild() {
    let mut b = run(
        || {
            stateful(|_: &i32, h| {
                let rows = (0..20).map(move |i| {
                    tall("row", move |cx| {
                        log(format!("row {i}"));
                        cx.update(h, |n: &mut i32| *n += 1);
                    })
                });
                column([scroll(column(rows)).fill().into()]).fill()
            })
            .fill()
            .into()
        },
        (200.0, 200.0),
    );
    b.event(Event::Wheel {
        x: 20.0,
        y: 20.0,
        dy: 400.0,
    });
    b.settle();
    tap(&mut b, 20.0, 20.0);
    tap(&mut b, 20.0, 20.0);
    assert_eq!(logged(), ["row 10", "row 10"]);
}

#[test]
fn async_data_shows_loading_then_the_result() {
    let mut b = run(
        || {
            future_builder(
                (),
                |_| async {
                    sleep(Duration::from_secs(1)).await;
                    7
                },
                |s| {
                    log(format!("{:?}", s.data));
                    text("x")
                },
            )
            .into()
        },
        (200.0, 100.0),
    );
    assert_eq!(logged().first().unwrap(), "None");
    assert_eq!(logged(), Vec::<String>::new());
    b.advance(Duration::from_millis(1100));
    b.settle();
    assert_eq!(logged().last().unwrap(), "Some(7)");
}

#[test]
fn an_outside_store_updates_through_a_notifier() {
    let store = Rc::new(RefCell::new(1));
    let n = Notifier::new();
    let (s, w) = (store.clone(), n.clone());
    let mut b = run(
        move || {
            let s = s.clone();
            watch(&w, move || {
                log(format!("store {}", s.borrow()));
                text("x")
            })
            .into()
        },
        (200.0, 100.0),
    );
    *store.borrow_mut() = 2;
    n.notify();
    b.advance(Duration::from_millis(50));
    b.settle();
    assert_eq!(logged().last().unwrap(), "store 2");
}

#[test]
fn dropdown_selection_feeds_back_into_the_view() {
    let mut b = run(
        || {
            stateful(|&i: &usize, h| {
                log(format!("picked {i}"));
                row([dropdown(["A", "B", "C"], i)
                    .on_change(move |cx, v| cx.update(h, move |s: &mut usize| *s = v))
                    .into()])
            })
            .into()
        },
        (200.0, 240.0),
    );
    tap(&mut b, 20.0, 20.0);
    logged();
    tap(&mut b, 20.0, 90.0);
    assert_eq!(logged().last().unwrap(), "picked 1");
}
