//! Disk usage measurement, by top-level folder under the storage root.

use std::collections::BTreeMap;
use std::path::Path;

use crate::error::StorageError;
use crate::root::StorageRoot;

/// Bytes used under `root`, one entry per top-level folder name found there (such as
/// `market-data`, `portfolios` or `sessions`).
///
/// A missing root gives an empty map, never an error: there is simply nothing to
/// measure yet.
///
/// # Errors
///
/// [`StorageError::Io`] when a folder exists but cannot be read.
pub fn usage_by_folder(root: &StorageRoot) -> Result<BTreeMap<String, u64>, StorageError> {
    let entries = match std::fs::read_dir(root.path()) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
        Err(source) => {
            return Err(StorageError::Io {
                action: "read folder",
                path: root.path().to_path_buf(),
                source,
            });
        }
    };
    let mut usage = BTreeMap::new();
    for entry in entries {
        let entry = entry.map_err(StorageError::io("read folder", root.path()))?;
        let is_dir = entry
            .file_type()
            .map_err(StorageError::io("stat", entry.path()))?
            .is_dir();
        if !is_dir {
            continue;
        }
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        usage.insert(name, size_of(&entry.path())?);
    }
    Ok(usage)
}

/// Total size of every file under `path`, recursively.
///
/// Symlinks are not followed (there should be none in the storage folder), so a stray
/// symlink cannot cause an infinite walk.
///
/// # Errors
///
/// [`StorageError::Io`] when a folder or file cannot be read.
pub fn size_of(path: &Path) -> Result<u64, StorageError> {
    let metadata = std::fs::symlink_metadata(path).map_err(StorageError::io("stat", path))?;
    if !metadata.is_dir() {
        return Ok(metadata.len());
    }
    let mut total = 0;
    for entry in std::fs::read_dir(path).map_err(StorageError::io("read folder", path))? {
        let entry = entry.map_err(StorageError::io("read folder", path))?;
        total += size_of(&entry.path())?;
    }
    Ok(total)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usage_by_folder_of_an_empty_root_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        let root = StorageRoot::at(dir.path());
        assert!(usage_by_folder(&root).unwrap().is_empty());
    }

    #[test]
    fn usage_by_folder_of_a_missing_root_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        let root = StorageRoot::at(dir.path().join("nope"));
        assert!(usage_by_folder(&root).unwrap().is_empty());
    }

    #[test]
    fn usage_by_folder_sums_bytes_per_top_level_folder() {
        let dir = tempfile::tempdir().unwrap();
        let root = StorageRoot::at(dir.path());
        std::fs::create_dir_all(dir.path().join("sessions/one")).unwrap();
        std::fs::write(dir.path().join("sessions/one/history.jsonl"), "12345").unwrap();
        std::fs::create_dir_all(dir.path().join("portfolios")).unwrap();
        std::fs::write(dir.path().join("portfolios/current.json"), "123").unwrap();
        let usage = usage_by_folder(&root).unwrap();
        assert_eq!(usage.get("sessions"), Some(&5));
        assert_eq!(usage.get("portfolios"), Some(&3));
    }

    #[test]
    fn size_of_a_single_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("f.txt");
        std::fs::write(&path, "abcdefgh").unwrap();
        assert_eq!(size_of(&path).unwrap(), 8);
    }
}
