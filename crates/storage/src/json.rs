//! JSON files: atomic overwrite, never-overwrite timestamped files, listing and reading.

use std::fs::{self, File, OpenOptions};
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use chrono::{DateTime, Utc};
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::error::StorageError;
use crate::root::check_part;

/// How many same-second name clashes `write_json_new` tries before giving up.
const MAX_NAME_ATTEMPTS: u32 = 1000;

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// A UTC time in the file-name form `YYYYMMDDTHHMMSSZ`, which sorts in time order.
///
/// ```
/// use chrono::{TimeZone, Utc};
///
/// let t = Utc.with_ymd_and_hms(2026, 9, 26, 14, 5, 9).unwrap();
/// assert_eq!(storage::timestamp_name(t), "20260926T140509Z");
/// ```
pub fn timestamp_name(time: DateTime<Utc>) -> String {
    time.format("%Y%m%dT%H%M%SZ").to_string()
}

fn to_bytes<T: Serialize>(path: &Path, value: &T) -> Result<Vec<u8>, StorageError> {
    let mut bytes = serde_json::to_vec_pretty(value).map_err(|source| StorageError::Json {
        path: path.to_path_buf(),
        source,
    })?;
    bytes.push(b'\n');
    Ok(bytes)
}

/// Writes `value` as JSON to `path` so a crash never leaves half a file.
///
/// The value is serialised first, then written to a temp file in the same folder,
/// synced, and renamed over the target. If anything fails the target keeps its old
/// contents and the temp file is removed. The folder must already exist.
///
/// # Errors
///
/// [`StorageError::Json`] when serialising fails and [`StorageError::Io`] for any file
/// system failure.
pub fn write_json_atomic<T: Serialize>(path: &Path, value: &T) -> Result<(), StorageError> {
    let bytes = to_bytes(path, value)?;
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| StorageError::UnsafePath(path.display().to_string()))?;
    let folder = path.parent().unwrap_or_else(|| Path::new("."));
    let n = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let temp = folder.join(format!(".{name}.tmp-{}-{n}", std::process::id()));

    let result = write_synced(&temp, &bytes)
        .and_then(|()| fs::rename(&temp, path).map_err(StorageError::io("rename onto", path)));
    match result {
        Ok(()) => {
            sync_folder(folder);
            Ok(())
        }
        Err(err) => {
            // Best effort: the temp file may not exist if creating it failed.
            let _ = fs::remove_file(&temp);
            Err(err)
        }
    }
}

fn write_synced(path: &Path, bytes: &[u8]) -> Result<(), StorageError> {
    let mut file = File::create(path).map_err(StorageError::io("create", path))?;
    file.write_all(bytes)
        .map_err(StorageError::io("write", path))?;
    file.sync_all().map_err(StorageError::io("sync", path))
}

/// Flushes the folder entry after a rename. Failure is not an error: the data is safe.
fn sync_folder(folder: &Path) {
    #[cfg(unix)]
    if let Ok(dir) = File::open(folder) {
        let _ = dir.sync_all();
    }
    #[cfg(not(unix))]
    let _ = folder;
}

/// Writes `value` to a new file `<UTC timestamp>-<stem>.json` in `dir` and returns its path.
///
/// The file is opened with `create_new`, so an existing file is never overwritten. If the
/// name is taken within the same second the file becomes `<timestamp>-<stem>_02.json`,
/// then `_03` and so on. The counter sits after the stem because any other place sorts
/// before the plain name in byte order and would break time order. `now` is passed in so
/// tests are deterministic. The folder must already exist.
///
/// # Errors
///
/// [`StorageError::UnsafePath`] for a bad stem, [`StorageError::Json`] when serialising
/// fails and [`StorageError::Io`] for any file system failure.
pub fn write_json_new<T: Serialize>(
    dir: &Path,
    stem: &str,
    now: DateTime<Utc>,
    value: &T,
) -> Result<PathBuf, StorageError> {
    check_part(stem)?;
    let stamp = timestamp_name(now);
    let bytes = to_bytes(dir, value)?;
    for attempt in 1..=MAX_NAME_ATTEMPTS {
        let name = if attempt == 1 {
            format!("{stamp}-{stem}.json")
        } else {
            format!("{stamp}-{stem}_{attempt:02}.json")
        };
        let path = dir.join(name);
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut file) => {
                let written = file.write_all(&bytes).and_then(|()| file.sync_all());
                if let Err(source) = written {
                    let _ = fs::remove_file(&path);
                    return Err(StorageError::Io {
                        action: "write",
                        path,
                        source,
                    });
                }
                sync_folder(dir);
                return Ok(path);
            }
            Err(e) if e.kind() == ErrorKind::AlreadyExists => {}
            Err(source) => {
                return Err(StorageError::Io {
                    action: "create",
                    path,
                    source,
                });
            }
        }
    }
    Err(StorageError::Io {
        action: "find a free name in",
        path: dir.to_path_buf(),
        source: std::io::Error::new(ErrorKind::AlreadyExists, "too many files in one second"),
    })
}

/// The files in `dir` sorted by name, which is time order for [`write_json_new`] files.
///
/// Sub-folders and hidden files (leading `.`, which covers atomic-write temp files) are
/// left out. A folder that does not exist gives an empty list.
///
/// # Errors
///
/// [`StorageError::Io`] when the folder exists but cannot be read.
pub fn list(dir: &Path) -> Result<Vec<PathBuf>, StorageError> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(source) => {
            return Err(StorageError::Io {
                action: "read folder",
                path: dir.to_path_buf(),
                source,
            });
        }
    };
    let mut files = Vec::new();
    for entry in entries {
        let entry = entry.map_err(StorageError::io("read folder", dir))?;
        let hidden = entry.file_name().to_string_lossy().starts_with('.');
        let is_file = entry
            .file_type()
            .map_err(StorageError::io("stat", entry.path()))?
            .is_file();
        if is_file && !hidden {
            files.push(entry.path());
        }
    }
    files.sort_by(|a, b| a.file_name().cmp(&b.file_name()));
    Ok(files)
}

/// Reads and parses a JSON file.
///
/// # Errors
///
/// [`StorageError::Io`] when the file cannot be read and [`StorageError::Json`] when it
/// does not parse as `T`.
pub fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T, StorageError> {
    let bytes = fs::read(path).map_err(StorageError::io("read", path))?;
    serde_json::from_slice(&bytes).map_err(|source| StorageError::Json {
        path: path.to_path_buf(),
        source,
    })
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;
    use serde::ser::{Error as _, SerializeStruct, Serializer};
    use serde::{Deserialize, Serialize};

    use super::*;

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct Doc {
        name: String,
        n: u32,
    }

    fn doc(n: u32) -> Doc {
        Doc {
            name: "x".into(),
            n,
        }
    }

    fn t(s: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 26, 10, 0, s).unwrap()
    }

    /// Fails half way through serialising, after a field was already produced.
    struct Explodes;

    impl Serialize for Explodes {
        fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
            let mut st = s.serialize_struct("Explodes", 2)?;
            st.serialize_field("ok", &1)?;
            Err(S::Error::custom("boom"))
        }
    }

    fn names(dir: &Path) -> Vec<String> {
        let mut out: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        out.sort();
        out
    }

    #[test]
    fn atomic_write_round_trips_and_replaces() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        write_json_atomic(&path, &doc(1)).unwrap();
        write_json_atomic(&path, &doc(2)).unwrap();
        assert_eq!(read_json::<Doc>(&path).unwrap(), doc(2));
        assert_eq!(names(dir.path()), ["state.json"], "no temp file left");
    }

    #[test]
    fn failed_serialisation_leaves_the_old_file_intact() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        write_json_atomic(&path, &doc(1)).unwrap();
        let before = fs::read(&path).unwrap();
        let err = write_json_atomic(&path, &Explodes).unwrap_err();
        assert!(matches!(err, StorageError::Json { .. }));
        assert_eq!(fs::read(&path).unwrap(), before);
        assert_eq!(names(dir.path()), ["state.json"]);
    }

    #[test]
    fn failed_rename_leaves_the_target_and_no_temp_file() {
        let dir = tempfile::tempdir().unwrap();
        // A non-empty folder cannot be replaced by a file, so the rename fails after the
        // temp file was fully written.
        let target = dir.path().join("state.json");
        fs::create_dir(&target).unwrap();
        fs::write(target.join("keep.txt"), "kept").unwrap();
        let err = write_json_atomic(&target, &doc(1)).unwrap_err();
        assert!(matches!(err, StorageError::Io { .. }));
        assert_eq!(fs::read_to_string(target.join("keep.txt")).unwrap(), "kept");
        assert_eq!(
            names(dir.path()),
            ["state.json"],
            "temp file was cleaned up"
        );
    }

    #[test]
    fn new_files_are_named_by_time_and_never_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let a = write_json_new(dir.path(), "daily", t(0), &doc(1)).unwrap();
        let b = write_json_new(dir.path(), "daily", t(0), &doc(2)).unwrap();
        let c = write_json_new(dir.path(), "daily", t(0), &doc(3)).unwrap();
        let d = write_json_new(dir.path(), "daily", t(1), &doc(4)).unwrap();
        let file = |p: &Path| p.file_name().unwrap().to_string_lossy().into_owned();
        assert_eq!(file(&a), "20260926T100000Z-daily.json");
        assert_eq!(file(&b), "20260926T100000Z-daily_02.json");
        assert_eq!(file(&c), "20260926T100000Z-daily_03.json");
        assert_eq!(file(&d), "20260926T100001Z-daily.json");
        assert_eq!(read_json::<Doc>(&a).unwrap(), doc(1));
        assert_eq!(read_json::<Doc>(&b).unwrap(), doc(2));
        // Name order is write order.
        assert_eq!(list(dir.path()).unwrap(), [a, b, c, d]);
    }

    #[test]
    fn a_failed_new_write_creates_no_file() {
        let dir = tempfile::tempdir().unwrap();
        assert!(write_json_new(dir.path(), "daily", t(0), &Explodes).is_err());
        assert!(names(dir.path()).is_empty());
    }

    #[test]
    fn bad_stems_are_rejected() {
        let dir = tempfile::tempdir().unwrap();
        for bad in ["", "..", "../x", "a/b", "/abs", "a\\b"] {
            assert!(
                matches!(
                    write_json_new(dir.path(), bad, t(0), &doc(1)),
                    Err(StorageError::UnsafePath(_))
                ),
                "{bad:?}"
            );
        }
        assert!(names(dir.path()).is_empty());
    }

    #[test]
    fn list_sorts_by_name_and_skips_folders_and_hidden_files() {
        let dir = tempfile::tempdir().unwrap();
        for name in ["b.json", "a.json", ".a.json.tmp-1-0", "c.json"] {
            fs::write(dir.path().join(name), "{}").unwrap();
        }
        fs::create_dir(dir.path().join("sub")).unwrap();
        let listed: Vec<String> = list(dir.path())
            .unwrap()
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(listed, ["a.json", "b.json", "c.json"]);
    }

    #[test]
    fn list_of_a_missing_folder_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        assert!(list(&dir.path().join("nope")).unwrap().is_empty());
    }

    #[test]
    fn read_json_reports_bad_content() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.json");
        fs::write(&path, "not json").unwrap();
        assert!(matches!(
            read_json::<Doc>(&path),
            Err(StorageError::Json { .. })
        ));
        assert!(matches!(
            read_json::<Doc>(&dir.path().join("missing.json")),
            Err(StorageError::Io { .. })
        ));
    }
}
