use std::fmt::Write;
use std::path::Path;

use serde::Serialize;

use crate::parse::{build, profile, trace};

/// Full benchmark report for a single run.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct BenchmarkReport {
    /// File-by-file build times from lakeprof.log
    pub build_times: Option<build::BuildTimesReport>,
    /// Longest pole / critical path from lakeprof.trace_event
    pub longest_pole: Option<trace::LongestPoleReport>,
    /// Per-file declaration profiling data
    pub profiles: Vec<profile::ProfileReport>,
}

/// Generate a benchmark report from an extracted artifact directory.
///
/// Expected layout inside the extracted dir:
/// ```text
/// bench_results/
///   lakeprof.trace_event
///   profiles/
///     ModuleName__SubModule.profile
/// ```
///
/// The lakeprof.log is typically at the repo root level, but since the archive
/// is of `bench_results/`, build times come from parsing the lakeprof.log if present.
pub(crate) fn generate_report(extracted_dir: &Path) -> BenchmarkReport {
    let bench_results = extracted_dir.join("bench_results");
    let base = if bench_results.exists() {
        bench_results
    } else {
        extracted_dir.to_path_buf()
    };

    // Parse build times from lakeprof.log if present
    let build_times = {
        let log_path = base.join("lakeprof.log");
        if log_path.exists() {
            std::fs::read_to_string(&log_path)
                .ok()
                .map(|content| build::parse_build_times(&content))
        } else {
            None
        }
    };

    // Parse longest pole from trace_event
    let longest_pole = {
        let trace_path = base.join("lakeprof.trace_event");
        if trace_path.exists() {
            std::fs::read_to_string(&trace_path)
                .ok()
                .and_then(|content| trace::parse_longest_pole(&content).ok())
        } else {
            None
        }
    };

    // Parse all profile files
    let mut profiles = Vec::new();
    let profile_dir = base.join("profiles");
    if profile_dir.exists() {
        if let Ok(entries) = std::fs::read_dir(&profile_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) == Some("profile") {
                    let source_file = path
                        .file_stem()
                        .map(|s| s.to_string_lossy().replace("__", "/"))
                        .unwrap_or_default();

                    if let Ok(content) = std::fs::read_to_string(&path) {
                        let report = profile::parse_profile(&source_file, &content);
                        profiles.push(report);
                    }
                }
            }
        }
    }

    // Sort profiles by source file name
    profiles.sort_by(|a, b| a.source_file.cmp(&b.source_file));

    BenchmarkReport {
        build_times,
        longest_pole,
        profiles,
    }
}

fn fmt_duration(secs: f64) -> String {
    if secs >= 60.0 {
        format!("{:.0}m {:.0}s", (secs / 60.0).floor(), secs % 60.0)
    } else if secs >= 1.0 {
        format!("{:.1}s", secs)
    } else {
        format!("{:.0}ms", secs * 1000.0)
    }
}

/// Render a weekly benchmark summary as markdown.
pub(crate) fn render_weekly(report: &BenchmarkReport) -> String {
    let mut md = String::new();

    // Build times
    if let Some(bt) = &report.build_times {
        writeln!(md, "## Build Times\n").unwrap();
        writeln!(md, "**Total**: {}\n", fmt_duration(bt.total_secs)).unwrap();

        let mut sorted: Vec<_> = bt.files.iter().collect();
        sorted.sort_by(|a, b| b.duration_secs.partial_cmp(&a.duration_secs).unwrap());

        writeln!(md, "| Module | Duration |").unwrap();
        writeln!(md, "|--------|----------|").unwrap();
        for f in sorted.iter().take(20) {
            writeln!(md, "| {} | {} |", f.module, fmt_duration(f.duration_secs)).unwrap();
        }
        if sorted.len() > 20 {
            writeln!(md, "\n*...and {} more files*\n", sorted.len() - 20).unwrap();
        }
        writeln!(md).unwrap();
    }

    // Longest pole
    if let Some(lp) = &report.longest_pole {
        writeln!(md, "## Longest Build Path\n").unwrap();
        writeln!(md, "**Total**: {}\n", fmt_duration(lp.total_secs)).unwrap();

        writeln!(md, "| Module | Duration |").unwrap();
        writeln!(md, "|--------|----------|").unwrap();
        for e in &lp.entries {
            writeln!(md, "| {} | {} |", e.name, fmt_duration(e.duration_secs)).unwrap();
        }
        writeln!(md).unwrap();
    }

    // Slowest declarations
    if !report.profiles.is_empty() {
        writeln!(md, "## Slowest Declarations\n").unwrap();

        let mut all_decls: Vec<_> = report
            .profiles
            .iter()
            .flat_map(|p| {
                profile::declaration_summary(p)
                    .into_iter()
                    .map(move |d| (p.source_file.clone(), d))
            })
            .collect();
        all_decls.sort_by(|a, b| b.1.elapsed_secs.partial_cmp(&a.1.elapsed_secs).unwrap());

        writeln!(md, "| File | Declaration | Duration |").unwrap();
        writeln!(md, "|------|-------------|----------|").unwrap();
        for (file, d) in all_decls.iter().take(20) {
            let label = if d.category == "Elab.async" {
                format!("proof of {}", d.description)
            } else {
                d.description.clone()
            };
            writeln!(
                md,
                "| {} | {} | {} |",
                file,
                label,
                fmt_duration(d.elapsed_secs)
            )
            .unwrap();
        }
        writeln!(md).unwrap();
    }

    md
}

/// Render a PR differential report as markdown.
pub(crate) fn render_pr(head: &BenchmarkReport, base: &BenchmarkReport) -> String {
    let mut md = String::new();

    // Build time diff
    if let (Some(head_bt), Some(base_bt)) = (&head.build_times, &base.build_times) {
        let total_diff = head_bt.total_secs - base_bt.total_secs;
        let sign = if total_diff >= 0.0 { "+" } else { "-" };
        writeln!(md, "## Build Times\n").unwrap();
        writeln!(
            md,
            "**Total**: {} ({}{})  \n**Base**: {}\n",
            fmt_duration(head_bt.total_secs),
            sign,
            fmt_duration(total_diff.abs()),
            fmt_duration(base_bt.total_secs),
        )
        .unwrap();

        // Build a map of base durations for diffing
        let base_map: std::collections::HashMap<&str, f64> = base_bt
            .files
            .iter()
            .map(|f| (f.module.as_str(), f.duration_secs))
            .collect();

        let mut diffs: Vec<_> = head_bt
            .files
            .iter()
            .map(|f| {
                let base_secs = base_map.get(f.module.as_str()).copied().unwrap_or(0.0);
                let diff = f.duration_secs - base_secs;
                (&f.module, f.duration_secs, diff)
            })
            .collect();
        diffs.sort_by(|a, b| b.2.abs().partial_cmp(&a.2.abs()).unwrap());

        // Only show files with meaningful changes
        let significant: Vec<_> = diffs
            .iter()
            .filter(|(_, _, diff)| diff.abs() >= 0.5)
            .take(20)
            .collect();

        if !significant.is_empty() {
            writeln!(md, "| Module | Duration | Delta |").unwrap();
            writeln!(md, "|--------|----------|-------|").unwrap();
            for (module, dur, diff) in &significant {
                let sign = if *diff >= 0.0 { "+" } else { "-" };
                writeln!(
                    md,
                    "| {} | {} | {}{} |",
                    module,
                    fmt_duration(*dur),
                    sign,
                    fmt_duration(diff.abs())
                )
                .unwrap();
            }
            writeln!(md).unwrap();
        }
    }

    // Longest pole diff
    if let (Some(head_lp), Some(base_lp)) = (&head.longest_pole, &base.longest_pole) {
        let total_diff = head_lp.total_secs - base_lp.total_secs;
        let sign = if total_diff >= 0.0 { "+" } else { "" };
        writeln!(md, "## Longest Build Path\n").unwrap();
        writeln!(
            md,
            "**Total**: {} ({}{})  \n**Base**: {}\n",
            fmt_duration(head_lp.total_secs),
            sign,
            fmt_duration(total_diff.abs()),
            fmt_duration(base_lp.total_secs),
        )
        .unwrap();

        writeln!(md, "| Module | Duration |").unwrap();
        writeln!(md, "|--------|----------|").unwrap();
        for e in &head_lp.entries {
            writeln!(md, "| {} | {} |", e.name, fmt_duration(e.duration_secs)).unwrap();
        }
        writeln!(md).unwrap();
    }

    // Slowest declarations from head
    if !head.profiles.is_empty() {
        writeln!(md, "## Slowest Declarations\n").unwrap();

        let mut all_decls: Vec<_> = head
            .profiles
            .iter()
            .flat_map(|p| {
                profile::declaration_summary(p)
                    .into_iter()
                    .map(move |d| (p.source_file.clone(), d))
            })
            .collect();
        all_decls.sort_by(|a, b| b.1.elapsed_secs.partial_cmp(&a.1.elapsed_secs).unwrap());

        writeln!(md, "| File | Declaration | Duration |").unwrap();
        writeln!(md, "|------|-------------|----------|").unwrap();
        for (file, d) in all_decls.iter().take(20) {
            let label = if d.category == "Elab.async" {
                format!("proof of {}", d.description)
            } else {
                d.description.clone()
            };
            writeln!(
                md,
                "| {} | {} | {} |",
                file,
                label,
                fmt_duration(d.elapsed_secs)
            )
            .unwrap();
        }
        writeln!(md).unwrap();
    }

    md
}
