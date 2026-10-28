use std::cell::OnceCell;

use taffy::{FlexDirection, Style};

use super::Styled;
use crate::widget::{Element, Handle, State, Widget};

type Build<S, Msg> = Box<dyn Fn(&S, Handle<S>) -> Element<Msg>>;

/// Widget with local state `S`, kept across rebuilds by tree position or key.
/// Handlers change it with `cx.update(handle, |s| ..)`, which rebuilds the subtree.
pub struct Stateful<S, Msg> {
    build: Build<S, Msg>,
    child: OnceCell<[Element<Msg>; 1]>,
    style: Style,
}

pub fn stateful<S, Msg, E>(build: impl Fn(&S, Handle<S>) -> E + 'static) -> Stateful<S, Msg>
where
    S: Default + 'static,
    E: Into<Element<Msg>>,
{
    Stateful {
        build: Box::new(move |s, h| build(s, h).into()),
        child: OnceCell::new(),
        style: Style {
            flex_direction: FlexDirection::Column,
            ..Style::default()
        },
    }
}

impl<S: Default + 'static, Msg: 'static> From<Stateful<S, Msg>> for Element<Msg> {
    fn from(w: Stateful<S, Msg>) -> Self {
        Element::new(w)
    }
}

impl<S, Msg> Styled for Stateful<S, Msg> {
    fn style_mut(&mut self) -> &mut Style {
        &mut self.style
    }
}

impl<S: Default + 'static, Msg> Widget<Msg> for Stateful<S, Msg> {
    fn style(&self) -> Style {
        self.style.clone()
    }
    fn expand(&self, id: u64, state: &mut State) {
        self.child
            .get_or_init(|| [(self.build)(state.get::<S>(), Handle::new(id))]);
    }
    fn children(&self) -> &[Element<Msg>] {
        self.child.get().map_or(&[], |c| c.as_slice())
    }
}
