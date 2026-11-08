//! # List
//!
//! `list(model, row)` builds only the rows near the viewport, so thousands of
//! items cost the same as a screenful. Rows are matched by key: the first
//! visible one stays in place when items change above it, and inserts, removals
//! and moves slide.
//!
//! A `Vec` of `Keyed` items is a model, and so is a shared `Rc<RefCell<..>>`.
//! Like `scroll`, a list needs a bounded height: give every flex box between it
//! and the window `.fill()`.

use std::cell::RefCell;
use std::rc::Rc;

use dfui::model::Keyed;
use dfui::taffy::AlignItems;
use dfui::widgets::{Styled, button, column, list, row, spacer, stateful, text};
use dfui::{Element, Role};

#[derive(Clone)]
struct Song(u64);

impl Keyed for Song {
    fn key(&self) -> u64 {
        self.0
    }
}

type Songs = Rc<RefCell<Vec<Song>>>;

thread_local! {
    static SONGS: Songs = Rc::new(RefCell::new((0..1000).map(Song).collect()));
}

fn view() -> Element {
    let songs = SONGS.with(Rc::clone);
    // The state only asks for a rebuild; the model holds the data.
    stateful(move |_: &u32, h| {
        let model = songs.clone();
        let rows = list(songs.clone(), move |s: &Song| {
            let (key, model) = (s.0, model.clone());
            row([
                text(format!("Song {key}")).into(),
                spacer().grow(1.0).into(),
                button(text("Remove").size(13.0))
                    .flat()
                    .on_click(move |cx| {
                        model.borrow_mut().retain(|s| s.0 != key);
                        cx.update(h, |n: &mut u32| *n += 1);
                    })
                    .into(),
            ])
            .align(AlignItems::CENTER)
            .padding_xy(12.0, 6.0)
            .bg(Role::Surface)
        });
        column([rows.fill().into()]).fill().padding(16.0)
    })
    .fill()
    .into()
}

fn main() {
    dfui_site::run(view);
}
