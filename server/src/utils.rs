use std::path::{Path, PathBuf};

/// Derive the `.trace_event` path from a `.tar.gz` archive path.
///
/// `1.tar.gz` → `1.trace_event`
pub(crate) fn trace_event_path(archive_path: impl AsRef<Path>) -> PathBuf {
    let p = archive_path.as_ref();
    let stem = p
        .file_stem()
        .and_then(|s| Path::new(s).file_stem())
        .unwrap_or_default();
    p.with_file_name(stem).with_extension("trace_event")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_trace_event_path_simple() {
        let result = trace_event_path("1.tar.gz");
        assert_eq!(result, PathBuf::from("1.trace_event"));
    }

    #[test]
    fn test_trace_event_path_with_directory() {
        let result = trace_event_path("/data/org/repo/abc123/3.tar.gz");
        assert_eq!(result, PathBuf::from("/data/org/repo/abc123/3.trace_event"));
    }
}
