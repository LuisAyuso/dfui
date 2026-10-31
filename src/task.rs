//! Futures and streams owned by widgets, polled on the UI thread.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Wake, Waker};

use winit::event_loop::EventLoopProxy;

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
    proxy: Mutex<EventLoopProxy<()>>,
    pending: AtomicBool,
}

impl LoopWaker {
    pub fn new(proxy: EventLoopProxy<()>) -> Arc<Self> {
        Arc::new(LoopWaker {
            proxy: Mutex::new(proxy),
            pending: AtomicBool::new(false),
        })
    }

    /// Call before polling, so wakes during the poll send a new event.
    pub fn clear(&self) {
        self.pending.store(false, Ordering::Release);
    }
}

impl Wake for LoopWaker {
    fn wake(self: Arc<Self>) {
        self.wake_by_ref();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        if !self.pending.swap(true, Ordering::AcqRel) {
            // Fails only once the loop has exited.
            let _ = self.proxy.lock().unwrap().send_event(());
        }
    }
}
