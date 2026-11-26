// Each bench binary uses a subset of these.
#![allow(dead_code)]

use std::rc::Rc;

use dfui::model::Keyed;
use dfui::taffy::AlignItems;
use dfui::widgets::{
    Column, Scroll, Styled, button, column, container, list, row, scroll, spacer, stateful, table, text,
    text_input,
};
use dfui::{App, Color, Element, Role, Shadow};

/// A scrolling list of demo-style cards: the common shape of a real screen.
/// Each update bumps a count shown on the first card.
pub struct Cards {
    n: u32,
    edits: u32,
}

impl Cards {
    pub fn new(n: u32) -> Self {
        Cards { n, edits: 0 }
    }

    pub fn edit(&mut self) {
        self.edits += 1;
    }
}

impl Cards {
    fn scroll(&self) -> Scroll {
        let cards = (1..=self.n).map(|n| {
            let card: Element = row([
                container(text(n.to_string()).size(20.0).color(Color::WHITE))
                    .bg(Color::hex(0x3b6fe0))
                    .radius(24.0)
                    .width(48.0)
                    .height(48.0)
                    .into(),
                column([
                    text(if n == 1 {
                        format!("Card {n}, edited {}", self.edits)
                    } else {
                        format!("Card {n}")
                    })
                    .size(18.0)
                    .into(),
                    text("Rounded box SDF with border and soft shadow.")
                        .size(14.0)
                        .into(),
                ])
                .grow(1.0)
                .into(),
                button(text("×")).on_click(|_| {}).into(),
            ])
            .align(AlignItems::CENTER)
            .gap(14.0)
            .padding(14.0)
            .bg(Color::WHITE)
            .radius(14.0)
            .shadow(Shadow {
                color: Color::BLACK.alpha(0.12),
                offset: (0.0, 3.0),
                blur: 8.0,
            })
            .into();
            card.key(n as u64)
        });
        scroll(column(cards).gap(12.0))
    }
}

impl App for Cards {
    fn view(&self) -> Element {
        column([self.scroll().grow(1.0).into()])
            .padding(20.0)
            .into()
    }
}

/// The demo's Bench screen: a page of large glyphs, the worst case for text.
pub struct TextWall;

impl App for TextWall {
    fn view(&self) -> Element {
        column((0..14).map(|_| text("Rag&@ %g1 WM").size(64.0).into()))
            .padding(16.0)
            .into()
    }
}

#[derive(Clone)]
pub struct Item {
    id: u64,
    name: String,
    score: u32,
}

impl Keyed for Item {
    fn key(&self) -> u64 {
        self.id
    }
}

fn items(n: u32) -> Rc<Vec<Item>> {
    Rc::new(
        (0..n as u64)
            .map(|id| Item {
                id,
                name: format!("Item {id}"),
                score: (id.wrapping_mul(2654435761) % 1000) as u32,
            })
            .collect(),
    )
}

/// A virtualized `list` of `n` rows shaped like the list example's.
pub struct Rows(Rc<Vec<Item>>);

impl Rows {
    pub fn new(n: u32) -> Self {
        Rows(items(n))
    }
}

impl App for Rows {
    fn view(&self) -> Element {
        let rows = list(self.0.clone(), |it: &Item| {
            row([
                text(it.name.clone()).into(),
                spacer().grow(1.0).into(),
                button(text("Remove").size(13.0))
                    .flat()
                    .on_click(|_| {})
                    .into(),
            ])
            .align(AlignItems::CENTER)
            .padding_xy(12.0, 6.0)
            .bg(Role::Surface)
        });
        column([rows.fill().into()]).fill().padding(16.0).into()
    }
}

/// A `table` of `n` rows with two sortable columns; a tap on the header at
/// [`Table::HEADER`] sorts by name.
pub struct Table(Rc<Vec<Item>>);

impl Table {
    pub const HEADER: (f32, f32) = (40.0, 26.0);

    pub fn new(n: u32) -> Self {
        Table(items(n))
    }
}

impl App for Table {
    fn view(&self) -> Element {
        let columns = vec![
            Column::new("Name", |it: &Item| text(it.name.clone()))
                .sort_by(|a, b| a.name.cmp(&b.name)),
            Column::new("Score", |it: &Item| text(it.score.to_string()))
                .width(96.0)
                .sort_by(|a, b| a.score.cmp(&b.score)),
        ];
        column([table(self.0.clone(), columns).fill().into()])
            .fill()
            .padding(16.0)
            .into()
    }
}

/// A text field at [`Typing::FIELD`] above `n` cards, echoing what's typed.
pub struct Typing(Cards);

impl Typing {
    pub const FIELD: (f32, f32) = (40.0, 36.0);

    pub fn new(n: u32) -> Self {
        Typing(Cards::new(n))
    }
}

impl App for Typing {
    fn view(&self) -> Element {
        let form = stateful(|name: &String, h| {
            column([
                text_input(name.clone())
                    .placeholder("Your name")
                    .on_change(move |cx, v| cx.update(h, |n: &mut String| *n = v))
                    .into(),
                text(format!("Hello, {name}!")).into(),
            ])
            .gap(12.0)
        });
        column([form.into(), self.0.scroll().grow(1.0).into()])
            .gap(12.0)
            .fill()
            .padding(16.0)
            .into()
    }
}
