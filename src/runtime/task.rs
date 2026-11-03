//! Futures and streams owned by widgets, polled on the UI thread.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{self, Context, Wake, Waker};
use std::time::Duration;

use web_time::Instant;

use crate::widget::State;

/// Polls a task's source, writing results into its node's state. Returns
/// whether the state changed and whether the source is finished.
pub(crate) type Poll = Box<dyn FnMut(&mut Context, &mut State) -> (bool, bool)>;

pub(crate) struct Task {
    /// Hash of the inputs the task was started from; a change restarts it.
    pub deps: u64,
    /// `None` once finished; the entry stays so equal deps don't restart it.
    pub poll: Option<Poll>,
}

/// Tasks by node id. Dropped with their node, which cancels them.
pub(crate) type Tasks = HashMap<u64, Task>;

/// Polls every live task once; returns whether any changed its node's state.
// ponytail: one waker for all tasks, so any wake polls them all; per-task wakers if there are many.
pub(crate) fn poll(tasks: &mut Tasks, states: &mut HashMap<u64, State>, waker: &Waker) -> bool {
    let mut cx = Context::from_waker(waker);
    let mut changed = false;
    for (id, t) in tasks.iter_mut() {
        if let Some(p) = &mut t.poll {
            let (c, done) = p(&mut cx, states.entry(*id).or_default());
            changed |= c;
            if done {
                t.poll = None;
            }
        }
    }
    changed
}

/// Wakes the event loop from any thread or runtime; one pending wake at a time.
pub(crate) struct LoopWaker {
    wake: Box<dyn Fn() + Send + Sync>,
    pending: AtomicBool,
}

impl LoopWaker {
    pub fn new(wake: impl Fn() + Send + Sync + 'static) -> Arc<Self> {
        Arc::new(LoopWaker {
            wake: Box::new(wake),
            pending: AtomicBool::new(false),
        })
    }

    /// Call before polling, so wakes during the poll send a new event.
    pub fn clear(&self) {
        self.pending.store(false, Ordering::Release);
    }

    /// A wake was sent and not yet handled.
    #[cfg(target_os = "android")]
    pub fn pending(&self) -> bool {
        self.pending.load(Ordering::Acquire)
    }
}

impl Wake for LoopWaker {
    fn wake(self: Arc<Self>) {
        self.wake_by_ref();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        if !self.pending.swap(true, Ordering::AcqRel) {
            (self.wake)();
        }
    }
}

/// Asks the UI to rebuild, from any thread: worker threads hold clones and
/// call `notify` after changing data the view reads; `widgets::watch`
/// rebuilds its subtree when that happens. Notifies before the UI exists, or
/// several in a burst, cost one rebuild.
#[derive(Clone, Default)]
pub struct Notifier(Arc<Notify>);

#[derive(Default)]
pub(crate) struct Notify {
    version: AtomicU64,
    /// The UI's waker, set by the `watch` polling this notifier.
    // ponytail: one slot, so one runtime per notifier; a list if a notifier ever feeds several windows.
    waker: Mutex<Option<Waker>>,
}

impl Notifier {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn notify(&self) {
        self.0.version.fetch_add(1, Ordering::AcqRel);
        if let Some(w) = &*self.0.waker.lock().unwrap() {
            w.wake_by_ref();
        }
    }

    /// Current version; registers `cx`'s waker for the next `notify`.
    pub(crate) fn poll_version(&self, cx: &Context) -> u64 {
        let mut slot = self.0.waker.lock().unwrap();
        if !slot.as_ref().is_some_and(|w| w.will_wake(cx.waker())) {
            *slot = Some(cx.waker().clone());
        }
        drop(slot);
        self.0.version.load(Ordering::Acquire)
    }

    /// Tells notifiers apart, for `watch`'s restart key.
    pub(crate) fn id(&self) -> usize {
        Arc::as_ptr(&self.0) as usize
    }
}

thread_local! {
    /// Pending `sleep` deadlines on the UI thread.
    static TIMERS: RefCell<Vec<(Instant, Waker)>> = const { RefCell::new(Vec::new()) };
    /// Fixed frame time for tests; the wall clock otherwise.
    static CLOCK: Cell<Option<Instant>> = const { Cell::new(None) };
}

/// Frame time: the wall clock unless a test fixed it.
pub(crate) fn now() -> Instant {
    CLOCK.get().unwrap_or_else(Instant::now)
}

pub(crate) fn set_clock(t: Option<Instant>) {
    CLOCK.set(t);
}

/// Completes after `d`, on every platform including the web. Driven by the
/// dfui event loop, so await it in builder sources, not on another runtime.
pub fn sleep(d: Duration) -> Sleep {
    Sleep(now() + d)
}

pub struct Sleep(Instant);

impl Future for Sleep {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context) -> task::Poll<()> {
        if now() >= self.0 {
            return task::Poll::Ready(());
        }
        TIMERS.with_borrow_mut(|t| {
            if !t
                .iter()
                .any(|(d, w)| *d == self.0 && w.will_wake(cx.waker()))
            {
                t.push((self.0, cx.waker().clone()));
            }
        });
        task::Poll::Pending
    }
}

/// Wakes timers due at `now`; returns the next deadline.
pub(crate) fn fire_timers(now: Instant) -> Option<Instant> {
    TIMERS.with_borrow_mut(|t| {
        t.retain(|(d, w)| {
            if *d <= now {
                w.wake_by_ref();
            }
            *d > now
        });
        t.iter().map(|(d, _)| *d).min()
    })
}
