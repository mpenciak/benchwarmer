use std::fmt::{Display, Formatter};

use serde::Serialize;
use winnow::{
    Parser, Result as PResult,
    ascii::{line_ending, multispace0, multispace1, space0},
    combinator::{alt, delimited, eof, not, opt, peek, preceded, repeat, terminated},
    error::ContextError,
    token::{literal, take_till, take_until, take_while},
};

/// The result of parsing a Lean declaration header from a profile description.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LeanDeclHeader {
    /// The doc comment text, trimmed, without the `/-- -/` delimiters.
    pub doc_comment: Option<String>,
    /// Attribute strings, e.g. `["simp, norm_num"]` from `@[simp, norm_num]`.
    pub attributes: Vec<String>,
    /// Modifier keywords, e.g. `["private", "noncomputable"]`.
    pub modifiers: Vec<String>,
    /// The declaration keyword: `"theorem"`, `"def"`, `"instance"`, etc.
    pub keyword: String,
    /// The declaration name, if present. Absent for anonymous `instance` declarations.
    pub name: Option<String>,
    /// The return type of the declaration. `None` when the type is inferred
    pub result_type: Option<String>,
}

impl Display for LeanDeclHeader {
    /// A concise display string, e.g. `"theorem schnorr_complete"` or `"instance : NeZero q"`.
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        if self.keyword == "instance" {
            write!(
                f,
                "{} {}",
                self.keyword,
                self.result_type.as_deref().unwrap_or(": <unknown>"),
            )
        } else {
            write!(
                f,
                "{} {}",
                self.keyword,
                self.name.as_deref().unwrap_or("<anonymous>"),
            )
        }
    }
}

/// Parse a Lean declaration header from a top level node of a profiler run
/// Returns `None` if the description does not begin with a recognisable Lean declaration.
pub(crate) fn parse_lean_decl(input: &str) -> Option<LeanDeclHeader> {
    let mut s = input.trim();
    lean_decl_header.parse_next(&mut s).ok()
}

// ---------------------------------------------------------------------------
// Internal parsers
// ---------------------------------------------------------------------------

fn lean_decl_header(input: &mut &str) -> PResult<LeanDeclHeader> {
    // Zero or more `set_option <name> <value> in` stanzas (always precede the doc comment)
    let _: Vec<()> = repeat(0.., set_option_in).parse_next(input)?;
    multispace0.parse_next(input)?;

    // Optional `/-- ... -/` doc comment
    let doc_comment = opt(doc_comment).parse_next(input)?;
    multispace0.parse_next(input)?;

    // Zero or more `@[attr, ...]` attribute blocks
    let raw_attrs: Vec<&str> = repeat(
        0..,
        terminated(
            delimited(literal("@["), take_until(0.., "]"), literal("]")),
            multispace0,
        ),
    )
    .parse_next(input)?;

    // Zero or more modifier keywords (each followed by whitespace)
    let raw_modifiers: Vec<&str> =
        repeat(0.., terminated(modifier_keyword, multispace1)).parse_next(input)?;

    // Mandatory declaration keyword
    let keyword: &str = decl_keyword.parse_next(input)?;

    // Optional name — absent for anonymous `instance` declarations.
    let name: Option<&str> = opt(preceded(multispace1, lean_ident)).parse_next(input)?;

    // For instances, try to parse binders then extract the type from the first line.
    let instance_type = if keyword == "instance" {
        opt(instance_type_spec).parse_next(input)?
    } else {
        None
    };

    Ok(LeanDeclHeader {
        doc_comment: doc_comment.map(|s: &str| s.trim().to_string()),
        attributes: raw_attrs.into_iter().map(String::from).collect(),
        modifiers: raw_modifiers.into_iter().map(String::from).collect(),
        keyword: keyword.to_string(),
        name: name.map(String::from),
        result_type: instance_type,
    })
}

/// Parses a `/-- ... -/` doc comment and returns the inner text (without delimiters).
fn doc_comment<'s>(input: &mut &'s str) -> PResult<&'s str> {
    delimited(literal("/--"), take_until(0.., "-/"), literal("-/")).parse_next(input)
}

/// Skips a `set_option <name> <value> in` stanza, consuming to end of line.
fn set_option_in(input: &mut &str) -> PResult<()> {
    (
        literal("set_option"),
        take_until(0.., "in"),
        literal("in"),
        opt(line_ending),
    )
        .void()
        .parse_next(input)
}

/// Parses one modifier keyword. Requires trailing whitespace to enforce a word boundary.
fn modifier_keyword<'s>(input: &mut &'s str) -> PResult<&'s str> {
    terminated(
        alt((
            literal("noncomputable"),
            literal("protected"),
            literal("private"),
            literal("partial"),
            literal("unsafe"),
            literal("nonrec"),
            literal("meta"),
        )),
        peek(multispace1),
    )
    .parse_next(input)
}

/// Parses a declaration keyword. Requires trailing whitespace or end-of-input so that
/// e.g. "definition" does not match as "def".
fn decl_keyword<'s>(input: &mut &'s str) -> PResult<&'s str> {
    // `alt` supports at most 10 alternatives per tuple, so we nest two groups.
    terminated(
        alt((alt((
            literal("theorem"),
            literal("lemma"),
            literal("inductive"),
            literal("instance"),
            literal("structure"),
            literal("abbrev"),
            literal("class"),
            literal("def"),
        )),)),
        peek(alt((multispace1.void(), eof.void()))),
    )
    .parse_next(input)
}

/// Parses a Lean identifier: alphanumeric characters, underscores, primes, and dots
fn lean_ident<'s>(input: &mut &'s str) -> PResult<&'s str> {
    take_while(1.., |c: char| {
        c.is_alphanumeric() || c == '_' || c == '\'' || c == '.'
    })
    .parse_next(input)
}

/// Parses the instance type from the remainder of the first line after the keyword/name.
/// Skips any binder groups (for future use), then consumes the `:` type separator and
/// takes everything up to `:=`, `where`, or end of line.
///
/// Stays on the first line (uses `space0`, not `multispace0`, between tokens).
fn instance_type_spec(input: &mut &str) -> PResult<String> {
    space0.parse_next(input)?;

    // Parse binder groups — discarded for now, but parsed structurally for future use.
    let _binders: Vec<&str> = repeat(0.., preceded(space0, balanced_group)).parse_next(input)?;

    space0.parse_next(input)?;

    // Consume the type colon, failing (with backtrack) if it's part of ':='
    (literal(":"), not(peek(literal("="))))
        .void()
        .parse_next(input)?;

    space0.parse_next(input)?;

    // Take the rest of the first line as the raw type text
    let rest: &str = take_till(0.., ['\n', '\r']).parse_next(input)?;
    let ty = rest.trim();

    // Strip trailing ':=' or 'where' keyword
    let ty = if let Some(idx) = ty.find(":=") {
        ty[..idx].trim()
    } else if let Some(idx) = ty.find(" where") {
        ty[..idx].trim()
    } else {
        ty
    };

    if ty.is_empty() {
        return Err(ContextError::new());
    }

    Ok(ty.to_string())
}

/// Parses a single balanced delimiter group — `(...)`, `[...]`, or `{...}` — handling
/// nesting of the same delimiter. Returns the full span including the delimiters.
fn balanced_group<'s>(input: &mut &'s str) -> PResult<&'s str> {
    let s = *input;
    let mut chars = s.char_indices();

    let (_, first) = chars.next().ok_or(ContextError::new())?;

    let (open, close) = match first {
        '(' => ('(', ')'),
        '[' => ('[', ']'),
        '{' => ('{', '}'),
        _ => return Err(ContextError::new()),
    };

    let mut depth = 1u32;
    for (i, c) in chars {
        if c == open {
            depth += 1;
        } else if c == close {
            depth -= 1;
            if depth == 0 {
                let end = i + c.len_utf8();
                *input = &s[end..];
                return Ok(&s[..end]);
            }
        }
    }

    // Unclosed delimiter
    Err(ContextError::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_def() {
        let h = parse_lean_decl("def foo : Nat := 42").unwrap();
        assert_eq!(h.keyword, "def");
        assert_eq!(h.name, Some("foo".to_string()));
        assert!(h.modifiers.is_empty());
        assert!(h.attributes.is_empty());
        assert!(h.doc_comment.is_none());
    }

    #[test]
    fn test_theorem_with_doc_comment() {
        let input = "/-- Completeness: honest prover always convinces honest verifier. -/\n    theorem schnorr_complete (x e r : ZMod q) :";
        let h = parse_lean_decl(input).unwrap();
        assert_eq!(h.keyword, "theorem");
        assert_eq!(h.name, Some("schnorr_complete".to_string()));
        assert_eq!(
            h.doc_comment,
            Some("Completeness: honest prover always convinces honest verifier.".to_string())
        );
    }

    #[test]
    fn test_private_anonymous_instance() {
        let h = parse_lean_decl("private instance : NeZero q :=").unwrap();
        assert_eq!(h.keyword, "instance");
        assert_eq!(h.name, None);
        assert_eq!(h.modifiers, vec!["private"]);
        assert_eq!(h.result_type, Some("NeZero q".to_string()));
    }

    #[test]
    fn test_private_named_instance_type_on_next_line() {
        // Type is on the next line — first line only has binders and a trailing ':'
        let h = parse_lean_decl("private instance pedersen_finite (h : G) :").unwrap();
        assert_eq!(h.keyword, "instance");
        assert_eq!(h.name, Some("pedersen_finite".to_string()));
        assert_eq!(h.modifiers, vec!["private"]);
        assert_eq!(h.result_type, None);
    }

    #[test]
    fn test_named_instance_type_on_first_line() {
        let h = parse_lean_decl("instance myInst : Fintype (ZMod n) :=").unwrap();
        assert_eq!(h.name, Some("myInst".to_string()));
        assert_eq!(h.result_type, Some("Fintype (ZMod n)".to_string()));
    }

    #[test]
    fn test_instance_type_with_binders_stops_at_where() {
        let h = parse_lean_decl("instance [Fintype α] : Fintype (Option α) where").unwrap();
        assert_eq!(h.name, None);
        assert_eq!(h.result_type, Some("Fintype (Option α)".to_string()));
    }

    #[test]
    fn test_noncomputable_def() {
        let h = parse_lean_decl("noncomputable def myFun : T").unwrap();
        assert_eq!(h.keyword, "def");
        assert_eq!(h.name, Some("myFun".to_string()));
        assert_eq!(h.modifiers, vec!["noncomputable"]);
    }

    #[test]
    fn test_multiple_modifiers() {
        let h = parse_lean_decl("private noncomputable def secret : T").unwrap();
        assert_eq!(h.keyword, "def");
        assert_eq!(h.name, Some("secret".to_string()));
        assert_eq!(h.modifiers, vec!["private", "noncomputable"]);
    }

    #[test]
    fn test_attribute_and_theorem() {
        let h = parse_lean_decl("@[simp, norm_num] theorem foo : True").unwrap();
        assert_eq!(h.keyword, "theorem");
        assert_eq!(h.name, Some("foo".to_string()));
        assert_eq!(h.attributes, vec!["simp, norm_num"]);
    }

    #[test]
    fn test_multiple_attributes() {
        let h = parse_lean_decl("@[simp] @[ext] def bar : T").unwrap();
        assert_eq!(h.keyword, "def");
        assert_eq!(h.name, Some("bar".to_string()));
        assert_eq!(h.attributes, vec!["simp", "ext"]);
    }

    #[test]
    fn test_set_option_before_doc_comment() {
        let input = "set_option maxHeartbeats 400000 in\n/-- Big proof. -/\ntheorem bigProof : T";
        let h = parse_lean_decl(input).unwrap();
        assert_eq!(h.keyword, "theorem");
        assert_eq!(h.name, Some("bigProof".to_string()));
        assert_eq!(h.doc_comment, Some("Big proof.".to_string()));
    }

    #[test]
    fn test_set_option_before_theorem_no_doc() {
        let input = "set_option maxHeartbeats 400000 in\ntheorem bigProof : T";
        let h = parse_lean_decl(input).unwrap();
        assert_eq!(h.keyword, "theorem");
        assert_eq!(h.name, Some("bigProof".to_string()));
    }

    #[test]
    fn test_structure_with_doc() {
        let input = "/-- A sigma protocol (3-move interactive proof). -/\n    structure SigmaProtocol where";
        let h = parse_lean_decl(input).unwrap();
        assert_eq!(h.keyword, "structure");
        assert_eq!(h.name, Some("SigmaProtocol".to_string()));
        assert!(h.doc_comment.is_some());
    }

    #[test]
    fn test_def_with_doc_and_qualified_name() {
        let input = "/-- The Pedersen commitment scheme: commit(m, r) = g^m * h^r. -/\n    def PedersenCommitment (h : G) : CommitmentScheme";
        let h = parse_lean_decl(input).unwrap();
        assert_eq!(h.keyword, "def");
        assert_eq!(h.name, Some("PedersenCommitment".to_string()));
    }

    #[test]
    fn test_qualified_name() {
        let h = parse_lean_decl("def Foo.bar : Nat").unwrap();
        assert_eq!(h.name, Some("Foo.bar".to_string()));
    }

    #[test]
    fn test_multiline_doc_comment() {
        let input = "/-- This is a multi-line\n    doc comment. -/\ndef SchnorrProtocol : SigmaProtocol where";
        let h = parse_lean_decl(input).unwrap();
        assert_eq!(h.keyword, "def");
        assert_eq!(h.name, Some("SchnorrProtocol".to_string()));
        assert!(h.doc_comment.unwrap().contains("multi-line"));
    }

    #[test]
    fn test_not_a_decl_returns_none() {
        assert!(parse_lean_decl("Lean.Elab.Command.runLintersAsync").is_none());
        assert!(parse_lean_decl("running linters").is_none());
        assert!(parse_lean_decl("elaborating proof of foo").is_none());
        assert!(parse_lean_decl("expected type: Sort ?u.42, term").is_none());
    }

    #[test]
    fn test_def_not_matched_as_prefix() {
        // "definition" must not match as keyword "def"
        assert!(parse_lean_decl("definition foo").is_none());
    }

    #[test]
    fn test_instance_keyword_only() {
        // A bare `instance` at end of input (no type, no name) should still parse
        let h = parse_lean_decl("instance").unwrap();
        assert_eq!(h.keyword, "instance");
        assert_eq!(h.name, None);
        assert_eq!(h.result_type, None);
    }
}
