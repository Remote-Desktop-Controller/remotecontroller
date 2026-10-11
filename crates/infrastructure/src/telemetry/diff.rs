//! In-memory Myers diff engine and binary file discriminator.
//!
//! Enforces:
//! - Exact line metric calculations (`+lines / -lines`, bytes) via Myers algorithm (`similar`).
//! - Automatic binary file detection (null bytes `\0` or size > 2MB) without wasteful line diffing.
//! - Safe snapshot comparison against existing disk content or in-memory baseline.

use similar::{ChangeTag, TextDiff};
use std::path::Path;

/// Maximum file size (2MB) for full text line-by-line diffing.
pub const MAX_DIFF_TEXT_SIZE: usize = 2 * 1024 * 1024;

/// Metrics computed from comparing previous file content with new content.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DiffMetrics {
    pub lines_added: usize,
    pub lines_removed: usize,
    pub total_bytes: usize,
    pub is_binary: bool,
}

/// Computes exact diff metrics between disk file at `path` (if it exists) and `new_content`.
pub fn compute_file_diff(path: &Path, new_content: &[u8]) -> DiffMetrics {
    let old_content = if path.is_file() {
        std::fs::read(path).unwrap_or_default()
    } else {
        Vec::new()
    };

    compute_content_diff(&old_content, new_content)
}

/// Computes diff metrics between two raw byte slices.
pub fn compute_content_diff(old_bytes: &[u8], new_bytes: &[u8]) -> DiffMetrics {
    let total_bytes = new_bytes.len();

    // Check if either old or new content is binary:
    // 1. Exceeds 2MB threshold
    // 2. Contains null byte \0
    if is_binary_payload(old_bytes) || is_binary_payload(new_bytes) {
        return DiffMetrics {
            lines_added: 0,
            lines_removed: 0,
            total_bytes,
            is_binary: true,
        };
    }

    // Try decoding both as UTF-8
    let old_text = match std::str::from_utf8(old_bytes) {
        Ok(s) => s,
        Err(_) => {
            return DiffMetrics {
                lines_added: 0,
                lines_removed: 0,
                total_bytes,
                is_binary: true,
            };
        }
    };

    let new_text = match std::str::from_utf8(new_bytes) {
        Ok(s) => s,
        Err(_) => {
            return DiffMetrics {
                lines_added: 0,
                lines_removed: 0,
                total_bytes,
                is_binary: true,
            };
        }
    };

    // Calculate exact Myers diff
    let diff = TextDiff::from_lines(old_text, new_text);
    let mut lines_added = 0;
    let mut lines_removed = 0;

    for change in diff.iter_all_changes() {
        match change.tag() {
            ChangeTag::Insert => lines_added += 1,
            ChangeTag::Delete => lines_removed += 1,
            ChangeTag::Equal => {}
        }
    }

    DiffMetrics {
        lines_added,
        lines_removed,
        total_bytes,
        is_binary: false,
    }
}

/// Checks whether a byte slice should be treated as binary.
pub fn is_binary_payload(bytes: &[u8]) -> bool {
    if bytes.len() > MAX_DIFF_TEXT_SIZE {
        return true;
    }
    bytes.contains(&0u8)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_text_diff_line_metrics() {
        let old = b"line 1\nline 2\nline 3\n";
        let new = b"line 1\nline 2 modified\nline 3\nline 4 added\n";

        let metrics = compute_content_diff(old, new);
        assert!(!metrics.is_binary);
        assert_eq!(metrics.lines_removed, 1); // "line 2\n" replaced
        assert_eq!(metrics.lines_added, 2); // "line 2 modified\n" + "line 4 added\n"
        assert_eq!(metrics.total_bytes, new.len());
    }

    #[test]
    fn test_new_file_diff() {
        let old = b"";
        let new = b"hello\nworld\n";

        let metrics = compute_content_diff(old, new);
        assert!(!metrics.is_binary);
        assert_eq!(metrics.lines_added, 2);
        assert_eq!(metrics.lines_removed, 0);
        assert_eq!(metrics.total_bytes, new.len());
    }

    #[test]
    fn test_deleted_file_diff() {
        let old = b"alpha\nbeta\ngamma\n";
        let new = b"";

        let metrics = compute_content_diff(old, new);
        assert!(!metrics.is_binary);
        assert_eq!(metrics.lines_added, 0);
        assert_eq!(metrics.lines_removed, 3);
        assert_eq!(metrics.total_bytes, 0);
    }

    #[test]
    fn test_binary_null_byte_detected() {
        let old = b"some text";
        let new = b"data with \0 null byte";

        let metrics = compute_content_diff(old, new);
        assert!(metrics.is_binary);
        assert_eq!(metrics.lines_added, 0);
        assert_eq!(metrics.lines_removed, 0);
        assert_eq!(metrics.total_bytes, new.len());
    }

    #[test]
    fn test_oversized_file_treated_as_binary() {
        // > 2MB
        let oversized = vec![b'a'; MAX_DIFF_TEXT_SIZE + 10];
        let metrics = compute_content_diff(b"", &oversized);
        assert!(metrics.is_binary);
        assert_eq!(metrics.lines_added, 0);
        assert_eq!(metrics.lines_removed, 0);
        assert_eq!(metrics.total_bytes, oversized.len());
    }

    #[test]
    fn test_file_on_disk_diff() {
        let tmp = tempdir().unwrap();
        let file_path = tmp.path().join("source.rs");
        std::fs::write(&file_path, b"fn alpha() {}\n").unwrap();

        let new_content = b"fn alpha() {}\nfn beta() {}\n";
        let metrics = compute_file_diff(&file_path, new_content);

        assert!(!metrics.is_binary);
        assert_eq!(metrics.lines_added, 1);
        assert_eq!(metrics.lines_removed, 0);
    }
}
