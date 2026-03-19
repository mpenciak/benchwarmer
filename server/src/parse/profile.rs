use serde::Serialize;

/// A single profiler entry from a Lean trace.profiler output.
///
/// Top-level entries represent declarations. Each may have nested children
/// representing tactic steps, elaboration phases, etc.
#[derive(Debug, Clone, Serialize)]
pub struct ProfileEntry {
    /// The category tag, e.g. "Elab.async", "Elab.definition.value", "Elab.step"
    pub category: String,
    /// Elapsed time in seconds
    pub elapsed_secs: f64,
    /// The description text after the time
    pub description: String,
    /// Nesting depth (0 = top-level declaration)
    pub depth: usize,
    /// Child entries (for hierarchical representation)
    pub children: Vec<ProfileEntry>,
}

/// The result of parsing a profile file.
#[derive(Debug, Clone, Serialize)]
pub struct ProfileReport {
    /// The source file name this profile came from
    pub source_file: String,
    /// Top-level declaration entries (depth 0), with children nested inside
    pub declarations: Vec<ProfileEntry>,
}

/// Parse a single profile log line into a ProfileEntry, if it matches the expected format.
///
/// Expected format: `<indent>[Category] [time] description`
/// where indent is spaces (2 per level).
fn parse_line(line: &str) -> Option<ProfileEntry> {
    // Count leading spaces to determine depth
    let trimmed = line.trim_start();
    if trimmed.is_empty() {
        return None;
    }

    let leading_spaces = line.len() - trimmed.len();
    let depth = leading_spaces / 2;

    // Must start with '['
    if !trimmed.starts_with('[') {
        return None;
    }

    // Extract category: [Category]
    let close_bracket = trimmed.find(']')?;
    let category = trimmed[1..close_bracket].to_string();

    let rest = trimmed[close_bracket + 1..].trim_start();

    // Extract time: [0.123456]
    if !rest.starts_with('[') {
        return None;
    }
    let time_close = rest.find(']')?;
    let time_str = &rest[1..time_close];
    let elapsed_secs: f64 = time_str.parse().ok()?;

    let description = rest[time_close + 1..].trim().to_string();

    Some(ProfileEntry {
        category,
        elapsed_secs,
        description,
        depth,
        children: Vec::new(),
    })
}

/// Parse a profile log file and return only the top-level declaration entries.
///
/// Top-level entries are those with no leading indentation (depth 0).
/// When full hierarchical parsing is desired in the future, the `children`
/// field can be populated by tracking the depth stack.
pub fn parse_profile(source_file: &str, content: &str) -> ProfileReport {
    let mut declarations: Vec<ProfileEntry> = Vec::new();
    // Stack for building the hierarchy: (depth, entry_index_in_parent_children or declarations)
    // For now we collect flat, but structure supports hierarchy.
    let mut stack: Vec<(usize, ProfileEntry)> = Vec::new();

    for line in content.lines() {
        let Some(entry) = parse_line(line) else {
            // Continuation line: append to the most recent entry's description
            let trimmed = line.trim();
            if !trimmed.is_empty() {
                if let Some((_, parent)) = stack.last_mut() {
                    if parent.description.is_empty() {
                        parent.description = trimmed.to_string();
                    } else {
                        parent.description.push('\n');
                        parent.description.push_str(trimmed);
                    }
                }
            }
            continue;
        };

        let depth = entry.depth;

        // Pop entries from the stack that are at the same depth or deeper
        while let Some((d, _)) = stack.last() {
            if *d >= depth {
                let (_, completed) = stack.pop().unwrap();
                if let Some((_, parent)) = stack.last_mut() {
                    parent.children.push(completed);
                } else {
                    declarations.push(completed);
                }
            } else {
                break;
            }
        }

        stack.push((depth, entry));
    }

    // Drain remaining stack
    while let Some((_, completed)) = stack.pop() {
        if let Some((_, parent)) = stack.last_mut() {
            parent.children.push(completed);
        } else {
            declarations.push(completed);
        }
    }

    ProfileReport {
        source_file: source_file.to_string(),
        declarations,
    }
}

/// Clean up a description for display in a markdown table row.
///
/// Strips "elaborating (proof of)" prefixes, removes Lean doc comments (`/-- ... -/`),
/// and takes only the first remaining line.
fn sanitize_description(desc: &str) -> String {
    let desc = desc
        .strip_prefix("elaborating proof of ")
        .or_else(|| desc.strip_prefix("elaborating "))
        .unwrap_or(desc);

    // Strip doc comments: /-- ... -/ possibly spanning multiple lines
    let stripped = if let Some(start) = desc.find("/--") {
        let after_open = &desc[start + 3..];
        if let Some(end) = after_open.find("-/") {
            let before = desc[..start].trim();
            let after = after_open[end + 2..].trim();
            if before.is_empty() {
                after
            } else if after.is_empty() {
                before
            } else {
                after
            }
        } else {
            // Unclosed doc comment — just use everything after it
            after_open.trim()
        }
    } else {
        desc
    };

    // Take only the first non-empty line
    stripped
        .lines()
        .map(|l| l.trim())
        .find(|l| !l.is_empty())
        .unwrap_or("")
        .to_string()
}

/// Extract only declaration-level data (no children) for summary reporting.
pub(crate) fn declaration_summary(report: &ProfileReport) -> Vec<DeclarationSummary> {
    report
        .declarations
        .iter()
        .map(|d| DeclarationSummary {
            category: d.category.clone(),
            elapsed_secs: d.elapsed_secs,
            description: sanitize_description(&d.description),
        })
        .collect()
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct DeclarationSummary {
    pub category: String,
    pub elapsed_secs: f64,
    pub description: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_line_top_level() {
        let line = "[Elab.async] [0.027186] elaborating proof of foo";
        let entry = parse_line(line).unwrap();
        assert_eq!(entry.category, "Elab.async");
        assert_eq!(entry.elapsed_secs, 0.027186);
        assert_eq!(entry.description, "elaborating proof of foo");
        assert_eq!(entry.depth, 0);
    }

    #[test]
    fn test_parse_line_nested() {
        let line = "  [Elab.definition.value] [0.026738] some description";
        let entry = parse_line(line).unwrap();
        assert_eq!(entry.depth, 1);
        assert_eq!(entry.category, "Elab.definition.value");
    }

    #[test]
    fn test_parse_profile_declarations_only() {
        let content = "\
[Elab.async] [0.027186] elaborating proof of foo
  [Elab.definition.value] [0.026738] foo
    [Elab.step] [0.026328] unfold
[Elab.async] [0.096688] elaborating proof of bar
  [Elab.definition.value] [0.095357] bar
";
        let report = parse_profile("test.profile", content);
        assert_eq!(report.declarations.len(), 2);
        assert_eq!(report.declarations[0].elapsed_secs, 0.027186);
        assert_eq!(report.declarations[1].elapsed_secs, 0.096688);
        // Children should be populated
        assert_eq!(report.declarations[0].children.len(), 1);
    }

    #[test]
    fn test_multiple_descendants() {
        let content = "\
[Elab.async] [0.027186] elaborating proof of foo
  [Elab.definition.value] [0.026738] foo
    [Elab.step] [0.026328] unfold
  [Elab.definition.value] [0.095357] bar
    [Elab.step] [0.094000]
      refine ?_
      simp [spec_ok]
    [Elab.step] [0.001000] done
";
        let report = parse_profile("test.profile", content);
        let top_level = &report.declarations[0];
        assert_eq!(top_level.children.len(), 2);
        assert_eq!(top_level.children[1].children.len(), 2);
    }

    #[test]
    fn test_parse_multiline_description() {
        let content = "\
[Elab.async] [0.027186] elaborating proof of foo
  [Elab.definition.value] [0.026738] foo
    [Elab.step] [0.026328]
          unfold some.long.name
          simp [spec_ok]
";
        let report = parse_profile("test.profile", content);
        let elab_step = &report.declarations[0].children[0].children[0];
        assert_eq!(elab_step.category, "Elab.step");
        assert_eq!(
            elab_step.description,
            "unfold some.long.name\nsimp [spec_ok]"
        );
    }

    #[test]
    fn test_sanitize_strips_doc_comment() {
        let desc = "/-- The Schnorr identification protocol as a sigma protocol. -/\ndef SchnorrProtocol : SigmaProtocol where";
        assert_eq!(sanitize_description(desc), "def SchnorrProtocol : SigmaProtocol where");
    }

    #[test]
    fn test_sanitize_strips_elaborating_prefix() {
        assert_eq!(sanitize_description("elaborating proof of foo"), "foo");
        assert_eq!(sanitize_description("elaborating bar"), "bar");
    }

    #[test]
    fn test_sanitize_multiline_no_doc_comment() {
        let desc = "variable (g : G) (hord : orderOf g = q)\n(hcard : Fintype.card G = q)";
        assert_eq!(sanitize_description(desc), "variable (g : G) (hord : orderOf g = q)");
    }

    #[test]
    fn test_declaration_summary() {
        let content = "\
[Elab.async] [0.027186] elaborating proof of foo
  [Elab.definition.value] [0.026738] foo
[Elab.async] [0.096688] elaborating proof of bar
";
        let report = parse_profile("test.profile", content);
        let summary = declaration_summary(&report);
        assert_eq!(summary.len(), 2);
        assert_eq!(summary.len(), 2);
    }
}
