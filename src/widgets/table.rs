use std::cmp::Ordering;
use std::rc::Rc;

use taffy::{AlignItems, JustifyContent};

use super::list::{List, Rows, Thunk};
use super::{Stateful, Styled, button, column, container, row, spacer, stateful, text};
use crate::model::ListModel;
use crate::theme::Role;
use crate::widget::Element;

type Cell<T> = Rc<dyn Fn(&T) -> Element>;
type Cmp<T> = Rc<dyn Fn(&T, &T) -> Ordering>;

/// One table column: a header, a cell builder, and optionally a sort order.
pub struct Column<T> {
    title: String,
    width: Option<f32>,
    cell: Cell<T>,
    cmp: Option<Cmp<T>>,
}

impl<T> Column<T> {
    /// Shares the width left by fixed-width columns with the other flexible ones.
    pub fn new<E: Into<Element>>(
        title: impl Into<String>,
        cell: impl Fn(&T) -> E + 'static,
    ) -> Self {
        Column {
            title: title.into(),
            width: None,
            cell: Rc::new(move |t| cell(t).into()),
            cmp: None,
        }
    }
    /// Fixed width, logical px, instead of a share of what's left.
    pub fn width(mut self, w: f32) -> Self {
        self.width = Some(w);
        self
    }
    /// Tapping the header sorts by `cmp`, then reversed, then unsorted.
    pub fn sort_by(mut self, cmp: impl Fn(&T, &T) -> Ordering + 'static) -> Self {
        self.cmp = Some(Rc::new(cmp));
        self
    }

    fn sized(&self, e: impl Into<Element>) -> Element {
        let c = container(e).padding_xy(12.0, 10.0);
        match self.width {
            Some(w) => c.width(w).shrink(0.0).into(),
            None => c.fill().into(),
        }
    }
}

/// Column and direction a table is sorted by.
#[derive(Default)]
pub struct TableSort(Option<(usize, bool)>);

struct Sorted<M: ListModel> {
    model: Rc<M>,
    order: Vec<usize>,
    columns: Rc<Vec<Column<M::Item>>>,
}

impl<M> Rows for Sorted<M>
where
    M: ListModel + 'static,
    M::Item: Clone + 'static,
{
    fn len(&self) -> usize {
        self.order.len()
    }
    fn key(&self, i: usize) -> u64 {
        self.model.key(self.order[i])
    }
    fn row(&self, i: usize) -> Thunk {
        let item = self.model.with(self.order[i], Clone::clone);
        let columns = self.columns.clone();
        Rc::new(move || {
            let cells = columns.iter().map(|c| c.sized((c.cell)(&item)));
            column([
                row(cells)
                    .align(AlignItems::CENTER)
                    .bg(Role::Surface)
                    .into(),
                spacer().height(1.0).bg(Role::Divider).into(),
            ])
            .into()
        })
    }
}

/// `model`'s items as rows of `columns`, under a header that sorts by the
/// sortable ones. Rows are a virtualized `list`; bound its height the same way.
pub fn table<M>(model: M, columns: Vec<Column<M::Item>>) -> Stateful<TableSort>
where
    M: ListModel + 'static,
    M::Item: Clone + 'static,
{
    let (model, columns) = (Rc::new(model), Rc::new(columns));
    stateful(move |sort: &TableSort, h| {
        let mut order: Vec<usize> = (0..model.len()).collect();
        if let Some((c, desc)) = sort.0
            && let Some(cmp) = &columns[c].cmp
        {
            order.sort_by(|&a, &b| {
                let o = model.with(a, |a| model.with(b, |b| cmp(a, b)));
                if desc { o.reverse() } else { o }
            });
        }
        let header = columns.iter().enumerate().map(|(i, c)| {
            let arrow = match sort.0 {
                Some((s, false)) if s == i => " ▲",
                Some((s, true)) if s == i => " ▼",
                _ => "",
            };
            let label = text(format!("{}{arrow}", c.title))
                .size(14.0)
                .color(Role::Ink);
            if c.cmp.is_none() {
                return c.sized(label);
            }
            let next = move |s: &mut TableSort| {
                s.0 = match s.0 {
                    Some((c, false)) if c == i => Some((i, true)),
                    Some((c, true)) if c == i => None,
                    _ => Some((i, false)),
                }
            };
            let tap = button(label)
                .flat()
                .radius(0.0)
                .color(Role::Track)
                .on_click(move |cx| cx.update(h, next))
                .padding(0.0)
                .justify(JustifyContent::START);
            c.sized(tap)
        });
        let rows = Sorted {
            model: model.clone(),
            order,
            columns: columns.clone(),
        };
        column([
            row(header).bg(Role::Track).into(),
            List::new(rows).fill().into(),
        ])
        .fill()
    })
}
