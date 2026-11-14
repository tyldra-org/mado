//! One-time OS authentication tokens (spec 06 §5, 01 §4.2 `app.authenticate`):
//! valid for 60 seconds, usable once.

use std::collections::HashMap;

use mado_settings::AuthTokens;

const VALID_MS: i64 = 60_000;

#[derive(Debug, Default)]
pub struct Tokens {
    issued: HashMap<String, i64>,
    now: i64,
}

impl Tokens {
    /// Sets the clock the next `consume` checks against.
    pub fn at(&mut self, now: i64) -> &mut Self {
        self.now = now;
        self.issued.retain(|_, expires| *expires > now);
        self
    }

    /// Records a token `app.authenticate` handed out after Touch ID.
    pub fn issue(&mut self, token: String, now: i64) {
        self.issued.insert(token, now + VALID_MS);
    }
}

impl AuthTokens for Tokens {
    fn consume(&mut self, token: &str) -> bool {
        self.issued
            .remove(token)
            .is_some_and(|expires| expires > self.now)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_work_once_and_expire() {
        let mut t = Tokens::default();
        t.issue("a".into(), 0);
        t.issue("b".into(), 0);
        assert!(t.at(1000).consume("a"));
        assert!(!t.at(1000).consume("a"));
        assert!(!t.at(61_000).consume("b"));
    }
}
