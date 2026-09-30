//! Local file storage: storage root, folder layout, atomic writes, append-only logs, readers and disk usage. Generic over serde types and free of HTTP.
//!
//! This part lays the base: the storage root ([`StorageRoot`]), safe path
//! building, atomic JSON writes ([`write_json_atomic`]), never-overwrite
//! timestamped files ([`write_json_new`], [`write_bytes_new`]), sorted listings
//! ([`list`], [`list_dirs`]), append-only JSON-lines logs ([`append_jsonl`],
//! [`read_jsonl`]) and disk usage ([`usage_by_folder`]).
//! The only delete is [`remove_dir_confined`], for one folder under one parent, reached only from an
//! explicit user action. Nothing here knows about market data or risk types.

#![warn(missing_docs)]

mod error;
mod json;
mod jsonl;
mod remove;
mod root;
mod usage;

pub use error::StorageError;
pub use json::{
    list, list_dirs, read_json, timestamp_name, write_bytes_new, write_json_atomic, write_json_new,
};
pub use jsonl::{append_jsonl, read_jsonl};
pub use remove::remove_dir_confined;
pub use root::{StorageRoot, check_part, resolve_from, slugify};
pub use usage::{size_of, usage_by_folder};
