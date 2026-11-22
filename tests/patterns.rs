//! The patterns apps are built from, driven through input events like a user.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use dfui::model::Keyed;
use dfui::taffy::{Dimension, Size, Style};
use dfui::widgets::{
    Overlays, Styled, button, column, dialog, dropdown, future_builder, heading, list, on_back,
    overlays, page, row, scroll, stateful, stateful_with, text, text_input, watch,
};
use dfui::widgets::{Snapshot, stream_builder};
use dfui::{
    App, Bench, Cache, Element, Event, EventCx, Handle, Latest, Notifier, Widget, blocking, cached,
    polled, sleep,
};

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
fn polled_source_refreshes_on_its_schedule_then_stops() {
    let mut b = run(
        || {
            let mut n = 0;
            let source = polled(
                move || {
                    n += 1;
                    let v = n;
                    async move { v }
                },
                |&v: &u32| (v < 3).then_some(Duration::from_secs(10)),
            );
            stream_builder(
                (),
                |_| source,
                |s: &Snapshot<Latest<u32>>| {
                    log(format!("{:?} {}", s.data.get(), s.done));
                    text("x")
                },
            )
            .into()
        },
        (200.0, 100.0),
    );
    b.advance(Duration::from_millis(1));
    assert_eq!(logged().last().unwrap(), "Some(1) false");
    b.advance(Duration::from_secs(9));
    assert_eq!(logged(), Vec::<String>::new(), "waits between fetches");
    b.advance(Duration::from_secs(2));
    assert_eq!(logged().last().unwrap(), "Some(2) false");
    b.advance(Duration::from_secs(11));
    b.settle();
    assert_eq!(logged().last().unwrap(), "Some(3) true");
}

#[test]
fn blocking_work_resolves_on_the_ui_thread() {
    let mut b = run(
        || {
            future_builder(
                (),
                |_| blocking(|_| 6 * 7),
                |s| {
                    log(format!("{:?}", s.data));
                    text("x")
                },
            )
            .into()
        },
        (200.0, 100.0),
    );
    for _ in 0..200 {
        if logged().iter().any(|l| l == "Some(42)") {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
        b.advance(Duration::from_millis(5));
    }
    panic!("never resolved");
}

#[test]
fn cached_results_survive_their_node() {
    let fetches = Rc::new(RefCell::new(0));
    let cache: Cache<u32, String> = Cache::new();
    let shown = Rc::new(RefCell::new(true));
    let (f, c, s) = (fetches.clone(), cache.clone(), shown.clone());
    let mut b = run(
        move || {
            if !*s.borrow() {
                return column([]).into();
            }
            let f = f.clone();
            cached(
                &c,
                7,
                move |_| {
                    *f.borrow_mut() += 1;
                    async { Some("seven".to_string()) }
                },
                |v| {
                    log(format!("{v:?}"));
                    text("x")
                },
            )
        },
        (200.0, 100.0),
    );
    b.advance(Duration::from_millis(1));
    assert_eq!(logged().last().unwrap(), "Some(\"seven\")");
    *shown.borrow_mut() = false;
    b.frame();
    *shown.borrow_mut() = true;
    b.frame();
    assert_eq!(logged().last().unwrap(), "Some(\"seven\")");
    assert_eq!(*fetches.borrow(), 1, "built from the cache the second time");
}

/// Counts presses in its own state and follows the pointer it captured.
struct Presses;

impl Widget for Presses {
    fn style(&self) -> Style {
        Style {
            size: Size {
                width: Dimension::length(50.0),
                height: Dimension::length(50.0),
            },
            ..Style::default()
        }
    }

    fn event(&self, ev: &Event, cx: &mut EventCx) {
        match ev {
            Event::PointerDown { .. } => cx.with_state(|n: &mut u32, cx| {
                *n += 1;
                cx.capture();
                log(format!("down {n}"));
            }),
            Event::PointerMove { x, .. } if !cx.hit(*x, 10.0) => log("moved outside"),
            _ => {}
        }
    }
}

#[test]
fn custom_widget_keeps_state_and_captures_in_one_handler() {
    let mut b = run(|| column([Element::new(Presses)]).into(), (200.0, 100.0));
    for _ in 0..2 {
        b.event(Event::PointerDown { x: 10.0, y: 10.0 });
        b.event(Event::PointerMove { x: 150.0, y: 10.0 });
        b.event(Event::PointerUp { x: 150.0, y: 10.0 });
    }
    assert_eq!(
        logged(),
        ["down 1", "moved outside", "down 2", "moved outside"]
    );
}

#[test]
fn page_back_arrow_and_back_key_run_the_same_handler() {
    let mut b = run(
        || page(heading("Linie 3")).back(|_| log("back")).into(),
        (200.0, 100.0),
    );
    b.event(Event::Back);
    // The arrow: 16 px page padding plus half the 44 px icon button.
    tap(&mut b, 38.0, 38.0);
    assert_eq!(logged(), ["back", "back"]);
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
