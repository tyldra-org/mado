//! The private window lock (spec 08 §6): with `privacy.lockPrivateWindows`
//! on, a private window five minutes in the background hides its page
//! behind "Unlock with Touch ID" until macOS authenticates the user.

use std::time::Duration;

use mado_engine::{EngineOp, TabCommand};
use mado_types::command::Origin;
use mado_types::{ErrorCode, MadoError, WindowId};
use serde_json::Value;

use super::{Core, Internal, Reply, Timer};

/// How long a private window stays in the background before it locks.
pub const LOCK_AFTER: Duration = Duration::from_secs(300);

impl Core {
    fn lock_on(&self) -> bool {
        self.state.profile.is_private
            && self
                .settings
                .get::<mado_settings::PrivacyLockPrivateWindows>()
    }

    /// A window became or stopped being the key window.
    pub(super) fn window_focus_changed(&mut self, window: WindowId, focused: bool) {
        if focused {
            self.background_since.remove(&window);
        } else if self.state.profile.is_private {
            self.background_since.insert(window, self.now());
            self.timers.schedule(Timer::PrivateLock(window), LOCK_AFTER);
        }
    }

    /// Locks the windows that have been in the background long enough.
    pub(super) fn private_lock_check(&mut self) {
        if !self.lock_on() {
            return;
        }
        let now = self.now();
        let due: Vec<WindowId> = self
            .background_since
            .iter()
            .filter(|(_, since)| {
                now - **since >= i64::try_from(LOCK_AFTER.as_millis()).unwrap_or(i64::MAX)
            })
            .map(|(w, _)| *w)
            .collect();
        for w in due {
            if let Some(window) = self.model.windows.get_mut(&w)
                && !window.locked
            {
                window.locked = true;
                if let Some(tab) = window.active_tab_id {
                    self.outbox
                        .push(EngineOp::Apply(tab, TabCommand::SetVisible(false)));
                }
            }
        }
    }

    /// `window.unlock`: asks macOS to authenticate the user, then shows the
    /// window's page again.
    pub(super) fn unlock_command(
        &mut self,
        origin: &Origin,
        window: WindowId,
        reply_to: &mut Option<Reply>,
    ) -> Result<Value, MadoError> {
        if !origin.is_user() {
            return Err(MadoError::new(
                ErrorCode::Protected,
                "error.auth.notAllowed",
            ));
        }
        let locked = self
            .model
            .windows
            .get(&window)
            .ok_or_else(|| MadoError::new(ErrorCode::NotFound, "error.window.notFound"))?
            .locked;
        if !locked {
            return Ok(Value::Null);
        }
        #[cfg(any(test, feature = "test-hooks"))]
        let fake = self.hooks.auth_success;
        #[cfg(not(any(test, feature = "test-hooks")))]
        let fake = false;
        if let Some(reply) = reply_to.take() {
            let inputs = self.inputs.clone();
            let reason = mado_settings::Strings::english()
                .get("lock.reason")
                .to_owned();
            tokio::task::spawn_blocking(move || {
                let ok = fake || mado_platform::auth::authenticate(&reason).is_ok();
                let _ = inputs.send(super::Input::Internal(Internal::UnlockAnswered {
                    window,
                    ok,
                    reply,
                }));
            });
        }
        Ok(Value::Null)
    }

    pub(super) fn unlock_answered(&mut self, window: WindowId, ok: bool, reply: Reply) {
        if !ok {
            let _ = reply.send(Err(MadoError::new(
                ErrorCode::PermissionDenied,
                "error.auth.failed",
            )));
            return;
        }
        if let Some(w) = self.model.windows.get_mut(&window)
            && std::mem::take(&mut w.locked)
            && let Some(tab) = w.active_tab_id
        {
            self.outbox
                .push(EngineOp::Apply(tab, TabCommand::SetVisible(true)));
        }
        self.background_since.remove(&window);
        let _ = reply.send(Ok(Value::Null));
    }
}
