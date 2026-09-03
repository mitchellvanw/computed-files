//! The per-clone trust store: `trust.toml` under `XDG_CONFIG_HOME`, one
//! entry per canonical repository root, never anything inside a working tree.

use std::fmt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::fs;

#[derive(Debug)]
pub struct Error(pub String);

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Grants {
    #[serde(default)]
    roots: Vec<PathBuf>,
}

/// The store file, injectable for tests.
pub struct Store {
    path: PathBuf,
}

impl Store {
    pub fn at(path: PathBuf) -> Store {
        Store { path }
    }

    /// `$XDG_CONFIG_HOME/computed/trust.toml`, default `~/.config/computed/trust.toml`.
    pub fn default_path() -> Result<PathBuf, Error> {
        let config = match std::env::var_os("XDG_CONFIG_HOME").filter(|v| !v.is_empty()) {
            Some(x) => PathBuf::from(x),
            None => PathBuf::from(std::env::var_os("HOME").ok_or_else(|| Error("HOME is not set".into()))?).join(".config"),
        };
        Ok(config.join("computed").join("trust.toml"))
    }

    fn read(&self) -> Result<Grants, Error> {
        match std::fs::read_to_string(&self.path) {
            Ok(text) => toml::from_str(&text).map_err(|e| Error(format!("{}: {e}", self.path.display()))),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Grants::default()),
            Err(e) => Err(Error(format!("{}: {e}", self.path.display()))),
        }
    }

    fn save(&self, grants: &Grants) -> Result<(), Error> {
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| Error(format!("{}: {e}", dir.display())))?;
        }
        let text = toml::to_string(grants).map_err(|e| Error(e.to_string()))?;
        fs::write(&self.path, &text).map_err(|e| Error(format!("{}: {e}", self.path.display())))?;
        Ok(())
    }

    /// Records a grant for `root` (canonicalised) and returns the root recorded.
    pub fn grant(&self, root: &Path) -> Result<PathBuf, Error> {
        let root = canonical(root)?;
        let mut grants = self.read()?;
        if !grants.roots.contains(&root) {
            grants.roots.push(root.clone());
            self.save(&grants)?;
        }
        Ok(root)
    }

    /// Removes the grant for `root`; `false` when there was none.
    pub fn revoke(&self, root: &Path) -> Result<bool, Error> {
        let root = canonical(root)?;
        let mut grants = self.read()?;
        let before = grants.roots.len();
        grants.roots.retain(|r| r != &root);
        if grants.roots.len() == before {
            return Ok(false);
        }
        self.save(&grants)?;
        Ok(true)
    }

    pub fn is_trusted(&self, root: &Path) -> Result<bool, Error> {
        let root = match root.canonicalize() {
            Ok(r) => r,
            Err(_) => return Ok(false),
        };
        Ok(self.read()?.roots.contains(&root))
    }
}

fn canonical(path: &Path) -> Result<PathBuf, Error> {
    path.canonicalize().map_err(|e| Error(format!("{}: {e}", path.display())))
}

/// The root a grant is keyed by: the repository root containing `path`, or
/// outside a repository the directory itself, canonical either way.
pub fn root_for(path: &Path) -> Result<PathBuf, Error> {
    match fs::repo_root(path) {
        Some(r) => Ok(r),
        None => canonical(path),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn grant_revoke_and_lookup_round_trip_through_the_store_file() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::at(dir.path().join("nested/computed/trust.toml"));
        let repo = tempfile::tempdir().unwrap();
        fs::create_dir(repo.path().join(".git")).unwrap();
        let root = repo.path().canonicalize().unwrap();
        assert!(!store.is_trusted(&root).unwrap());
        assert_eq!(store.grant(&root).unwrap(), root);
        assert!(store.is_trusted(&root).unwrap());
        assert!(store.grant(&root).unwrap() == root, "granting twice is idempotent");
        let text = fs::read_to_string(dir.path().join("nested/computed/trust.toml")).unwrap();
        assert_eq!(text.matches(root.to_str().unwrap()).count(), 1, "{text}");
        assert!(store.revoke(&root).unwrap());
        assert!(!store.revoke(&root).unwrap());
        assert!(!store.is_trusted(&root).unwrap());
    }

    #[test]
    fn root_for_is_the_repository_root_or_the_directory_itself() {
        let repo = tempfile::tempdir().unwrap();
        fs::create_dir_all(repo.path().join(".git")).unwrap();
        fs::create_dir_all(repo.path().join("a/b")).unwrap();
        let canon = repo.path().canonicalize().unwrap();
        assert_eq!(root_for(&repo.path().join("a/b")).unwrap(), canon);
        let plain = tempfile::tempdir().unwrap();
        fs::create_dir_all(plain.path().join("x")).unwrap();
        assert_eq!(root_for(&plain.path().join("x")).unwrap(), plain.path().join("x").canonicalize().unwrap());
    }

    #[test]
    fn lookup_resolves_symlinks_and_grants_are_per_root() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::at(dir.path().join("trust.toml"));
        let repo = tempfile::tempdir().unwrap();
        fs::create_dir(repo.path().join(".git")).unwrap();
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(repo.path(), &link).unwrap();
        store.grant(&root_for(&link).unwrap()).unwrap();
        assert!(store.is_trusted(&repo.path().canonicalize().unwrap()).unwrap());
        assert!(!store.is_trusted(&repo.path().join("sub")).unwrap());
    }

    #[test]
    fn a_missing_store_is_empty_and_a_malformed_one_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("trust.toml");
        assert!(!Store::at(path.clone()).is_trusted(Path::new("/")).unwrap());
        fs::write(&path, "not = [toml").unwrap();
        assert!(Store::at(path).is_trusted(Path::new("/")).is_err());
    }
}
