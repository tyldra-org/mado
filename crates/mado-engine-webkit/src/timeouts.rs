//! One thread that applies replies' safe defaults when the core takes
//! longer than `REPLY_TIMEOUT` (spec 02 §1).

use std::collections::BinaryHeap;
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

use mado_engine::{Expiry, REPLY_TIMEOUT};

type Job = Box<dyn FnOnce() + Send>;

struct Due(Instant, u64, Job);

impl PartialEq for Due {
    fn eq(&self, other: &Self) -> bool {
        (self.0, self.1) == (other.0, other.1)
    }
}
impl Eq for Due {}
impl PartialOrd for Due {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Due {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // Earliest first in a max-heap.
        (other.0, other.1).cmp(&(self.0, self.1))
    }
}

fn sender() -> Option<Sender<(Instant, Job)>> {
    static TX: OnceLock<Mutex<Sender<(Instant, Job)>>> = OnceLock::new();
    let tx = TX.get_or_init(|| {
        let (tx, rx) = mpsc::channel::<(Instant, Job)>();
        let _ = std::thread::Builder::new()
            .name("mado-reply-timeouts".into())
            .spawn(move || {
                let mut heap: BinaryHeap<Due> = BinaryHeap::new();
                let mut seq = 0u64;
                loop {
                    let wait = heap
                        .peek()
                        .map(|d| d.0.saturating_duration_since(Instant::now()));
                    let got = match wait {
                        Some(w) => rx.recv_timeout(w),
                        None => rx.recv().map_err(|_| RecvTimeoutError::Disconnected),
                    };
                    match got {
                        Ok((at, job)) => {
                            seq += 1;
                            heap.push(Due(at, seq, job));
                        }
                        Err(RecvTimeoutError::Timeout) => {}
                        Err(RecvTimeoutError::Disconnected) => return,
                    }
                    while heap.peek().is_some_and(|d| d.0 <= Instant::now()) {
                        if let Some(Due(_, _, job)) = heap.pop() {
                            job();
                        }
                    }
                }
            });
        Mutex::new(tx)
    });
    tx.lock().ok().map(|t| t.clone())
}

/// Applies `expiry`'s default after the reply timeout unless answered.
pub fn arm<T: Send + 'static>(expiry: Expiry<T>) {
    if let Some(tx) = sender() {
        let _ = tx.send((
            Instant::now() + REPLY_TIMEOUT,
            Box::new(move || {
                expiry.expire();
            }),
        ));
    }
}
