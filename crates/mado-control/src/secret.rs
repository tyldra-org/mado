//! The per-profile launch secret (spec 01 §1.3): 32 random bytes in
//! `<profile>/launch.secret`, mode 0600.

use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

#[derive(Clone, PartialEq, Eq)]
pub struct LaunchSecret([u8; 32]);

impl std::fmt::Debug for LaunchSecret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("LaunchSecret(..)")
    }
}

impl LaunchSecret {
    /// Reads the secret, creating it with fresh random bytes the first time.
    pub fn load_or_create(path: &Path) -> std::io::Result<Self> {
        if let Ok(bytes) = std::fs::read(path)
            && let Ok(array) = <[u8; 32]>::try_from(bytes.as_slice())
        {
            return Ok(Self(array));
        }
        let mut bytes = [0u8; 32];
        getrandom::fill(&mut bytes).map_err(|e| std::io::Error::other(e.to_string()))?;
        let tmp = path.with_extension("tmp");
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&tmp)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        std::fs::rename(&tmp, path)?;
        Ok(Self(bytes))
    }

    /// Reads an existing secret, as the forwarding process does.
    pub fn read(path: &Path) -> std::io::Result<Self> {
        let bytes = std::fs::read(path)?;
        <[u8; 32]>::try_from(bytes.as_slice())
            .map(Self)
            .map_err(|_| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "launch.secret is not 32 bytes",
                )
            })
    }

    pub fn to_hex(&self) -> String {
        self.0.iter().map(|b| format!("{b:02x}")).collect()
    }

    /// Compares with a hex string in constant time.
    pub fn matches_hex(&self, hex: &str) -> bool {
        let expected = self.to_hex();
        if hex.len() != expected.len() {
            return false;
        }
        expected
            .bytes()
            .zip(hex.bytes())
            .fold(0u8, |acc, (a, b)| acc | (a ^ b))
            == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn the_secret_is_private_stable_and_compared_whole() {
        let dir = tempfile::tempdir().expect("dir");
        let path = dir.path().join("launch.secret");
        let a = LaunchSecret::load_or_create(&path).expect("creates");
        let b = LaunchSecret::load_or_create(&path).expect("reads");
        assert_eq!(a, b);
        assert_eq!(
            std::fs::metadata(&path).expect("meta").permissions().mode() & 0o777,
            0o600
        );
        assert!(a.matches_hex(&a.to_hex()));
        assert!(!a.matches_hex(&a.to_hex()[..63]));
        assert!(!a.matches_hex(&"0".repeat(64)));
    }
}
