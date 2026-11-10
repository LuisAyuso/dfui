//! Starter widget set. Composite widgets should be plain functions over these.
//!
//! A blanket `impl<W: Widget> From<W> for Element` is rejected by coherence, so
//! each widget opts in next to its definition. Third-party widgets can do the
//! same or use `Element::new`.

/// A site example (`site/examples/<name>.rs`) as a doc section: the same
/// code the site shows and runs, compiled as a doc test but not run, since
/// it opens a window.
macro_rules! example {
    ($name:literal) => {
        concat!(
            "\n# Example\n\n```no_run\n",
            include_str!(concat!("../../site/examples/", $name, ".rs")),
            "```"
        )
    };
}

mod builder;
mod button;
mod chip;
mod container;
mod icon;
mod image;
mod label;
mod list;
mod overlay;
mod progress;
mod provide;
mod radio;
mod scroll;
mod segmented;
mod slider;
mod stateful;
mod table;
mod text_input;
mod toggle;
mod tree;

use taffy::{
    AlignItems, Dimension, FlexWrap, JustifyContent, LengthPercentage, LengthPercentageAuto, Size,
    Style,
};

pub use builder::{Builder, Seen, Snapshot, future_builder, stream_builder, watch};
pub use button::{Button, button};
pub use chip::{badge, chip, choice_chip};
pub use container::{Container, column, container, row, spacer};
pub use icon::{Icon, icon, icon_button};
pub use image::{Image, image};
pub use label::{Label, text};
pub use list::{List, list};
pub use overlay::{
    Dropdown, OnBack, Overlay, Overlays, Shown, bottom_sheet, dialog, drawer, dropdown, on_back,
    overlay, overlays, popup, snackbar, tooltip,
};
pub use progress::{Progress, Spinner, progress, spinner, stacked_progress, wait_bar};
pub use provide::{Provide, provide};
pub use radio::{Radio, radio};
pub use scroll::{Scroll, scroll};
pub use segmented::{Segmented, segmented, tabs};
pub use slider::{Slider, slider};
pub use stateful::{Stateful, stateful, stateful_with};
pub use table::{Column, TableSort, table};
pub use text_input::{TextInput, text_input};
pub use toggle::{Toggle, toggle};
pub use tree::{Expanded, tree_view};

/// Flex-layout modifiers shared by every widget that owns a [`Style`].
pub trait Styled: Sized {
    /// The taffy style the modifiers write; also an escape hatch for the rest.
    fn style_mut(&mut self) -> &mut Style;

    /// Space between the box's edges and its children on all sides, logical px.
    fn padding(mut self, p: f32) -> Self {
        self.style_mut().padding = taffy::Rect::length(p);
        self
    }
    /// Padding of `x` left and right and `y` top and bottom, logical px.
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
    /// Space between children, along both axes, logical px.
    fn gap(mut self, g: f32) -> Self {
        self.style_mut().gap = Size::length(g);
        self
    }
    /// Share of the parent's spare main-axis space to take; 0 by default.
    /// The content still sets the minimum, so see `fill` around a [`scroll()`].
    fn grow(mut self, g: f32) -> Self {
        self.style_mut().flex_grow = g;
        self
    }
    /// How much to give up when the parent is too small; 1 by default, 0 keeps the size.
    fn shrink(mut self, s: f32) -> Self {
        self.style_mut().flex_shrink = s;
        self
    }
    /// Fixed width, logical px.
    fn width(mut self, w: f32) -> Self {
        self.style_mut().size.width = Dimension::length(w);
        self
    }
    /// Fixed height, logical px.
    fn height(mut self, h: f32) -> Self {
        self.style_mut().size.height = Dimension::length(h);
        self
    }
    /// Lower bound on the width, logical px; `min_width(0.0)` lets a flex child shrink below its content.
    fn min_width(mut self, w: f32) -> Self {
        self.style_mut().min_size.width = LengthPercentageAuto::length(w);
        self
    }
    /// Lower bound on the height, logical px; `min_height(0.0)` lets a flex child shrink below its content.
    fn min_height(mut self, h: f32) -> Self {
        self.style_mut().min_size.height = LengthPercentageAuto::length(h);
        self
    }
    /// Upper bound on the height, logical px, e.g. a [`scroll()`] that grows with its content up to a point.
    fn max_height(mut self, h: f32) -> Self {
        self.style_mut().max_size.height = LengthPercentageAuto::length(h);
        self
    }
    /// Takes the space left along the parent's main axis, and no more: unlike
    /// `grow(1.0)`, the content's size doesn't push the parent. Use it on
    /// every flex box between the window and a [`scroll()`]. In a row, siblings
    /// that all `fill` get equal widths, whatever their content.
    fn fill(mut self) -> Self {
        let s = self.style_mut();
        s.flex_grow = 1.0;
        s.flex_basis = Dimension::length(0.0);
        s.min_size = Size::length(0.0);
        self
    }
    /// As wide as the parent's content box, whatever the flex direction.
    fn fill_width(mut self) -> Self {
        self.style_mut().size.width = Dimension::percent(1.0);
        self
    }
    /// Places children on the cross axis: across a row, along a column.
    fn align(mut self, a: AlignItems) -> Self {
        self.style_mut().align_items = a;
        self
    }
    /// Distributes children and spare space along the main axis.
    fn justify(mut self, j: JustifyContent) -> Self {
        self.style_mut().justify_content = j;
        self
    }
    /// Children that don't fit start a new line instead of shrinking.
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
