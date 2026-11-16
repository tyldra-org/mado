//! The last good copies at `<root>/Lists/` (spec 09 §7). Each is one
//! file, `<list>.signed`: its minisign signature (four lines), then the
//! list, replaced in one rename so a crash never pairs a list with another
//! one's signature. Every profile process shares the folder.

use std::path::{Path, PathBuf};
use std::time::Duration;

use minisign_verify::PublicKey;

use crate::signature::{public_key, verify};
use crate::{List, ListError, current};

/// What [`Store::install`] did with a verified list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Installed {
    /// It is now the current copy, with this serial.
    Updated(u64),
    /// Mado already had this copy.
    Unchanged,
}

pub struct Store {
    dir: PathBuf,
    key: PublicKey,
}

impl std::fmt::Debug for Store {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Store")
            .field("dir", &self.dir)
            .finish_non_exhaustive()
    }
}

/// Splits a `.signed` file into its signature and its list.
fn split(file: &[u8]) -> Option<(&str, &[u8])> {
    let mut end = 0;
    for _ in 0..4 {
        end += file.get(end..)?.iter().position(|b| *b == b'\n')? + 1;
    }
    Some((std::str::from_utf8(&file[..end]).ok()?, &file[end..]))
}

impl Store {
    /// `public_key` is [`crate::PUBLIC_KEY`] outside tests.
    pub fn new(dir: PathBuf, public_key_b64: &str) -> Result<Self, ListError> {
        Ok(Self {
            dir,
            key: public_key(public_key_b64)?,
        })
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn path(&self, list: List) -> PathBuf {
        self.dir.join(format!("{}.signed", list.file()))
    }

    /// The serial of the copy on disk, if one verifies.
    fn on_disk(&self, list: List) -> Result<Option<(u64, Vec<u8>)>, ListError> {
        let file = match std::fs::read(self.path(list)) {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(source) => {
                return Err(ListError::Io {
                    file: list.file(),
                    source,
                });
            }
        };
        let Some((minisig, bytes)) = split(&file) else {
            return Err(ListError::MalformedSignature { file: list.file() });
        };
        let serial = verify(&self.key, list, bytes, minisig)?;
        Ok(Some((serial, bytes.to_vec())))
    }

    /// Makes the last good copies current where they are newer than the
    /// bundled ones. A copy that no longer verifies or reads is left on
    /// disk, unused, until a good one replaces it.
    pub fn load(&self) -> Vec<(List, Result<Installed, ListError>)> {
        List::ALL
            .into_iter()
            .map(|list| (list, self.load_one(list)))
            .collect()
    }

    fn load_one(&self, list: List) -> Result<Installed, ListError> {
        let Some((serial, bytes)) = self.on_disk(list)? else {
            return Ok(Installed::Unchanged);
        };
        match current::set(list, serial, &bytes) {
            Ok(true) => Ok(Installed::Updated(serial)),
            Ok(false) => Ok(Installed::Unchanged),
            Err(reason) => Err(ListError::Invalid {
                file: list.file(),
                reason,
            }),
        }
    }

    /// Checks a downloaded list and its signature, keeps it as the last
    /// good copy and makes it current. A list that fails any check, or is
    /// older than the copy in use or on disk, changes nothing.
    pub fn install(&self, list: List, bytes: &[u8], minisig: &str) -> Result<Installed, ListError> {
        let file = list.file();
        let serial = verify(&self.key, list, bytes, minisig)?;
        let disk = self.on_disk(list).ok().flatten().map_or(0, |(s, _)| s);
        let have = current::serial(list).max(disk);
        if serial < have {
            if disk > current::serial(list) {
                let _ = self.load_one(list);
            }
            return Err(ListError::Older {
                file,
                got: serial,
                have,
            });
        }
        if serial == have && disk == have {
            return Ok(Installed::Unchanged);
        }
        list.check(bytes)?;
        let io = |source| ListError::Io { file, source };
        std::fs::create_dir_all(&self.dir).map_err(io)?;
        let tmp = self.dir.join(format!(".{file}.{}.tmp", std::process::id()));
        let mut out = minisig.trim_end_matches('\n').as_bytes().to_vec();
        out.push(b'\n');
        out.extend_from_slice(bytes);
        std::fs::write(&tmp, &out).map_err(io)?;
        std::fs::rename(&tmp, self.path(list)).map_err(io)?;
        match current::set(list, serial, bytes) {
            Ok(true) => Ok(Installed::Updated(serial)),
            Ok(false) => Ok(Installed::Unchanged),
            Err(reason) => Err(ListError::Invalid { file, reason }),
        }
    }

    fn stamp(&self) -> PathBuf {
        self.dir.join("fetched-at")
    }

    /// How long until the next fetch is due, `every` after the last one
    /// any profile made (Unix milliseconds).
    pub fn next_fetch_in(&self, now: i64, every: Duration) -> Duration {
        let Some(last) = std::fs::read_to_string(self.stamp())
            .ok()
            .and_then(|s| s.trim().parse::<i64>().ok())
        else {
            return Duration::ZERO;
        };
        let every = i64::try_from(every.as_millis()).unwrap_or(i64::MAX);
        let due = last.saturating_add(every).saturating_sub(now);
        Duration::from_millis(u64::try_from(due.clamp(0, every)).unwrap_or(0))
    }

    /// Records a finished fetch.
    pub fn fetched(&self, now: i64) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        std::fs::write(self.stamp(), now.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{compat, trusted_comment};

    struct Keys(minisign::KeyPair);

    impl Keys {
        fn new() -> Self {
            Self(minisign::KeyPair::generate_unencrypted_keypair().expect("keys"))
        }
        fn public(&self) -> String {
            self.0.pk.to_base64()
        }
        fn sign(&self, list: List, serial: u64, bytes: &[u8]) -> String {
            minisign::sign(
                Some(&self.0.pk),
                &self.0.sk,
                std::io::Cursor::new(bytes),
                Some(&trusted_comment(list, serial)),
                None,
            )
            .expect("sign")
            .into_string()
        }
    }

    fn compat_with(engine: &str) -> Vec<u8> {
        format!(r#"{{"minSafeEngine":{{"webkit":"{engine}"}}}}"#).into_bytes()
    }

    /// M3-T01: a tampered list is rejected and the last good copy stays in
    /// use, in this process and the next.
    #[test]
    fn a_tampered_list_is_rejected_and_the_last_good_copy_stays() {
        let dir = tempfile::tempdir().expect("dir");
        let keys = Keys::new();
        let store = Store::new(dir.path().to_owned(), &keys.public()).expect("store");
        let bundled = compat().min_safe_engine.webkit.clone();

        let good = compat_with("700.1");
        let sig = keys.sign(List::Compat, 100, &good);
        assert_eq!(
            store.install(List::Compat, &good, &sig).expect("good"),
            Installed::Updated(100)
        );
        assert_eq!(compat().min_safe_engine.webkit, "700.1");
        assert_ne!(bundled, "700.1");

        let newer = compat_with("800.1");
        let mut tampered = newer.clone();
        tampered[30] = b'9';
        let sig = keys.sign(List::Compat, 200, &newer);
        assert!(matches!(
            store.install(List::Compat, &tampered, &sig),
            Err(ListError::BadSignature { .. })
        ));
        let other = Keys::new().sign(List::Compat, 200, &newer);
        assert!(
            store.install(List::Compat, &newer, &other).is_err(),
            "another key"
        );
        let as_other_list = keys.sign(List::SensitiveHosts, 200, &newer);
        assert!(matches!(
            store.install(List::Compat, &newer, &as_other_list),
            Err(ListError::WrongFile { .. })
        ));
        let replayed = compat_with("600.1");
        let sig = keys.sign(List::Compat, 50, &replayed);
        assert!(matches!(
            store.install(List::Compat, &replayed, &sig),
            Err(ListError::Older { .. })
        ));
        let malformed = br#"{"minSafeEngine":{"webkit":"soon"}}"#;
        let sig = keys.sign(List::Compat, 300, malformed);
        assert!(matches!(
            store.install(List::Compat, malformed, &sig),
            Err(ListError::Invalid { .. })
        ));
        assert_eq!(
            compat().min_safe_engine.webkit,
            "700.1",
            "still the good copy"
        );

        // The copy on disk is the good one, and a tampered file on disk is
        // not used.
        let path = dir.path().join("compat.json.signed");
        let (sig, bytes) = {
            let file = std::fs::read(&path).expect("cached");
            let (s, b) = split(&file).expect("split");
            (s.to_owned(), b.to_vec())
        };
        assert_eq!(bytes, good);
        assert_eq!(
            crate::signature::verify(&store.key, List::Compat, &bytes, &sig).expect("verifies"),
            100
        );
        let mut bad = std::fs::read(&path).expect("cached");
        let last = bad.len() - 3;
        bad[last] = b'9';
        std::fs::write(&path, bad).expect("tamper");
        assert!(matches!(
            store.load_one(List::Compat),
            Err(ListError::BadSignature { .. })
        ));
    }

    #[test]
    fn fetches_are_due_a_day_after_the_last() {
        let dir = tempfile::tempdir().expect("dir");
        let store = Store::new(dir.path().to_owned(), &Keys::new().public()).expect("store");
        let day = Duration::from_secs(86_400);
        assert_eq!(store.next_fetch_in(1_000, day), Duration::ZERO);
        store.fetched(1_000).expect("stamp");
        assert_eq!(
            store.next_fetch_in(1_000 + 3_600_000, day),
            day - Duration::from_secs(3_600)
        );
        assert_eq!(
            store.next_fetch_in(1_000 + 2 * 86_400_000, day),
            Duration::ZERO
        );
    }

    #[test]
    fn a_signed_file_splits_after_four_lines() {
        assert_eq!(
            split(b"a\nb\nc\nd\nbody"),
            Some(("a\nb\nc\nd\n", &b"body"[..]))
        );
        assert_eq!(split(b"a\nb\nc"), None);
    }
}
