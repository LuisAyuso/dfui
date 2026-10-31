use std::cell::{Cell, OnceCell};
use std::future::Future;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::task::{Context, Poll};

use futures_core::Stream;
use taffy::{FlexDirection, Style};

use super::Styled;
use crate::task::Task;
use crate::widget::{BuildCx, Element, Scope, State, Widget};

/// What an async source has produced so far, and whether it has finished.
#[derive(Default)]
pub struct Snapshot<T> {
    pub data: T,
    pub done: bool,
}

type Source<S> = Box<dyn FnMut(&mut Context, &mut S) -> (bool, bool)>;
type Start<S> = Box<dyn FnOnce(Scope) -> Source<S>>;

/// Subtree built from the state an async source writes. The source starts on
/// first build, restarts with fresh state when `deps` change, and is dropped
/// (cancelled) with the widget.
pub struct Builder<S> {
    deps: u64,
    start: Cell<Option<Start<S>>>,
    build: Box<dyn Fn(&S) -> Element>,
    child: OnceCell<[Element; 1]>,
    style: Style,
}

/// Rebuilds from a stream's items, collected into `S` (a `Vec`, `String`, ...).
/// `stream` gets the providers in scope.
pub fn stream_builder<St, S, E>(
    deps: impl Hash,
    stream: impl FnOnce(Scope) -> St + 'static,
    build: impl Fn(&Snapshot<S>) -> E + 'static,
) -> Builder<Snapshot<S>>
where
    St: Stream + 'static,
    S: Default + Extend<St::Item> + 'static,
    E: Into<Element>,
{
    builder(deps, build, move |scope| {
        let mut st = Box::pin(stream(scope));
        Box::new(move |cx: &mut Context, snap: &mut Snapshot<S>| {
            let mut changed = false;
            // Drains everything ready, so a burst costs one rebuild.
            loop {
                match st.as_mut().poll_next(cx) {
                    Poll::Ready(Some(item)) => {
                        snap.data.extend([item]);
                        changed = true;
                    }
                    Poll::Ready(None) => {
                        snap.done = true;
                        return (true, true);
                    }
                    Poll::Pending => return (changed, false),
                }
            }
        })
    })
}

/// Rebuilds once `future` resolves. `future` gets the providers in scope.
pub fn future_builder<F, E>(
    deps: impl Hash,
    future: impl FnOnce(Scope) -> F + 'static,
    build: impl Fn(&Snapshot<Option<F::Output>>) -> E + 'static,
) -> Builder<Snapshot<Option<F::Output>>>
where
    F: Future + 'static,
    E: Into<Element>,
{
    builder(deps, build, move |scope| {
        let mut f = Box::pin(future(scope));
        Box::new(
            move |cx: &mut Context, snap: &mut Snapshot<Option<F::Output>>| match f
                .as_mut()
                .poll(cx)
            {
                Poll::Ready(v) => {
                    snap.data = Some(v);
                    snap.done = true;
                    (true, true)
                }
                Poll::Pending => (false, false),
            },
        )
    })
}

fn builder<S: Default + 'static, E: Into<Element>>(
    deps: impl Hash,
    build: impl Fn(&S) -> E + 'static,
    start: impl FnOnce(Scope) -> Source<S> + 'static,
) -> Builder<S> {
    let mut h = DefaultHasher::new();
    deps.hash(&mut h);
    Builder {
        deps: h.finish(),
        start: Cell::new(Some(Box::new(start))),
        build: Box::new(move |s| build(s).into()),
        child: OnceCell::new(),
        style: Style {
            flex_direction: FlexDirection::Column,
            ..Style::default()
        },
    }
}

impl<S: Default + 'static> From<Builder<S>> for Element {
    fn from(w: Builder<S>) -> Self {
        Element::new(w)
    }
}

impl<S> Styled for Builder<S> {
    fn style_mut(&mut self) -> &mut Style {
        &mut self.style
    }
}

impl<S: Default + 'static> Widget for Builder<S> {
    fn style(&self) -> Style {
        self.style.clone()
    }
    fn expand(&self, cx: &mut BuildCx) {
        let stale = cx.tasks.get(&cx.id).is_none_or(|t| t.deps != self.deps);
        if stale && let Some(start) = self.start.take() {
            *cx.state.get::<S>() = S::default();
            let mut source = start(cx.scope);
            let poll = move |c: &mut Context, st: &mut State| source(c, st.get::<S>());
            cx.tasks.insert(
                cx.id,
                Task {
                    deps: self.deps,
                    poll: Some(Box::new(poll)),
                },
            );
        }
        self.child
            .get_or_init(|| [(self.build)(cx.state.get::<S>())]);
    }
    fn children(&self) -> &[Element] {
        self.child.get().map_or(&[], |c| c.as_slice())
    }
}
