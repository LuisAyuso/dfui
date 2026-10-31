use std::any::Any;

use taffy::{FlexDirection, Style};

use super::Styled;
use crate::widget::{Element, Widget};

/// Makes `T` available to every descendant through `Scope::get`, e.g. a
/// handle to external state or a service a builder starts work from.
pub struct Provide<T> {
    value: T,
    child: [Element; 1],
    style: Style,
}

pub fn provide<T: 'static>(value: T, child: impl Into<Element>) -> Provide<T> {
    Provide {
        value,
        child: [child.into()],
        style: Style {
            flex_direction: FlexDirection::Column,
            ..Style::default()
        },
    }
}

impl<T: 'static> From<Provide<T>> for Element {
    fn from(w: Provide<T>) -> Self {
        Element::new(w)
    }
}

impl<T> Styled for Provide<T> {
    fn style_mut(&mut self) -> &mut Style {
        &mut self.style
    }
}

impl<T: 'static> Widget for Provide<T> {
    fn style(&self) -> Style {
        self.style.clone()
    }
    fn children(&self) -> &[Element] {
        &self.child
    }
    fn provide(&self) -> Option<&dyn Any> {
        Some(&self.value)
    }
}
