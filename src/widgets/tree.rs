use std::collections::HashSet;
use std::rc::Rc;

use taffy::{AlignItems, LengthPercentage};

use super::list::{List, Rows, Thunk};
use super::{Stateful, Styled, button, container, row, spacer, stateful, text};
use crate::model::{Row, TreeModel, flatten};
use crate::render::Color;
use crate::theme::Role;
use crate::widget::{Element, Handle};

const INDENT: f32 = 20.0;
const TOGGLE: f32 = 36.0;

/// Keys of the expanded nodes; view state, kept apart from the model.
#[derive(Default)]
pub struct Expanded(HashSet<u64>);

type Build<T> = Rc<dyn Fn(&T, &Row) -> Element>;

struct TreeRows<M: TreeModel> {
    model: Rc<M>,
    rows: Vec<Row>,
    build: Build<M::Item>,
    open: Handle<Expanded>,
}

impl<M> Rows for TreeRows<M>
where
    M: TreeModel + 'static,
    M::Item: Clone + 'static,
{
    fn len(&self) -> usize {
        self.rows.len()
    }
    fn key(&self, i: usize) -> u64 {
        self.rows[i].key
    }
    fn row(&self, i: usize) -> Thunk {
        let r = self.rows[i];
        let item = self.model.with(r.key, Clone::clone);
        let (build, h) = (self.build.clone(), self.open);
        Rc::new(move || {
            let key = r.key;
            let toggle: Element = if r.has_children {
                button(
                    text(if r.expanded { "▾" } else { "▸" })
                        .size(16.0)
                        .color(Role::Ink),
                )
                .flat()
                .color(Color::TRANSPARENT)
                .padding(0.0)
                .width(TOGGLE)
                .on_click(move |cx| {
                    cx.update(h, move |e: &mut Expanded| {
                        if !e.0.remove(&key) {
                            e.0.insert(key);
                        }
                    })
                })
                .into()
            } else {
                spacer().width(TOGGLE).into()
            };
            let body = container(build(&item, &r)).fill();
            let mut line = row([toggle, body.into()]).align(AlignItems::CENTER);
            line.style_mut().padding.left = LengthPercentage::length(r.depth as f32 * INDENT);
            line.into()
        })
    }
}

/// `model`'s nodes as an indented, virtualized list; the arrow before a node
/// with children expands or collapses it. `row` draws the rest of each line.
#[doc = example!("tree_view")]
pub fn tree_view<M, F, E>(model: M, row: F) -> Stateful<Expanded>
where
    M: TreeModel + 'static,
    M::Item: Clone + 'static,
    F: Fn(&M::Item, &Row) -> E + 'static,
    E: Into<Element>,
{
    let model = Rc::new(model);
    let build: Build<M::Item> = Rc::new(move |it, r| row(it, r).into());
    stateful(move |open: &Expanded, h| {
        List::new(TreeRows {
            rows: flatten(&*model, &open.0),
            model: model.clone(),
            build: build.clone(),
            open: h,
        })
        .fill()
    })
}
