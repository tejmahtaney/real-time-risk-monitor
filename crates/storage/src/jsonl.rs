//! Append-only JSON-lines files: batched appends and crash-tolerant reads.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;

use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::error::StorageError;

/// Appends `records`, one JSON object per line, to `path`, creating the file if it does
/// not exist.
///
/// Fsyncs once after the whole batch, not per record: callers (the session recorder)
/// batch several records per call, so this is "flush once a batch," not "flush every
/// record." A no-op for an empty batch, which does not even open the file.
///
/// # Errors
///
/// [`StorageError::Json`] when a record cannot be serialised and [`StorageError::Io`]
/// for any file system failure.
pub fn append_jsonl<T: Serialize>(path: &Path, records: &[T]) -> Result<(), StorageError> {
    if records.is_empty() {
        return Ok(());
    }
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(StorageError::io("open", path))?;
    for record in records {
        serde_json::to_writer(&mut file, record).map_err(|source| StorageError::Json {
            path: path.to_path_buf(),
            source,
        })?;
        file.write_all(b"\n")
            .map_err(StorageError::io("write", path))?;
    }
    file.sync_all().map_err(StorageError::io("sync", path))
}

/// Reads every record from a JSONL file.
///
/// A line that fails to parse *and* is the last line in the file is treated as a
/// partial write from a crash mid-append and is skipped with a warning; a bad line
/// anywhere else is a hard error, since only the last line can legitimately be partial
/// under this append-only, flush-at-least-once-a-second design. A missing file gives an
/// empty vec, matching [`crate::list`]'s rule.
///
/// # Errors
///
/// [`StorageError::Io`] when the file exists but cannot be read, and
/// [`StorageError::Json`] for a bad line that is not the last one.
pub fn read_jsonl<T: DeserializeOwned>(path: &Path) -> Result<Vec<T>, StorageError> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(source) => {
            return Err(StorageError::Io {
                action: "read",
                path: path.to_path_buf(),
                source,
            });
        }
    };
    let lines: Vec<&str> = text.lines().collect();
    let mut records = Vec::with_capacity(lines.len());
    for (i, line) in lines.iter().enumerate() {
        match serde_json::from_str::<T>(line) {
            Ok(record) => records.push(record),
            Err(source) if i + 1 == lines.len() => {
                tracing::warn!(path = %path.display(), error = %source, "skipping a truncated last line in a JSONL file");
            }
            Err(source) => {
                return Err(StorageError::Json {
                    path: path.to_path_buf(),
                    source,
                });
            }
        }
    }
    Ok(records)
}

#[cfg(test)]
mod tests {
    use serde::{Deserialize, Serialize};

    use super::*;

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct Row {
        n: u32,
    }

    #[test]
    fn round_trips_a_batch() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("history.jsonl");
        append_jsonl(&path, &[Row { n: 1 }, Row { n: 2 }]).unwrap();
        append_jsonl(&path, &[Row { n: 3 }]).unwrap();
        assert_eq!(
            read_jsonl::<Row>(&path).unwrap(),
            [Row { n: 1 }, Row { n: 2 }, Row { n: 3 }]
        );
    }

    #[test]
    fn empty_batch_is_a_no_op() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("events.jsonl");
        append_jsonl::<Row>(&path, &[]).unwrap();
        assert!(!path.exists());
    }

    #[test]
    fn missing_file_reads_as_empty() {
        let dir = tempfile::tempdir().unwrap();
        assert!(
            read_jsonl::<Row>(&dir.path().join("nope.jsonl"))
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn a_truncated_last_line_is_skipped_with_the_rest_returned() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("history.jsonl");
        std::fs::write(&path, "{\"n\":1}\n{\"n\":2}\n{\"n\":3").unwrap();
        assert_eq!(
            read_jsonl::<Row>(&path).unwrap(),
            [Row { n: 1 }, Row { n: 2 }]
        );
    }

    #[test]
    fn a_bad_line_that_is_not_last_is_a_hard_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("history.jsonl");
        std::fs::write(&path, "{\"n\":1}\nnot json\n{\"n\":3}\n").unwrap();
        assert!(matches!(
            read_jsonl::<Row>(&path),
            Err(StorageError::Json { .. })
        ));
    }
}
