//! Connection secrets (13 §2.1): in the Keychain under the profile's
//! service (12 §1.4), account `ai.<connectionId>`, so deleting the profile
//! removes them. They never enter the state, logs or the UI after saving.

use std::collections::BTreeMap;
use std::sync::Mutex;

/// Where secrets live. The app passes the Keychain; tests keep them in
/// memory.
pub trait Secrets: Send + Sync + std::fmt::Debug {
    fn get(&self, connection: &str) -> Result<Option<String>, String>;
    fn set(&self, connection: &str, secret: &str) -> Result<(), String>;
    fn delete(&self, connection: &str) -> Result<(), String>;
}

/// The Keychain account of a connection's secret.
pub fn account(connection: &str) -> String {
    format!("ai.{connection}")
}

/// The last four characters, the only part the UI shows.
pub fn hint(secret: &str) -> String {
    let chars: Vec<char> = secret.chars().collect();
    chars[chars.len().saturating_sub(4)..].iter().collect()
}

/// Secrets held in memory, for tests and private profiles.
#[derive(Debug, Default)]
pub struct Memory(Mutex<BTreeMap<String, String>>);

impl Secrets for Memory {
    fn get(&self, connection: &str) -> Result<Option<String>, String> {
        Ok(self
            .0
            .lock()
            .map_err(|e| e.to_string())?
            .get(connection)
            .cloned())
    }

    fn set(&self, connection: &str, secret: &str) -> Result<(), String> {
        self.0
            .lock()
            .map_err(|e| e.to_string())?
            .insert(connection.to_owned(), secret.to_owned());
        Ok(())
    }

    fn delete(&self, connection: &str) -> Result<(), String> {
        self.0.lock().map_err(|e| e.to_string())?.remove(connection);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_last_four_characters_show() {
        assert_eq!(hint("sk-ant-1234567890abcd"), "abcd");
        assert_eq!(hint("ab"), "ab");
        assert_eq!(account("c1"), "ai.c1");
        let m = Memory::default();
        m.set("c1", "k").expect("set");
        assert_eq!(m.get("c1").expect("get").as_deref(), Some("k"));
        m.delete("c1").expect("delete");
        assert_eq!(m.get("c1").expect("get"), None);
    }
}
