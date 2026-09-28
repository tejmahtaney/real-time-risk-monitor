//! Strict validation of an id or filename taken from a URL, before it ever reaches disk.

use crate::error::ApiError;

/// Rejects anything that is not one safe path segment.
///
/// Delegates entirely to `storage::check_part` (via `engine`'s re-export), the same
/// rule `StorageRoot::join`/`subdir` enforce internally, so a value this accepts is
/// guaranteed safe to hand straight to them. Covers `..`, an encoded slash (axum's
/// `Path` extractor decodes `%2F` to a literal `/` before a handler ever sees it, which
/// this then rejects) and an absolute-looking segment.
///
/// # Errors
///
/// [`ApiError::Validation`] (400) when `value` is not a safe single path segment.
pub fn safe_segment(value: &str) -> Result<(), ApiError> {
    engine::check_part(value)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_traversal_and_absolute_segments() {
        for bad in ["..", ".", "", "../etc", "a/b", "/abs", "a\\b"] {
            assert!(
                matches!(safe_segment(bad), Err(ApiError::Validation(_))),
                "{bad:?} should be rejected"
            );
        }
    }

    #[test]
    fn accepts_an_ordinary_id() {
        assert!(safe_segment("20260926T041500Z-k3f9").is_ok());
        assert!(safe_segment("20260926T041500Z-mybook.csv").is_ok());
    }
}
