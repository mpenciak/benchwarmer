use serde::Serialize;

use crate::parse::decl::{LeanDeclHeader, parse_lean_decl};

/// A structured representation of a profile description, which may be a
/// DeclHeader or a simple string.
#[derive(Debug, Clone, Serialize)]
pub enum ProfileDescription {
    DeclHeader(LeanDeclHeader),
    Simple(String),
}

impl ProfileDescription {
    /// Returns `true` if the description has no content.
    pub fn is_empty(&self) -> bool {
        match self {
            ProfileDescription::DeclHeader(_) => false,
            ProfileDescription::Simple(s) => s.is_empty(),
        }
    }

    /// Append a continuation line to the description text.
    /// Only meaningful for `Simple` variants (continuation lines are collected
    /// before we attempt structured parsing).
    pub fn append_line(&mut self, line: &str) {
        match self {
            ProfileDescription::Simple(s) => {
                if s.is_empty() {
                    *s = line.to_string();
                } else {
                    s.push('\n');
                    s.push_str(line);
                }
            }
            ProfileDescription::DeclHeader(_) => {}
        }
    }

    /// Return just the declaration name for display.
    ///
    /// For `DeclHeader` descriptions: the name (or result type for instances).
    /// For `Simple` descriptions: the sanitized text with elaboration prefixes stripped.
    pub fn display_name(&self) -> String {
        match self {
            ProfileDescription::DeclHeader(h) => {
                if h.keyword == "instance" {
                    h.result_type
                        .as_deref()
                        .unwrap_or("<anonymous instance>")
                        .to_string()
                } else {
                    h.name.as_deref().unwrap_or("<anonymous>").to_string()
                }
            }
            ProfileDescription::Simple(s) => sanitize_description(s),
        }
    }

    /// Try to upgrade a `Simple` description to a `DeclHeader` by running
    /// the Lean declaration parser. Returns the description unchanged if
    /// parsing fails or it is already a `DeclHeader`.
    fn try_upgrade(&mut self) {
        if let ProfileDescription::Simple(s) = self {
            // Strip "elaborating (proof of)" prefix before attempting parse
            let raw = s
                .strip_prefix("elaborating proof of ")
                .or_else(|| s.strip_prefix("elaborating "))
                .unwrap_or(s);
            if let Some(header) = parse_lean_decl(raw) {
                *self = ProfileDescription::DeclHeader(header);
            }
        }
    }
}

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
    pub description: ProfileDescription,
    /// Nesting depth (0 = top-level declaration)
    pub depth: usize,
    /// Child entries (for hierarchical representation)
    pub children: Vec<ProfileEntry>,
}

impl ProfileEntry {
    /// Map the raw profiler category + parsed description into one of
    /// `"definition"`, `"proof"`, or `"other"`.
    pub fn simplified_category(&self) -> &'static str {
        match &self.description {
            ProfileDescription::DeclHeader(h) => match h.keyword.as_str() {
                "theorem" | "lemma" => "proof",
                "def" | "abbrev" | "structure" | "class" | "inductive" | "instance" => "definition",
                _ => "other",
            },
            ProfileDescription::Simple(_) => {
                if self.category == "Elab.async" {
                    "proof"
                } else {
                    "other"
                }
            }
        }
    }
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
        description: ProfileDescription::Simple(description),
        depth,
        children: Vec::new(),
    })
}

/// Parse a profile log file and return only the top-level declaration entries.
///
/// Top-level entries are those with no leading indentation (depth 0).
/// When full hierarchical parsing is desired in the future, the `children`
/// field can be populated by tracking the depth stack.
pub fn parse_profile(source_file: &str, content: impl Iterator<Item = String>) -> ProfileReport {
    let mut declarations: Vec<ProfileEntry> = Vec::new();
    // Stack for building the hierarchy: (depth, entry_index_in_parent_children or declarations)
    // For now we collect flat, but structure supports hierarchy.
    let mut stack: Vec<(usize, ProfileEntry)> = Vec::new();

    for line in content {
        let Some(entry) = parse_line(&line) else {
            // Continuation line: append to the most recent entry's description
            let trimmed = line.trim();
            if !trimmed.is_empty()
                && let Some((_, parent)) = stack.last_mut()
            {
                parent.description.append_line(trimmed);
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

    // Post-process: attempt to upgrade top-level descriptions to structured DeclHeaders
    for decl in &mut declarations {
        decl.description.try_upgrade();
    }

    ProfileReport {
        source_file: source_file.to_string(),
        declarations,
    }
}

/// Strip elaboration prefixes, doc comments, and excess lines from a raw
/// description string, leaving just the bare declaration name.
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
///
/// For `DeclHeader` descriptions the concise `ToString` representation is used
/// (e.g. `"theorem schnorr_complete"`). For `Simple` descriptions the
/// sanitisation logic strips elaboration prefixes to leave just the name.
pub(crate) fn declaration_summary(report: &ProfileReport) -> Vec<DeclarationSummary> {
    report
        .declarations
        .iter()
        .map(|d| DeclarationSummary {
            category: d.simplified_category().to_string(),
            elapsed_secs: d.elapsed_secs,
            declaration: d.description.display_name(),
        })
        .collect()
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct DeclarationSummary {
    pub category: String,
    pub elapsed_secs: f64,
    pub declaration: String,
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
        assert!(
            matches!(&entry.description, ProfileDescription::Simple(s) if s == "elaborating proof of foo")
        );
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
        let report = parse_profile("test.profile", content.lines().map(String::from));
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
        let report = parse_profile("test.profile", content.lines().map(String::from));
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
        let report = parse_profile("test.profile", content.lines().map(String::from));
        let elab_step = &report.declarations[0].children[0].children[0];
        assert_eq!(elab_step.category, "Elab.step");
        assert!(matches!(
            &elab_step.description,
            ProfileDescription::Simple(s) if s == "unfold some.long.name\nsimp [spec_ok]"
        ));
    }

    #[test]
    fn test_decl_header_upgrade_for_top_level() {
        // Top-level entries whose description is a recognisable Lean declaration
        // should be upgraded to DeclHeader after parsing.
        let content = "\
[Elab.async] [0.050000] elaborating /-- Doc. -/\n    theorem schnorr_complete : T
  [Elab.definition.value] [0.049000] schnorr_complete
";
        let report = parse_profile("test.profile", content.lines().map(String::from));
        let desc = &report.declarations[0].description;
        assert!(
            matches!(desc, ProfileDescription::DeclHeader(h) if h.keyword == "theorem"),
            "expected DeclHeader, got {:?}",
            desc,
        );
    }

    #[test]
    fn test_simple_description_stays_simple() {
        // Descriptions that are not valid Lean declarations remain Simple.
        let content = "[Elab.async] [0.010000] running linters\n";
        let report = parse_profile("test.profile", content.lines().map(String::from));
        assert!(matches!(
            &report.declarations[0].description,
            ProfileDescription::Simple(s) if s == "running linters"
        ));
    }

    #[test]
    fn test_declaration_summary_def() {
        let content = "\
[Elab.async] [0.027186] elaborating def myFun : T
  [Elab.definition.value] [0.026738] myFun
";
        let report = parse_profile("test.profile", content.lines().map(String::from));
        let summary = declaration_summary(&report);
        assert_eq!(summary.len(), 1);
        assert_eq!(summary[0].declaration, "myFun");
        assert_eq!(summary[0].category, "definition");
    }

    #[test]
    fn test_declaration_summary_simple_fallback() {
        let content = "\
[Elab.async] [0.027186] running linters
";
        let report = parse_profile("test.profile", content.lines().map(String::from));
        let summary = declaration_summary(&report);
        assert_eq!(summary.len(), 1);
        assert_eq!(summary[0].declaration, "running linters");
        assert_eq!(summary[0].category, "proof");
    }

    #[test]
    fn test_declaration_summary_strips_elaborating_prefix() {
        let content = "\
[Elab.async] [0.027186] elaborating proof of foo.bar.baz
";
        let report = parse_profile("test.profile", content.lines().map(String::from));
        let summary = declaration_summary(&report);
        assert_eq!(summary.len(), 1);
        assert_eq!(summary[0].declaration, "foo.bar.baz");
        assert_eq!(summary[0].category, "proof");
    }

    #[test]
    fn test_sanitize_description_strips_doc_comment() {
        assert_eq!(
            sanitize_description("/-- Doc. -/\n    theorem schnorr_complete : T"),
            "theorem schnorr_complete : T"
        );
    }

    #[test]
    fn test_sanitize_description_strips_elaborating_prefix() {
        assert_eq!(sanitize_description("elaborating proof of foo"), "foo");
        assert_eq!(sanitize_description("elaborating bar"), "bar");
    }

    #[test]
    fn test_sanitize_description_first_line_only() {
        let desc = "variable (g : G) (hord : orderOf g = q)\n(hcard : Fintype.card G = q)";
        assert_eq!(
            sanitize_description(desc),
            "variable (g : G) (hord : orderOf g = q)"
        );
    }
}
