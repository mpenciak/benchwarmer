use regex::Regex;
use serde::Serialize;
use std::sync::LazyLock;

/// A single file build time entry extracted from the lakeprof log.
#[derive(Debug, Clone, Serialize)]
pub struct FileBuildTime {
    /// The module name, e.g. "Curve25519Dalek.Specs.Field.FieldElement51.SqrtRatioi"
    pub module: String,
    /// The build duration as a human-readable string from the log, e.g. "156s", "2.3s", "534ms"
    pub duration_display: String,
    /// The build duration in seconds
    pub duration_secs: f64,
}

/// Report of file-by-file build times.
#[derive(Debug, Clone, Serialize)]
pub struct BuildTimesReport {
    pub files: Vec<FileBuildTime>,
    /// Total build time in seconds (sum of all file build times)
    pub total_secs: f64,
}

/// Matches lines like: `[1.799] ✔ [3292/3476] Built Curve25519Dalek.ExternallyVerified (534ms)`
static BUILD_LINE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\[[\d.]+\] \S+ \[\d+/\d+\] Built (\S+) \(([^)]+)\)$").unwrap());

/// Parse duration string like "534ms", "1.2s", "12s", "156s" into seconds.
fn parse_duration(s: &str) -> Option<f64> {
    let s = s.trim();
    if let Some(ms) = s.strip_suffix("ms") {
        ms.parse::<f64>().ok().map(|v| v / 1000.0)
    } else if let Some(secs) = s.strip_suffix('s') {
        secs.parse::<f64>().ok()
    } else {
        None
    }
}

/// Parse lakeprof log output to extract file-by-file build times.
///
/// Matches lines of the form:
/// `[timestamp] ✔|⚠|ℹ [n/total] Built ModuleName (duration)`
pub fn parse_build_times(content: &str) -> BuildTimesReport {
    let mut files = Vec::new();

    for line in content.lines() {
        let Some(caps) = BUILD_LINE_RE.captures(line) else {
            continue;
        };

        let module = caps[1].to_string();
        let duration_display = caps[2].to_string();

        let Some(duration_secs) = parse_duration(&duration_display) else {
            continue;
        };

        files.push(FileBuildTime {
            module,
            duration_display,
            duration_secs,
        });
    }

    let total_secs: f64 = files.iter().map(|f| f.duration_secs).sum();

    BuildTimesReport { files, total_secs }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_duration() {
        assert_eq!(parse_duration("534ms").unwrap(), 0.534);
        assert_eq!(parse_duration("1.2s").unwrap(), 1.2);
        assert_eq!(parse_duration("12s").unwrap(), 12.0);
        assert_eq!(parse_duration("156s").unwrap(), 156.0);
    }

    #[test]
    fn test_parse_build_times() {
        let content = "\
[1.799] ✔ [3292/3476] Built Curve25519Dalek.ExternallyVerified (534ms)
[1.899] ✔ [3293/3476] Built Curve25519Dalek.Tactics (636ms)
[4.901] ⚠ [3296/3476] Built Curve25519Dalek.FunsExternal (1.7s)
some other line
";
        let report = parse_build_times(content);
        assert_eq!(report.files.len(), 3);
        assert_eq!(report.files[0].module, "Curve25519Dalek.ExternallyVerified");
        assert_eq!(report.files[0].duration_secs, 0.534);
        assert_eq!(report.files[2].duration_secs, 1.7);
    }
}
