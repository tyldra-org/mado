//! Where Mado keeps its files (spec 00, paths).

use std::path::{Path, PathBuf};

use mado_types::ProfileId;

/// Every path of spec 00's table, from one root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    root: PathBuf,
    logs: PathBuf,
}

impl Paths {
    /// `~/Library/Application Support/Mado` and `~/Library/Logs/Mado`.
    /// `MADO_DATA_DIR` replaces both with one folder: the test harness and
    /// side-by-side development builds use it so they never touch a real
    /// profile.
    pub fn system() -> std::io::Result<Self> {
        if let Some(dir) = std::env::var_os("MADO_DATA_DIR") {
            let root = PathBuf::from(dir);
            return Ok(Self {
                logs: root.join("Logs"),
                root,
            });
        }
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "HOME is not set"))?;
        Ok(Self {
            root: home.join("Library/Application Support/Mado"),
            logs: home.join("Library/Logs/Mado"),
        })
    }

    /// Paths under `root`, for tests.
    pub fn at(root: &Path) -> Self {
        Self {
            root: root.to_owned(),
            logs: root.join("Logs"),
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }
    pub fn app_db(&self) -> PathBuf {
        self.root.join("app.sqlite")
    }
    pub fn profile(&self, id: ProfileId) -> PathBuf {
        self.root.join("Profiles").join(id.to_string())
    }
    pub fn profile_db(&self, id: ProfileId) -> PathBuf {
        self.profile(id).join("profile.sqlite")
    }
    pub fn vault_db(&self, id: ProfileId) -> PathBuf {
        self.profile(id).join("vault.sqlite")
    }
    pub fn extensions(&self, id: ProfileId) -> PathBuf {
        self.profile(id).join("Extensions")
    }
    pub fn backups(&self, id: ProfileId) -> PathBuf {
        self.profile(id).join("Backups")
    }
    /// Where damaged databases are moved (spec 03 §5).
    pub fn broken(&self, id: ProfileId) -> PathBuf {
        self.profile(id).join("Broken")
    }
    /// The control endpoint. A Unix socket path must fit in 104 bytes on
    /// macOS, and `<profile>/control.sock` under a long home folder does not,
    /// so the socket lives in a `Mado` folder (mode 0700) in the user's
    /// private temporary folder; the
    /// control entry (`control_entry`) tells clients where.
    pub fn control_socket(&self, id: ProfileId) -> PathBuf {
        std::env::temp_dir().join("Mado").join(format!("{id}.sock"))
    }
    pub fn lock(&self, id: ProfileId) -> PathBuf {
        self.profile(id).join("SingletonLock")
    }
    pub fn launch_secret(&self, id: ProfileId) -> PathBuf {
        self.profile(id).join("launch.secret")
    }
    /// `<root>/Control/<ProfileId>.json`, the CLI's list of running profiles.
    pub fn control_entry(&self, id: ProfileId) -> PathBuf {
        self.root.join("Control").join(format!("{id}.json"))
    }
    pub fn blocker(&self) -> PathBuf {
        self.root.join("Blocker")
    }
    pub fn logs(&self) -> &Path {
        &self.logs
    }
    /// Temporary profiles for guest mode, deleted on close and swept on
    /// the next launch (spec 00).
    pub fn temporary_profiles() -> PathBuf {
        std::env::temp_dir().join("Mado-Temporary")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_paths_hang_off_the_profile_folder() {
        let p = Paths::at(Path::new("/r"));
        let id: ProfileId = "01928f3e-0000-7000-8000-000000000001".parse().expect("id");
        assert_eq!(
            p.profile_db(id),
            Path::new("/r/Profiles/01928f3e-0000-7000-8000-000000000001/profile.sqlite")
        );
        assert_eq!(
            p.control_entry(id),
            Path::new("/r/Control/01928f3e-0000-7000-8000-000000000001.json")
        );
        assert_eq!(p.app_db(), Path::new("/r/app.sqlite"));
    }
}
