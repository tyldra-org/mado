//! The core's timer wheel (spec 01 §2.2). Timers fire as inputs to the core
//! loop. M1 schedules notice expiry and the daily backup, M2 tab sleeping;
//! later tasks add auto-revoke, routines, updates and filter lists.

use std::time::Duration;

use tokio_util::time::{DelayQueue, delay_queue};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Timer {
    /// Removes a notice from `state.notices`.
    NoticeExpiry(String),
    /// Backs up the profile and vault databases (spec 03 §4).
    DailyBackup,
    /// Looks for tabs to sleep (spec 04 §7).
    SleepCheck,
    /// Looks for pages that stopped sending heartbeats (spec 02 §5.3).
    HeartbeatCheck,
    /// The daily permission auto-revoke (spec 07 §5).
    AutoRevoke,
    /// The daily fetch of the signed lists (spec 09 §7).
    ListsFetch,
    /// The daily filter list update (spec 10 §2).
    BlockerUpdate,
    /// The daily extension update check (spec 09 §10).
    ExtensionUpdate,
    /// Writes the blocker's counts (spec 10 §7).
    BlockerStatsFlush,
    /// An HTTPS-first upgrade had its time (spec 05 §5).
    UpgradeTimeout(mado_types::TabId),
    /// Quit even if downloads did not pause in time.
    QuitNow,
    /// Quit even if clearing on quit is not done (spec 08 §7).
    ClearOnExitLimit,
    /// A ready security update may relaunch now (spec 15 §5).
    SecurityRelaunch,
    /// A private window may have been in the background long enough to lock.
    PrivateLock(mado_types::WindowId),
    /// Deleted bookmarks' Undo ran out.
    ForgetDeletedBookmarks(u64),
    /// A deleted group's Undo ran out.
    ForgetDeletedGroup(mado_types::GroupId),
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
