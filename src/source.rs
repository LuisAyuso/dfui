//! Async sources for builders: a fetch repeated on a schedule ([`polled`]),
//! blocking work moved to a thread ([`blocking`]), a stream collector that
//! keeps only the newest item ([`Latest`]) and results kept by key across
//! rebuilds ([`Cache`], [`cached`]).

use std::cell::RefCell;
use std::collections::HashMap;
use std::future::Future;
use std::hash::Hash;
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll};
use std::time::Duration;

use futures_core::Stream;

use crate::runtime::task::{Sleep, sleep};
use crate::widget::{Element, Scope};
use crate::widgets::{Snapshot, future_builder};

type Fetch<T> = Box<dyn FnMut() -> Pin<Box<dyn Future<Output = T>>>>;
type Again<T> = Box<dyn FnMut(&T) -> Option<Duration>>;

/// A stream from [`polled`].
pub struct Polled<T> {
    fetch: Fetch<T>,
    again: Again<T>,
    state: Step<T>,
}

enum Step<T> {
    Start,
    Fetch(Pin<Box<dyn Future<Output = T>>>),
    Wait(Sleep),
    Done,
}

/// Yields what `fetch` resolves to, then fetches again after the delay
/// `again` picks for that result (a refresh after a success, a shorter retry
/// after an error) until it returns `None`. For `stream_builder`, usually
/// with [`Latest`]; dropping the builder stops it, and a fetch in flight is
/// dropped too.
pub fn polled<T: 'static, F: Future<Output = T> + 'static>(
    mut fetch: impl FnMut() -> F + 'static,
    again: impl FnMut(&T) -> Option<Duration> + 'static,
) -> Polled<T> {
    Polled {
        fetch: Box::new(move || Box::pin(fetch())),
        again: Box::new(again),
        state: Step::Start,
    }
}

impl<T> Unpin for Polled<T> {}

impl<T> Stream for Polled<T> {
    type Item = T;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Option<T>> {
        let me = &mut *self;
        loop {
            match &mut me.state {
                Step::Start => me.state = Step::Fetch((me.fetch)()),
                Step::Fetch(f) => {
                    let Poll::Ready(v) = f.as_mut().poll(cx) else {
                        return Poll::Pending;
                    };
                    me.state = match (me.again)(&v) {
                        Some(d) => Step::Wait(sleep(d)),
                        None => Step::Done,
                    };
                    return Poll::Ready(Some(v));
                }
                Step::Wait(s) => {
                    if Pin::new(s).poll(cx).is_pending() {
                        return Poll::Pending;
                    }
                    me.state = Step::Start;
                }
                Step::Done => return Poll::Ready(None),
            }
        }
    }
}

/// Collects a stream by keeping only its newest item, for a
/// `stream_builder` over a source that sends whole snapshots (a refreshed
/// list) rather than pieces to append.
pub struct Latest<T>(Option<T>);

impl<T> Latest<T> {
    /// The newest item, once there is one.
    pub fn get(&self) -> Option<&T> {
        self.0.as_ref()
    }
}

impl<T> Default for Latest<T> {
    fn default() -> Self {
        Latest(None)
    }
}

impl<T> Extend<T> for Latest<T> {
    fn extend<I: IntoIterator<Item = T>>(&mut self, items: I) {
        if let Some(v) = items.into_iter().last() {
            self.0 = Some(v);
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub use thread::{Blocking, Wanted, blocking};

#[cfg(not(target_arch = "wasm32"))]
mod thread {
    use std::any::Any;
    use std::future::Future;
    use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
    use std::pin::Pin;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};
    use std::task::{Context, Poll, Waker};

    type Outcome<T> = Result<T, Box<dyn Any + Send>>;

    struct Shared<T> {
        slot: Mutex<(Option<Outcome<T>>, Option<Waker>)>,
        wanted: AtomicBool,
    }

    /// Whether the result of a [`blocking`] call is still awaited. Long work
    /// (several requests in a row) can check it between steps and give up.
    pub struct Wanted(Arc<dyn Fn() -> bool + Send + Sync>);

    impl Wanted {
        /// False once the future was dropped, e.g. with its builder.
        pub fn get(&self) -> bool {
            (self.0)()
        }
    }

    /// The future [`blocking`] returns.
    pub struct Blocking<T>(Arc<Shared<T>>);

    /// Runs `work` on a new thread and resolves with its result, so blocking
    /// calls (HTTP, files) stay off the UI thread. Dropping the future marks
    /// it unwanted for `work`; a panic in `work` resumes when the future is
    /// polled. Not on the web, which has no threads.
    pub fn blocking<T: Send + 'static>(
        work: impl FnOnce(&Wanted) -> T + Send + 'static,
    ) -> Blocking<T> {
        let shared = Arc::new(Shared {
            slot: Mutex::new((None, None)),
            wanted: AtomicBool::new(true),
        });
        let s = shared.clone();
        std::thread::spawn(move || {
            let w = s.clone();
            let wanted = Wanted(Arc::new(move || w.wanted.load(Ordering::Acquire)));
            let out = catch_unwind(AssertUnwindSafe(|| work(&wanted)));
            let waker = {
                let mut slot = s.slot.lock().unwrap();
                slot.0 = Some(out);
                slot.1.take()
            };
            if let Some(w) = waker {
                w.wake();
            }
        });
        Blocking(shared)
    }

    impl<T> Future for Blocking<T> {
        type Output = T;

        fn poll(self: Pin<&mut Self>, cx: &mut Context) -> Poll<T> {
            let mut slot = self.0.slot.lock().unwrap();
            match slot.0.take() {
                Some(Ok(v)) => Poll::Ready(v),
                Some(Err(panic)) => {
                    drop(slot);
                    resume_unwind(panic)
                }
                None => {
                    slot.1 = Some(cx.waker().clone());
                    Poll::Pending
                }
            }
        }
    }

    impl<T> Drop for Blocking<T> {
        fn drop(&mut self) {
            self.0.wanted.store(false, Ordering::Release);
        }
    }
}

/// Values kept by key across rebuilds, e.g. per-row data a list fetches
/// once: rows scrolled away and back build from the cache instead of
/// fetching again. Cheap to clone; keep one in the app or `provide` it.
pub struct Cache<K, V>(Rc<RefCell<HashMap<K, V>>>);

impl<K, V> Clone for Cache<K, V> {
    fn clone(&self) -> Self {
        Cache(self.0.clone())
    }
}

impl<K, V> Default for Cache<K, V> {
    fn default() -> Self {
        Cache(Rc::default())
    }
}

impl<K: Hash + Eq, V: Clone> Cache<K, V> {
    /// An empty cache.
    pub fn new() -> Self {
        Self::default()
    }

    /// A copy of the value under `key`; wrap large values in `Rc`.
    pub fn get(&self, key: &K) -> Option<V> {
        self.0.borrow().get(key).cloned()
    }

    pub fn insert(&self, key: K, value: V) {
        self.0.borrow_mut().insert(key, value);
    }

    /// Forgets `key`, so its next build fetches again.
    pub fn remove(&self, key: &K) {
        self.0.borrow_mut().remove(key);
    }

    pub fn clear(&self) {
        self.0.borrow_mut().clear();
    }
}

/// Builds from `cache`'s value for `key`, or, while there is none, from a
/// `future_builder` over `fetch` that stores what it resolves to under
/// `key`. `None` (a failed fetch) isn't stored, so the next build of this
/// key tries again.
pub fn cached<K, V, F, E>(
    cache: &Cache<K, V>,
    key: K,
    fetch: impl FnOnce(Scope) -> F + 'static,
    build: impl Fn(Option<&V>) -> E + 'static,
) -> Element
where
    K: Hash + Eq + Clone + 'static,
    V: Clone + 'static,
    F: Future<Output = Option<V>> + 'static,
    E: Into<Element>,
{
    if let Some(v) = cache.get(&key) {
        return build(Some(&v)).into();
    }
    let store = cache.clone();
    let k = key.clone();
    let fetch = move |scope: Scope| {
        let f = fetch(scope);
        async move {
            let v = f.await;
            if let Some(v) = &v {
                store.insert(k, v.clone());
            }
            v
        }
    };
    future_builder(key, fetch, move |s: &Snapshot<Option<Option<V>>>| {
        build(s.data.as_ref().and_then(Option::as_ref))
    })
    .into()
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use std::sync::mpsc::channel;

    use super::*;

    #[test]
    fn dropping_blocking_work_marks_it_unwanted() {
        let (go, wait) = channel();
        let (tell, seen) = channel();
        let f = blocking(move |w| {
            tell.send(w.get()).unwrap();
            wait.recv().unwrap();
            tell.send(w.get()).unwrap();
        });
        assert!(seen.recv().unwrap(), "wanted while awaited");
        drop(f);
        go.send(()).unwrap();
        assert!(!seen.recv().unwrap(), "unwanted once dropped");
    }

    #[test]
    fn latest_keeps_the_newest_item() {
        let mut l = Latest::default();
        l.extend([1, 2]);
        l.extend([3]);
        assert_eq!(l.get(), Some(&3));
    }
}
