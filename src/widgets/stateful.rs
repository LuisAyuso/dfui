use std::cell::{Cell, OnceCell};

use taffy::{FlexDirection, Style};

use super::Styled;
use crate::widget::{BuildCx, Element, Handle, Widget};

type Build<S> = Box<dyn Fn(&S, Handle<S>) -> Element>;

/// Widget with local state `S`, kept across rebuilds by tree position or key.
/// Handlers change it with `cx.update(handle, |s| ..)`, which rebuilds the subtree.
pub struct Stateful<S> {
    init: Cell<Option<Box<dyn FnOnce() -> S>>>,
    build: Build<S>,
    child: OnceCell<[Element; 1]>,
    style: Style,
}

/// Builds a subtree from local state `S`, kept across rebuilds.
pub fn stateful<S, E>(build: impl Fn(&S, Handle<S>) -> E + 'static) -> Stateful<S>
where
    S: Default + 'static,
    E: Into<Element>,
{
    stateful_with(S::default, build)
}

/// [`stateful`] starting from `init`, e.g. a form over existing data. `init`
/// runs once; give the element a new `key` to start over from other data.
pub fn stateful_with<S, E>(
    init: impl FnOnce() -> S + 'static,
    build: impl Fn(&S, Handle<S>) -> E + 'static,
) -> Stateful<S>
where
    S: 'static,
    E: Into<Element>,
{
    Stateful {
        init: Cell::new(Some(Box::new(init))),
        build: Box::new(move |s, h| build(s, h).into()),
        child: OnceCell::new(),
        style: Style {
            flex_direction: FlexDirection::Column,
            ..Style::default()
        },
    }
}

impl<S: 'static> From<Stateful<S>> for Element {
    fn from(w: Stateful<S>) -> Self {
        Element::new(w)
    }
}

impl<S> Styled for Stateful<S> {
    fn style_mut(&mut self) -> &mut Style {
        &mut self.style
    }
}

impl<S: 'static> Widget for Stateful<S> {
    fn style(&self) -> Style {
        self.style.clone()
    }
    fn expand(&self, cx: &mut BuildCx) {
        self.child.get_or_init(|| {
            let init = || self.init.take().expect("init is taken once")();
            [(self.build)(
                cx.state.get_or_insert_with(init),
                Handle::new(cx.id),
            )]
        });
    }
    fn children(&self) -> &[Element] {
        self.child.get().map_or(&[], |c| c.as_slice())
    }
}
