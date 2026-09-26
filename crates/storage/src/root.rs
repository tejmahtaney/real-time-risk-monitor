//! The storage root and safe path building.

use std::ffi::OsStr;
use std::fs;
use std::path::{Component, Path, PathBuf};

use crate::error::StorageError;

/// Name of the environment variable that overrides the storage folder.
const ENV_VAR: &str = "RRM_STORAGE_DIR";

/// The folder that holds everything the app saves at run time.
///
/// Every path this type builds stays inside the root: parts that are empty,
/// absolute, `.` or `..`, or that contain a separator or NUL are rejected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorageRoot {
    path: PathBuf,
}

impl StorageRoot {
    /// A root at exactly `path`. Meant for tests, which pass a temp directory.
    /// The folder is created on first use, not here.
    pub fn at(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// Resolves the root from `RRM_STORAGE_DIR`, or `../storage` next to the repo root.
    ///
    /// A thin wrapper over [`resolve_from`]. It reads the environment variable and the
    /// working directory and nothing else.
    ///
    /// # Errors
    ///
    /// [`StorageError::Io`] when the working directory cannot be read and
    /// [`StorageError::RepoRootNotFound`] when no workspace is found above it.
    pub fn resolve() -> Result<Self, StorageError> {
        let env = std::env::var_os(ENV_VAR);
        let cwd = std::env::current_dir().map_err(StorageError::io("read current dir", "."))?;
        resolve_from(env.as_deref(), &cwd).map(Self::at)
    }

    /// The root folder.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The folder `parts` below the root, created if missing.
    ///
    /// # Errors
    ///
    /// [`StorageError::UnsafePath`] for a bad part and [`StorageError::Io`] when the
    /// folder cannot be created.
    pub fn subdir(&self, parts: &[&str]) -> Result<PathBuf, StorageError> {
        let dir = self.join(parts)?;
        fs::create_dir_all(&dir).map_err(StorageError::io("create folder", &dir))?;
        Ok(dir)
    }

    /// The path `parts` below the root, without touching the disk.
    ///
    /// # Errors
    ///
    /// [`StorageError::UnsafePath`] when any part could escape the root.
    pub fn join(&self, parts: &[&str]) -> Result<PathBuf, StorageError> {
        let mut path = self.path.clone();
        for part in parts {
            check_part(part)?;
            path.push(part);
        }
        Ok(path)
    }
}

/// Chooses the storage folder from an environment value and a working directory.
///
/// A non-empty `env_value` is used as given. Otherwise the result is `storage` in the
/// parent of the repo root, where the repo root is the nearest folder at or above `cwd`
/// whose `Cargo.toml` holds a `[workspace]` table. This is pure apart from reading
/// `Cargo.toml` files, so tests pass both arguments in.
///
/// # Errors
///
/// [`StorageError::RepoRootNotFound`] when `env_value` is unset and no workspace
/// folder is found, or the repo root has no parent.
pub fn resolve_from(env_value: Option<&OsStr>, cwd: &Path) -> Result<PathBuf, StorageError> {
    if let Some(value) = env_value.filter(|v| !v.is_empty()) {
        return Ok(PathBuf::from(value));
    }
    let not_found = || StorageError::RepoRootNotFound(cwd.to_path_buf());
    let root = cwd
        .ancestors()
        .find(|dir| is_workspace_root(dir))
        .ok_or_else(not_found)?;
    let parent = root.parent().ok_or_else(not_found)?;
    Ok(parent.join("storage"))
}

fn is_workspace_root(dir: &Path) -> bool {
    fs::read_to_string(dir.join("Cargo.toml"))
        .is_ok_and(|text| text.lines().any(|line| line.trim() == "[workspace]"))
}

/// Rejects a path part that could escape the folder it is joined onto.
pub(crate) fn check_part(part: &str) -> Result<(), StorageError> {
    let bad = part.is_empty()
        || part == "."
        || part == ".."
        || part.contains(['/', '\\', '\0'])
        || Path::new(part)
            .components()
            .any(|c| !matches!(c, Component::Normal(_)));
    if bad {
        Err(StorageError::UnsafePath(part.to_string()))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_value_wins() {
        let got = resolve_from(Some(OsStr::new("/somewhere/else")), Path::new("/nowhere")).unwrap();
        assert_eq!(got, PathBuf::from("/somewhere/else"));
    }

    #[test]
    fn empty_env_value_is_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        fs::create_dir(&repo).unwrap();
        fs::write(repo.join("Cargo.toml"), "[workspace]\nmembers = []\n").unwrap();
        let got = resolve_from(Some(OsStr::new("")), &repo).unwrap();
        assert_eq!(got, dir.path().join("storage"));
    }

    #[test]
    fn walks_up_to_the_workspace_and_uses_its_sibling() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        let nested = repo.join("crates").join("storage");
        fs::create_dir_all(&nested).unwrap();
        fs::write(repo.join("Cargo.toml"), "[workspace]\nmembers = []\n").unwrap();
        // A member manifest without a workspace table must not stop the walk.
        fs::write(nested.join("Cargo.toml"), "[package]\nname = \"storage\"\n").unwrap();
        let got = resolve_from(None, &nested).unwrap();
        assert_eq!(got, dir.path().join("storage"));
    }

    #[test]
    fn missing_workspace_is_a_typed_error() {
        let dir = tempfile::tempdir().unwrap();
        // The temp directory may sit under a Cargo workspace on an odd machine, so only
        // check the error type when nothing was found above it.
        let found_above = dir.path().ancestors().any(is_workspace_root);
        if !found_above {
            let err = resolve_from(None, dir.path()).unwrap_err();
            assert!(matches!(err, StorageError::RepoRootNotFound(_)));
        }
    }

    #[test]
    fn unsafe_parts_are_rejected() {
        let root = StorageRoot::at("/tmp/does-not-matter");
        for bad in [
            "", ".", "..", "../x", "a/b", "a\\b", "/abs", "a\0b", "..\\x", "x/..",
        ] {
            assert!(
                matches!(root.join(&[bad]), Err(StorageError::UnsafePath(_))),
                "{bad:?} should be rejected"
            );
        }
        assert!(root.join(&["market-data", "SPY"]).is_ok());
        assert!(root.join(&["ok", "..", "x"]).is_err());
    }

    #[test]
    fn subdir_creates_folders_inside_the_root() {
        let dir = tempfile::tempdir().unwrap();
        let root = StorageRoot::at(dir.path().join("root"));
        let made = root.subdir(&["market-data", "SPY"]).unwrap();
        assert!(made.is_dir());
        assert!(made.starts_with(root.path()));
    }
}
