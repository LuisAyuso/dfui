use std::rc::Rc;

use taffy::AlignItems;

use super::{Label, Styled, column, icon_button, on_back, row, text};
use crate::icons;
use crate::text::Font;
use crate::widget::{Element, EventCx};

type Action = Rc<dyn Fn(&mut EventCx)>;

enum Lead {
    Back(Action),
    Menu(Action),
}

/// A screen: a header row (a back or menu button, then `title`) above the
/// body, padded and filling its parent. `title` is any widget: a
/// [`heading`], or tabs.
#[doc = example!("page")]
pub fn page(title: impl Into<Element>) -> Page {
    Page {
        title: title.into(),
        lead: None,
        body: Vec::new(),
    }
}

/// Bold text sized for a [`page`] title.
pub fn heading(s: impl Into<String>) -> Label {
    text(s).font(Font::BOLD).size(22.0)
}

/// A screen built by [`page`].
pub struct Page {
    title: Element,
    lead: Option<Lead>,
    body: Vec<Element>,
}

impl Page {
    /// An arrow before the title that runs `f`, which Escape and the
    /// Android back button run too, as [`on_back`] does.
    pub fn back(mut self, f: impl Fn(&mut EventCx) + 'static) -> Self {
        self.lead = Some(Lead::Back(Rc::new(f)));
        self
    }

    /// A menu button before the title that runs `f`, e.g. showing a `drawer`.
    pub fn menu(mut self, f: impl Fn(&mut EventCx) + 'static) -> Self {
        self.lead = Some(Lead::Menu(Rc::new(f)));
        self
    }

    /// Adds `e` to the body, below the header and the children before it.
    pub fn child(mut self, e: impl Into<Element>) -> Self {
        self.body.push(e.into());
        self
    }

    /// Adds each of `items` to the body.
    pub fn children<E: Into<Element>>(mut self, items: impl IntoIterator<Item = E>) -> Self {
        self.body.extend(items.into_iter().map(Into::into));
        self
    }
}

impl From<Page> for Element {
    fn from(p: Page) -> Element {
        let mut items: Vec<Element> = Vec::new();
        let mut head: Vec<Element> = Vec::new();
        let lead = match p.lead {
            Some(Lead::Back(f)) => {
                let g = f.clone();
                items.push(on_back(move |cx| g(cx)).into());
                Some((icons::back(), f))
            }
            Some(Lead::Menu(f)) => Some((icons::menu(), f)),
            None => None,
        };
        if let Some((shape, f)) = lead {
            head.push(icon_button(&shape).on_click(move |cx| f(cx)).into());
        }
        head.push(p.title);
        items.push(row(head).align(AlignItems::CENTER).gap(4.0).into());
        items.extend(p.body);
        column(items).gap(12.0).padding(16.0).fill().into()
    }
}
