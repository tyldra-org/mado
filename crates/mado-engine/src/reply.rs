//! One-shot answers the core owes the engine (spec 02 §1): `Reply<T>` for
//! events that wait on a decision, and `Callback<T>` for commands whose
//! result comes back later.
//!
//! A reply is answered exactly once. If the core drops it unanswered, or the
//! adapter's timer calls [`Expiry::expire`] first (after [`REPLY_TIMEOUT`]),
//! the safe default given when the event was raised is applied instead.

use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

/// How long an adapter waits for the core before applying the safe default.
pub const REPLY_TIMEOUT: Duration = Duration::from_secs(30);

/// Called once with the result of a command.
pub type Callback<T> = Box<dyn FnOnce(T) + Send>;

struct Slot<T> {
    callback: Option<Callback<T>>,
    default: Option<T>,
}

fn lock<T>(slot: &Mutex<Slot<T>>) -> MutexGuard<'_, Slot<T>> {
    slot.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Finishes the slot with `value`, or the default when `value` is `None`.
/// Returns whether this call finished it.
fn finish<T>(slot: &Mutex<Slot<T>>, value: Option<T>) -> bool {
    let (callback, value) = {
        let mut s = lock(slot);
        let Some(callback) = s.callback.take() else {
            return false;
        };
        let default = s.default.take();
        (callback, value.or(default))
    };
    if let Some(value) = value {
        callback(value);
    }
    true
}

/// The core's side of a decision the engine waits on.
pub struct Reply<T: Send + 'static> {
    slot: Arc<Mutex<Slot<T>>>,
}

/// The adapter's side: applies the safe default when the core is too slow.
pub struct Expiry<T: Send + 'static> {
    slot: Arc<Mutex<Slot<T>>>,
}

impl<T: Send + 'static> Reply<T> {
    /// A reply whose answer goes to `callback`; `default` is the safe answer
    /// on timeout or when the reply is dropped.
    pub fn new(default: T, callback: impl FnOnce(T) + Send + 'static) -> (Self, Expiry<T>) {
        let slot = Arc::new(Mutex::new(Slot {
            callback: Some(Box::new(callback)),
            default: Some(default),
        }));
        (
            Self {
                slot: Arc::clone(&slot),
            },
            Expiry { slot },
        )
    }

    /// Answers. Does nothing if the adapter already applied the default.
    pub fn send(self, value: T) {
        finish(&self.slot, Some(value));
    }

    /// Whether the answer, or the default, was already given.
    pub fn is_settled(&self) -> bool {
        lock(&self.slot).callback.is_none()
    }
}

impl<T: Send + 'static> Drop for Reply<T> {
    fn drop(&mut self) {
        finish(&self.slot, None);
    }
}

impl<T: Send + 'static> Expiry<T> {
    /// Applies the safe default if the core has not answered. Returns
    /// whether it did.
    pub fn expire(&self) -> bool {
        finish(&self.slot, None)
    }
}

impl<T: Send + 'static> std::fmt::Debug for Reply<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Reply")
            .field("settled", &self.is_settled())
            .finish()
    }
}

impl<T: Send + 'static> std::fmt::Debug for Expiry<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Expiry")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    fn pair() -> (
        Reply<&'static str>,
        Expiry<&'static str>,
        mpsc::Receiver<&'static str>,
    ) {
        let (tx, rx) = mpsc::channel();
        let (reply, expiry) = Reply::new("default", move |v| {
            let _ = tx.send(v);
        });
        (reply, expiry, rx)
    }

    #[test]
    fn an_answer_is_delivered_once_and_the_timer_then_does_nothing() {
        let (reply, expiry, rx) = pair();
        reply.send("answer");
        assert_eq!(rx.try_recv(), Ok("answer"));
        assert!(!expiry.expire());
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn the_timer_applies_the_default_and_a_late_answer_is_ignored() {
        let (reply, expiry, rx) = pair();
        assert!(expiry.expire());
        assert_eq!(rx.try_recv(), Ok("default"));
        assert!(reply.is_settled());
        reply.send("late");
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn a_dropped_reply_gives_the_safe_default() {
        let (reply, _expiry, rx) = pair();
        drop(reply);
        assert_eq!(rx.try_recv(), Ok("default"));
    }
}
