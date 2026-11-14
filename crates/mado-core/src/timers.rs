//! The core's timer wheel (spec 01 §2.2). Timers fire as inputs to the core
//! loop. M1 schedules notice expiry and the daily backup; later tasks add
//! tab sleeping, auto-revoke, routines, updates and filter lists.

use std::time::Duration;

use tokio_util::time::{DelayQueue, delay_queue};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Timer {
    /// Removes a notice from `state.notices`.
    NoticeExpiry(String),
    /// Backs up the profile and vault databases (spec 03 §4).
    DailyBackup,
}

#[derive(Debug, Default)]
pub struct Timers {
    queue: DelayQueue<Timer>,
}

impl Timers {
    pub fn schedule(&mut self, timer: Timer, after: Duration) -> delay_queue::Key {
        self.queue.insert(timer, after)
    }

    /// The next timer to fire. Pending forever while none is scheduled.
    pub async fn next(&mut self) -> Timer {
        loop {
            if self.queue.is_empty() {
                std::future::pending::<()>().await;
            }
            if let Some(expired) = std::future::poll_fn(|cx| self.queue.poll_expired(cx)).await {
                return expired.into_inner();
            }
        }
    }
}
