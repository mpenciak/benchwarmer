use serde::{Deserialize, Serialize};

/// Raw trace event entry from lakeprof's Chrome trace event format.
#[derive(Debug, Clone, Deserialize)]
struct RawTraceEvent {
    name: String,
    ph: Option<String>,
    ts: Option<f64>,
    dur: Option<f64>,
    #[allow(dead_code)]
    pid: Option<i64>,
    tid: Option<serde_json::Value>,
    #[allow(dead_code)]
    cat: Option<String>,
    #[allow(dead_code)]
    args: Option<serde_json::Value>,
}

/// The top-level trace event file structure.
#[derive(Debug, Clone, Deserialize)]
struct TraceEventFile {
    #[serde(rename = "traceEvents")]
    trace_events: Vec<RawTraceEvent>,
}

/// A processed longest-pole entry.
#[derive(Debug, Clone, Serialize)]
pub struct LongestPoleEntry {
    /// Module/file name
    pub name: String,
    /// Start time in microseconds
    pub start_us: f64,
    /// Duration in microseconds
    pub duration_us: f64,
    /// Duration in seconds
    pub duration_secs: f64,
}

/// Result of analyzing the trace for the longest pole (critical path).
#[derive(Debug, Clone, Serialize)]
pub struct LongestPoleReport {
    /// Ordered entries on the critical path
    pub entries: Vec<LongestPoleEntry>,
    /// Total critical path duration in seconds
    pub total_secs: f64,
}

/// Parse a trace_event file and extract the longest pole (critical path).
///
/// The critical path consists of entries where `ph == "X"` (complete events)
/// and `tid == 0` (the critical path thread as identified by lakeprof).
pub fn parse_longest_pole(content: &str) -> Result<LongestPoleReport, serde_json::Error> {
    let file: TraceEventFile = serde_json::from_str(content)?;

    let mut entries: Vec<LongestPoleEntry> = file
        .trace_events
        .iter()
        .filter(|e| {
            let is_x = e.ph.as_deref() == Some("X");
            let is_tid_0 = match &e.tid {
                Some(serde_json::Value::Number(n)) => n.as_i64() == Some(0),
                _ => false,
            };
            is_x && is_tid_0
        })
        .filter_map(|e| {
            let dur = e.dur?;
            let ts = e.ts?;
            Some(LongestPoleEntry {
                name: e.name.clone(),
                start_us: ts,
                duration_us: dur,
                duration_secs: dur / 1_000_000.0,
            })
        })
        .collect();

    // Sort by start time
    entries.sort_by(|a, b| a.start_us.partial_cmp(&b.start_us).unwrap());

    // Total is the sum of durations on the critical path
    let total_secs: f64 = entries.iter().map(|e| e.duration_secs).sum();

    Ok(LongestPoleReport {
        entries,
        total_secs,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_filters_critical_path() {
        let json = r#"{"traceEvents": [
            {"name": "thread_name", "ph": "M", "pid": 0, "tid": 0, "args": {"name": "critical path"}},
            {"name": "Foo", "cat": null, "ph": "X", "ts": 1000.0, "dur": 500.0, "pid": 0, "tid": 0},
            {"name": "Bar", "cat": null, "ph": "X", "ts": 2000.0, "dur": 300.0, "pid": 0, "tid": 1},
            {"name": "Baz", "cat": null, "ph": "X", "ts": 3000.0, "dur": 700.0, "pid": 0, "tid": 0}
        ]}"#;

        let report = parse_longest_pole(json).unwrap();
        assert_eq!(report.entries.len(), 2);
        assert_eq!(report.entries[0].name, "Foo");
        assert_eq!(report.entries[1].name, "Baz");
        assert!((report.total_secs - 0.0012).abs() < 1e-15);
    }
}
