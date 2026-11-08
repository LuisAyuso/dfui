//! # Table
//!
//! `table(model, columns)` shows a model's items as rows, on top of a
//! virtualized `list`. `Column::new(title, cell)` describes one column;
//! `width` fixes it, otherwise columns share the space left. With `sort_by`,
//! tapping the header sorts by it, then reversed, then back to the model's order.

use dfui::Element;
use dfui::model::Keyed;
use dfui::widgets::{Column, Styled, column, table, text};

#[derive(Clone)]
struct Player {
    id: u64,
    name: &'static str,
    goals: u32,
}

impl Keyed for Player {
    fn key(&self) -> u64 {
        self.id
    }
}

fn players() -> Vec<Player> {
    [
        ("Griff", 12),
        ("Morg", 7),
        ("Varag", 21),
        ("Hakflem", 3),
        ("Skrull", 9),
    ]
    .into_iter()
    .enumerate()
    .map(|(i, (name, goals))| Player {
        id: i as u64,
        name,
        goals,
    })
    .collect()
}

fn view() -> Element {
    let columns = vec![
        Column::new("Name", |p: &Player| text(p.name)).sort_by(|a, b| a.name.cmp(b.name)),
        Column::new("Goals", |p: &Player| text(p.goals.to_string()))
            .width(96.0)
            .sort_by(|a, b| a.goals.cmp(&b.goals)),
    ];
    column([table(players(), columns).fill().into()])
        .fill()
        .padding(16.0)
        .into()
}

fn main() {
    dfui_site::run(view);
}
