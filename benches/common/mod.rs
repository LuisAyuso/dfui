// Each bench binary uses a subset of these.
#![allow(dead_code)]

use dfui::taffy::AlignItems;
use dfui::widgets::{Styled, button, column, container, row, scroll, text};
use dfui::{App, Color, Element, Shadow};

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

impl App for Cards {
    fn view(&self) -> Element {
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
        column([scroll(column(cards).gap(12.0)).grow(1.0).into()])
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
