//! Errors for the storage crate.

use std::path::PathBuf;

use thiserror::Error;

/// Something went wrong reading or writing local storage.
#[derive(Debug, Error)]
pub enum StorageError {
    /// A path part was empty, absolute, `.` or `..`, or held a separator or NUL.
    #[error(
        "unsafe path part {0:?}: parts must not be empty, absolute, '.', '..' or hold a separator"
    )]
    UnsafePath(String),
    /// No folder holding a workspace `Cargo.toml` was found above the start folder.
    #[error("no workspace Cargo.toml found at or above {0}")]
    RepoRootNotFound(PathBuf),
    /// A file system call failed.
    #[error("{action} {path}: {source}")]
    Io {
        /// What was being attempted, such as "create folder".
        action: &'static str,
        /// The path involved.
        path: PathBuf,
        /// The underlying error.
        #[source]
        source: std::io::Error,
    },
    /// A value could not be turned into JSON, or a file held invalid JSON.
    #[error("json for {path}: {source}")]
    Json {
        /// The file involved, or a description of the target.
        path: PathBuf,
        /// The underlying error.
        #[source]
        source: serde_json::Error,
    },
}

impl StorageError {
    pub(crate) fn io(
        action: &'static str,
        path: impl Into<PathBuf>,
    ) -> impl FnOnce(std::io::Error) -> Self {
        let path = path.into();
        move |source| Self::Io {
            action,
            path,
            source,
        }
    }
}
