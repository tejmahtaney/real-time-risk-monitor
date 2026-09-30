//! The one way this crate deletes anything: a single folder, confined to one parent.

use std::fs;

use crate::error::StorageError;
use crate::root::{StorageRoot, check_part};

/// Deletes the folder `<root>/<parent>/<name>` and everything in it.
///
/// This is the only delete in the storage layer and it is meant to be reached only from an explicit
/// user action. It is confined three ways:
///
/// 1. `parent` and `name` must each pass [`check_part`], so neither can hold `..` or a separator.
/// 2. The target must be a real directory, not a symbolic link to one, so a link cannot redirect
///    the delete elsewhere.
/// 3. The target's canonical path must be a direct child of the canonical `<root>/<parent>`.
///
/// Nothing is removed unless all three hold.
///
/// # Errors
///
/// [`StorageError::UnsafePath`] when a check fails, and [`StorageError::Io`] when the folder does
/// not exist (`NotFound`) or cannot be removed.
///
/// ```
/// use storage::{StorageRoot, remove_dir_confined};
///
/// let dir = tempfile::tempdir()?;
/// let root = StorageRoot::at(dir.path());
/// let run = root.subdir(&["backtests", "20260926T090000Z-abcd"])?;
/// std::fs::write(run.join("summary.json"), "{}")?;
/// remove_dir_confined(&root, "backtests", "20260926T090000Z-abcd")?;
/// assert!(!run.exists());
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub fn remove_dir_confined(
    root: &StorageRoot,
    parent: &str,
    name: &str,
) -> Result<(), StorageError> {
    check_part(parent)?;
    check_part(name)?;
    let base = root.path().join(parent);
    let target = base.join(name);
    let meta = fs::symlink_metadata(&target).map_err(StorageError::io("stat", &target))?;
    if meta.file_type().is_symlink() || !meta.is_dir() {
        return Err(StorageError::UnsafePath(target.display().to_string()));
    }
    let real_base = base
        .canonicalize()
        .map_err(StorageError::io("resolve", &base))?;
    let real_target = target
        .canonicalize()
        .map_err(StorageError::io("resolve", &target))?;
    if real_target.parent() != Some(real_base.as_path()) {
        return Err(StorageError::UnsafePath(target.display().to_string()));
    }
    fs::remove_dir_all(&real_target).map_err(StorageError::io("remove", &real_target))
}

#[cfg(test)]
mod tests {
    use std::io::ErrorKind;

    use super::*;

    fn root() -> (tempfile::TempDir, StorageRoot) {
        let dir = tempfile::tempdir().unwrap();
        let root = StorageRoot::at(dir.path());
        (dir, root)
    }

    #[test]
    fn removes_only_the_named_folder() {
        let (_dir, root) = root();
        let keep = root.subdir(&["backtests", "keep"]).unwrap();
        let gone = root.subdir(&["backtests", "gone"]).unwrap();
        fs::write(gone.join("a.csv"), "x").unwrap();
        fs::create_dir(gone.join("config")).unwrap();
        fs::write(gone.join("config").join("b.toml"), "y").unwrap();
        fs::write(keep.join("summary.json"), "{}").unwrap();
        let other = root.subdir(&["sessions", "gone"]).unwrap();

        remove_dir_confined(&root, "backtests", "gone").unwrap();
        assert!(!gone.exists());
        assert!(keep.join("summary.json").exists());
        assert!(
            other.exists(),
            "a folder of the same name elsewhere is untouched"
        );
    }

    #[test]
    fn refuses_anything_that_could_escape() {
        let (_dir, root) = root();
        let victim = root.subdir(&["sessions", "precious"]).unwrap();
        root.subdir(&["backtests", "run"]).unwrap();
        for name in ["..", ".", "", "../sessions/precious", "a/b", "/abs"] {
            assert!(
                matches!(
                    remove_dir_confined(&root, "backtests", name),
                    Err(StorageError::UnsafePath(_))
                ),
                "{name:?}"
            );
        }
        assert!(remove_dir_confined(&root, "..", "sessions").is_err());
        assert!(victim.exists());
    }

    #[test]
    fn a_missing_folder_is_not_found_and_a_file_is_refused() {
        let (_dir, root) = root();
        let base = root.subdir(&["backtests"]).unwrap();
        let error = remove_dir_confined(&root, "backtests", "nope").unwrap_err();
        assert!(
            matches!(&error, StorageError::Io { source, .. } if source.kind() == ErrorKind::NotFound),
            "{error:?}"
        );
        fs::write(base.join("file"), "x").unwrap();
        assert!(matches!(
            remove_dir_confined(&root, "backtests", "file"),
            Err(StorageError::UnsafePath(_))
        ));
        assert!(base.join("file").exists());
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_is_never_followed() {
        let (_dir, root) = root();
        let victim = root.subdir(&["sessions", "precious"]).unwrap();
        fs::write(victim.join("data"), "x").unwrap();
        let base = root.subdir(&["backtests"]).unwrap();
        std::os::unix::fs::symlink(&victim, base.join("link")).unwrap();
        assert!(matches!(
            remove_dir_confined(&root, "backtests", "link"),
            Err(StorageError::UnsafePath(_))
        ));
        assert!(victim.join("data").exists());
    }
}
