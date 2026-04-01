use std::fmt::{self, Write};

use tracing::instrument;

use crate::db::{BuildTimeReport, DeclTimeReport, RunReport};

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
#[instrument(skip_all)]
pub(crate) fn render_weekly(
    perfetto_link: Option<String>,
    run_report: RunReport,
    file_build_times: &[BuildTimeReport],
    longest_pole_times: &[BuildTimeReport],
    decl_times: &[DeclTimeReport],
) -> Result<String, fmt::Error> {
    tracing::info!("Generating markdown");
    let mut md = String::new();

    // Build times
    writeln!(md, "## Build Times\n")?;
    writeln!(
        md,
        "**Total**: {}\n",
        fmt_duration(run_report.total_build_secs)
    )?;

    writeln!(md, "| Module | Duration |")?;
    writeln!(md, "|--------|----------|")?;
    for file_report in file_build_times {
        writeln!(
            md,
            "| {} | {} |",
            file_report.module,
            fmt_duration(file_report.duration_secs)
        )?;
    }
    writeln!(md)?;

    if let Some(link) = perfetto_link {
        writeln!(md, "View in [Perfetto!]({link})")?;
    }

    // Longest Build Path
    writeln!(md, "## Longest Build Path\n")?;
    writeln!(
        md,
        "**Total**: {}\n",
        fmt_duration(run_report.total_longest_pole_secs)
    )?;

    writeln!(md, "| Module | Duration |")?;
    writeln!(md, "|--------|----------|")?;
    for file_report in longest_pole_times {
        writeln!(
            md,
            "| {} | {} |",
            file_report.module,
            fmt_duration(file_report.duration_secs)
        )?;
    }
    writeln!(md)?;

    // Slowest Declarations
    writeln!(md, "## Slowest Declarations\n")?;

    writeln!(md, "| File | Declaration | Duration |")?;
    writeln!(md, "|------|-------------|----------|")?;
    for decl_info in decl_times {
        writeln!(
            md,
            "| {} | {} | {} |",
            decl_info.module,
            decl_info.description(),
            fmt_duration(decl_info.elapsed_secs)
        )?;
    }
    writeln!(md)?;

    Ok(md)
}

/// Render a PR differential report as markdown.
pub(crate) fn render_pr(
    perfetto_link: Option<String>,
    run_report: RunReport,
    base_report: RunReport,
    file_build_times: &[BuildTimeReport],
    base_file_build_times: &[BuildTimeReport],
    longest_pole_times: &[BuildTimeReport],
    decl_times: &[DeclTimeReport],
) -> Result<String, fmt::Error> {
    let mut md = String::new();

    // Build time diff
    let total_diff = run_report.total_build_secs - base_report.total_build_secs;
    let sign = if total_diff >= 0.0 { "+" } else { "-" };
    writeln!(md, "## Build Times\n")?;
    writeln!(
        md,
        "**Total**: {} ({}{})\n",
        fmt_duration(run_report.total_build_secs),
        sign,
        fmt_duration(total_diff.abs())
    )?;
    writeln!(
        md,
        "**Base**: {}\n",
        fmt_duration(base_report.total_build_secs),
    )?;

    // Build a map of base durations for diffing
    let base_map: std::collections::HashMap<&str, f64> = base_file_build_times
        .iter()
        .map(|f| (f.module.as_str(), f.duration_secs))
        .collect();

    let mut diffs: Vec<_> = file_build_times
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
        writeln!(md, "| Module | Duration | Delta |")?;
        writeln!(md, "|--------|----------|-------|")?;
        for (module, dur, diff) in &significant {
            let sign = if *diff >= 0.0 { "+" } else { "-" };
            writeln!(
                md,
                "| {} | {} | {}{} |",
                module,
                fmt_duration(*dur),
                sign,
                fmt_duration(diff.abs())
            )?;
        }
        writeln!(md)?;
    }

    if let Some(link) = perfetto_link {
        writeln!(md, "View in [Perfetto!]({link})")?;
    }

    // Longest Build Path
    let pole_diff = run_report.total_longest_pole_secs - base_report.total_longest_pole_secs;
    let sign = if pole_diff >= 0.0 { "+" } else { "-" };
    writeln!(md, "## Longest Build Path\n")?;
    writeln!(
        md,
        "**Total**: {} ({}{})  \n",
        fmt_duration(run_report.total_longest_pole_secs),
        sign,
        fmt_duration(pole_diff.abs()),
    )?;
    writeln!(
        md,
        "**Base**: {}\n",
        fmt_duration(base_report.total_longest_pole_secs),
    )?;

    writeln!(md, "| Module | Duration |")?;
    writeln!(md, "|--------|----------|")?;
    for file_report in longest_pole_times {
        writeln!(
            md,
            "| {} | {} |",
            file_report.module,
            fmt_duration(file_report.duration_secs)
        )?;
    }
    writeln!(md)?;

    // Slowest declarations
    writeln!(md, "## Slowest Declarations\n")?;

    writeln!(md, "| File | Declaration | Duration |")?;
    writeln!(md, "|------|-------------|----------|")?;
    for decl_info in decl_times {
        writeln!(
            md,
            "| {} | {} | {} |",
            decl_info.module,
            decl_info.description(),
            fmt_duration(decl_info.elapsed_secs)
        )?;
    }
    writeln!(md)?;

    Ok(md)
}
