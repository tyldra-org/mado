//! State patches (spec 01 §3.1): numbered lists of RFC 6902 operations that
//! move the UI's copy of the state document to the next revision, and the
//! recorder that `#[derive(Patchable)]` setters write into.

use std::collections::BTreeMap;
use std::fmt::Display;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

/// One RFC 6902 operation. Paths are JSON Pointers (`/tabs/<id>/title`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "op", rename_all = "camelCase")]
#[ts(export)]
pub enum PatchOp {
    Add {
        path: String,
        #[ts(type = "unknown")]
        value: Value,
    },
    Remove {
        path: String,
    },
    Replace {
        path: String,
        #[ts(type = "unknown")]
        value: Value,
    },
    Move {
        from: String,
        path: String,
    },
}

impl PatchOp {
    pub fn path(&self) -> &str {
        match self {
            Self::Add { path, .. }
            | Self::Remove { path }
            | Self::Replace { path, .. }
            | Self::Move { path, .. } => path,
        }
    }
}

/// What the core emits as `mado://patch`: the operations of one revision.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Patch {
    pub rev: u64,
    pub ops: Vec<PatchOp>,
}

/// Escapes one JSON Pointer reference token (RFC 6901): `~` and `/`.
pub fn escape_token(token: &str) -> String {
    token.replace('~', "~0").replace('/', "~1")
}

/// Collects operations in order while handlers change the state.
#[derive(Debug, Default)]
pub struct PatchRecorder {
    ops: Vec<PatchOp>,
}

fn to_value<T: Serialize>(value: &T) -> Value {
    // State values are plain data with string map keys; serialising them
    // cannot fail.
    serde_json::to_value(value).expect("state values serialise to JSON")
}

impl PatchRecorder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Records a `replace`. A `replace` right after one on the same path
    /// overwrites it, so one revision carries only the last value.
    pub fn replace<T: Serialize>(&mut self, path: String, value: &T) {
        let value = to_value(value);
        if let Some(PatchOp::Replace {
            path: last,
            value: v,
        }) = self.ops.last_mut()
            && *last == path
        {
            *v = value;
            return;
        }
        self.ops.push(PatchOp::Replace { path, value });
    }

    pub fn add<T: Serialize>(&mut self, path: String, value: &T) {
        self.ops.push(PatchOp::Add {
            path,
            value: to_value(value),
        });
    }

    pub fn remove(&mut self, path: String) {
        self.ops.push(PatchOp::Remove { path });
    }

    pub fn is_empty(&self) -> bool {
        self.ops.is_empty()
    }

    pub fn ops(&self) -> &[PatchOp] {
        &self.ops
    }

    /// Hands over the operations recorded so far, leaving the recorder empty.
    pub fn take(&mut self) -> Vec<PatchOp> {
        std::mem::take(&mut self.ops)
    }
}

/// A state type with generated setters. Derive it with
/// `#[derive(Patchable)]`; see `mado-types-derive`.
pub trait Patchable {
    type Patch<'a>
    where
        Self: 'a;

    /// A handle on `self`, which sits at `path` in the document.
    fn patch<'a>(&'a mut self, path: String, recorder: &'a mut PatchRecorder) -> Self::Patch<'a>;
}

/// A handle on a `Record<Id, T>` in the document.
#[derive(Debug)]
pub struct MapPatch<'a, M> {
    target: &'a mut M,
    path: String,
    recorder: &'a mut PatchRecorder,
}

impl<'a, K, V> MapPatch<'a, BTreeMap<K, V>>
where
    K: Ord + Display,
    V: Serialize,
{
    pub fn new(
        target: &'a mut BTreeMap<K, V>,
        path: String,
        recorder: &'a mut PatchRecorder,
    ) -> Self {
        Self {
            target,
            path,
            recorder,
        }
    }

    fn entry_path(&self, key: &K) -> String {
        format!("{}/{}", self.path, escape_token(&key.to_string()))
    }

    pub fn contains(&self, key: &K) -> bool {
        self.target.contains_key(key)
    }

    pub fn len(&self) -> usize {
        self.target.len()
    }

    pub fn is_empty(&self) -> bool {
        self.target.is_empty()
    }

    /// Adds or replaces an entry (an RFC 6902 `add` does both).
    pub fn insert(&mut self, key: K, value: V) {
        let path = self.entry_path(&key);
        self.recorder.add(path, &value);
        self.target.insert(key, value);
    }

    /// Removes an entry, recording nothing if it was absent.
    pub fn remove(&mut self, key: &K) -> Option<V> {
        let removed = self.target.remove(key)?;
        let path = self.entry_path(key);
        self.recorder.remove(path);
        Some(removed)
    }
}

impl<'a, K, V> MapPatch<'a, BTreeMap<K, V>>
where
    K: Ord + Display,
    V: Serialize + Patchable + 'a,
{
    /// A handle on one entry, or `None` if it does not exist. Takes the map
    /// handle, so `tx.tabs().get(&id)` chains.
    pub fn get(self, key: &K) -> Option<V::Patch<'a>> {
        let path = format!("{}/{}", self.path, escape_token(&key.to_string()));
        let value = self.target.get_mut(key)?;
        Some(value.patch(path, self.recorder))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn consecutive_replaces_on_one_path_coalesce() {
        let mut r = PatchRecorder::new();
        r.replace("/a".into(), &1);
        r.replace("/a".into(), &2);
        r.replace("/b".into(), &3);
        r.replace("/a".into(), &4);
        assert_eq!(
            r.take(),
            vec![
                PatchOp::Replace {
                    path: "/a".into(),
                    value: 2.into()
                },
                PatchOp::Replace {
                    path: "/b".into(),
                    value: 3.into()
                },
                PatchOp::Replace {
                    path: "/a".into(),
                    value: 4.into()
                },
            ]
        );
        assert!(r.is_empty());
    }

    #[test]
    fn pointer_tokens_escape_tilde_and_slash() {
        assert_eq!(escape_token("a/b~c"), "a~1b~0c");
    }

    #[test]
    fn ops_serialise_as_rfc_6902() {
        let op = PatchOp::Replace {
            path: "/x".into(),
            value: 1.into(),
        };
        assert_eq!(
            serde_json::to_value(&op).expect("serialises"),
            serde_json::json!({ "op": "replace", "path": "/x", "value": 1 })
        );
    }
}
