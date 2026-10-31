//! Starter widget set. Composite widgets should be plain functions over these.
//!
//! A blanket `impl<W: Widget> From<W> for Element` is rejected by coherence, so
//! each widget opts in next to its definition. Third-party widgets can do the
//! same or use `Element::new`.

mod builder;
mod button;
mod container;
mod label;
mod provide;
mod radio;
mod scroll;
mod segmented;
mod slider;
mod stateful;
mod text_input;
mod toggle;

use taffy::{AlignItems, Dimension, FlexWrap, JustifyContent, LengthPercentage, Size, Style};

pub use builder::{Builder, Snapshot, future_builder, stream_builder};
pub use button::{Button, button};
pub use container::{Container, column, container, row, spacer};
pub use label::{Label, text};
pub use provide::{Provide, provide};
pub use radio::{Radio, radio};
pub use scroll::{Scroll, scroll};
pub use segmented::{Segmented, segmented, tabs};
pub use slider::{Slider, slider};
pub use stateful::{Stateful, stateful};
pub use text_input::{TextInput, text_input};
pub use toggle::{Toggle, toggle};

/// Flex-layout modifiers shared by every widget that owns a `Style`.
pub trait Styled: Sized {
    fn style_mut(&mut self) -> &mut Style;

    fn padding(mut self, p: f32) -> Self {
        self.style_mut().padding = taffy::Rect::length(p);
        self
    }
    fn padding_xy(mut self, x: f32, y: f32) -> Self {
        let (x, y) = (LengthPercentage::length(x), LengthPercentage::length(y));
        self.style_mut().padding = taffy::Rect {
            left: x,
            right: x,
            top: y,
            bottom: y,
        };
        self
    }
    fn gap(mut self, g: f32) -> Self {
        self.style_mut().gap = Size::length(g);
        self
    }
    fn grow(mut self, g: f32) -> Self {
        self.style_mut().flex_grow = g;
        self
    }
    fn shrink(mut self, s: f32) -> Self {
        self.style_mut().flex_shrink = s;
        self
    }
    fn width(mut self, w: f32) -> Self {
        self.style_mut().size.width = Dimension::length(w);
        self
    }
    fn height(mut self, h: f32) -> Self {
        self.style_mut().size.height = Dimension::length(h);
        self
    }
    fn fill_width(mut self) -> Self {
        self.style_mut().size.width = Dimension::percent(1.0);
        self
    }
    fn align(mut self, a: AlignItems) -> Self {
        self.style_mut().align_items = a;
        self
    }
    fn justify(mut self, j: JustifyContent) -> Self {
        self.style_mut().justify_content = j;
        self
    }
    fn wrap(mut self) -> Self {
        self.style_mut().flex_wrap = FlexWrap::Wrap;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Probe(Style);

    impl Styled for Probe {
        fn style_mut(&mut self) -> &mut Style {
            &mut self.0
        }
    }

    #[test]
    fn modifiers_write_style() {
        let s = Probe::default()
            .padding_xy(4.0, 2.0)
            .gap(3.0)
            .grow(1.0)
            .width(10.0)
            .wrap()
            .0;
        let (x, y) = (LengthPercentage::length(4.0), LengthPercentage::length(2.0));
        assert_eq!(
            s.padding,
            taffy::Rect {
                left: x,
                right: x,
                top: y,
                bottom: y
            }
        );
        assert_eq!(s.gap, Size::length(3.0));
        assert_eq!(s.flex_grow, 1.0);
        assert_eq!(s.size.width, Dimension::length(10.0));
        assert_eq!(s.flex_wrap, FlexWrap::Wrap);
    }
}
