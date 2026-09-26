//! Local file storage: storage root, folder layout, atomic writes, append-only logs, readers and disk usage. Generic over serde types and free of HTTP.
//!
//! This part lays the base: the storage root ([`StorageRoot`]), safe path
//! building, atomic JSON writes ([`write_json_atomic`]), never-overwrite
//! timestamped JSON files ([`write_json_new`]) and sorted listings ([`list`]).
//! Nothing here knows about market data or risk types.

#![warn(missing_docs)]

mod error;
mod json;
mod root;

pub use error::StorageError;
pub use json::{list, read_json, timestamp_name, write_json_atomic, write_json_new};
pub use root::{StorageRoot, resolve_from};
