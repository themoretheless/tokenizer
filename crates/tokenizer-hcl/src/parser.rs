//! Recovering HCL structure over the lossless token stream.
//!
//! The pass reads tokens only: every diagnostic points at bytes the lexer
//! already emitted, so recovery never consumes or drops input. Even the most
//! broken document keeps a lossless token stream while [`Parse::is_valid`]
//! reports `false`.
//!
//! Structure is deliberately shallow. The pass pairs brackets, matches
//! heredoc opens with their terminators, and — with a same-line lookahead —
//! decides which identifiers read as attribute names or block types and which
//! quoted strings read as block labels. Full expression parsing is out of
//! scope: HCL delimits items by newlines, and the lexer keeps newlines as
//! their own tokens, so a shallow structural pass is enough to classify
//! every token the semantic layer claims.

use std::collections::HashSet;
use std::fmt;

use themoretheless_tokenizer_core::{Diagnostic, DiagnosticKind as _, Span};

use crate::lexer::{Lexed, SyntaxKind, lex};

/// A stable kebab-case diagnostic code for HCL faults.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum DiagnosticKind {
    /// A `"` opened a string that never reached its closing quote.
    UnterminatedString,
    /// A `<<TAG` heredoc has no terminator line carrying its tag.
    UnterminatedHeredoc,
    /// A `${` template sequence is never closed by a matching `}`.
    UnterminatedInterpolation,
    /// A `/*` block comment is never closed by `*/`.
    UnterminatedComment,
    /// A backslash in a string does not start a known escape.
    InvalidEscape,
    /// A `{` is still open at end of input.
    UnclosedBrace,
    /// A `[` is still open at end of input.
    UnclosedBracket,
    /// A `(` is still open at end of input.
    UnclosedParen,
    /// A byte the grammar has no place for, or a closer with no opener.
    UnexpectedToken,
}

impl DiagnosticKind {
    /// Stable wire code.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::UnterminatedString => "unterminated-string",
            Self::UnterminatedHeredoc => "unterminated-heredoc",
            Self::UnterminatedInterpolation => "unterminated-interpolation",
            Self::UnterminatedComment => "unterminated-comment",
            Self::InvalidEscape => "invalid-escape",
            Self::UnclosedBrace => "unclosed-brace",
            Self::UnclosedBracket => "unclosed-bracket",
            Self::UnclosedParen => "unclosed-paren",
            Self::UnexpectedToken => "unexpected-token",
        }
    }

    /// Human-readable one-liner.
    #[must_use]
    pub const fn message(self) -> &'static str {
        match self {
            Self::UnterminatedString => "string is never closed",
            Self::UnterminatedHeredoc => "heredoc has no terminator line",
            Self::UnterminatedInterpolation => "interpolation is missing its closing brace",
            Self::UnterminatedComment => "block comment is never closed",
            Self::InvalidEscape => "unknown escape sequence",
            Self::UnclosedBrace => "brace is never closed",
            Self::UnclosedBracket => "bracket is never closed",
            Self::UnclosedParen => "parenthesis is never closed",
            Self::UnexpectedToken => "token has no place in the grammar",
        }
    }
}

impl themoretheless_tokenizer_core::DiagnosticKind for DiagnosticKind {
    fn code(self) -> &'static str {
        DiagnosticKind::code(self)
    }

    fn message(self) -> &'static str {
        DiagnosticKind::message(self)
    }
}

impl fmt::Display for DiagnosticKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.message())
    }
}

/// Shallow HCL structure: diagnostics plus the role readings the semantic
/// layer needs. Role sets hold the byte offset at which a token starts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parse<'source> {
    lexed: Lexed<'source>,
    diagnostics: Vec<Diagnostic>,
    attribute_names: HashSet<usize>,
    block_types: HashSet<usize>,
    block_labels: HashSet<usize>,
}

impl<'source> Parse<'source> {
    #[must_use]
    pub fn lexed(&self) -> &Lexed<'source> {
        &self.lexed
    }

    #[must_use]
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    /// No diagnostics at all: the document parsed clean.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.diagnostics.is_empty()
    }

    /// Whether the token at `span` reads as the name side of `name = value`.
    #[must_use]
    pub fn is_attribute_name(&self, span: Span) -> bool {
        self.attribute_names.contains(&span.start)
    }

    /// Whether the token at `span` names a block introduced on the same line.
    #[must_use]
    pub fn is_block_type(&self, span: Span) -> bool {
        self.block_types.contains(&span.start)
    }

    /// Whether the token at `span` is part of a quoted label between a block
    /// type and its `{`.
    #[must_use]
    pub fn is_block_label(&self, span: Span) -> bool {
        self.block_labels.contains(&span.start)
    }
}

/// Parse an HCL document: lex losslessly, then run the recovering
/// structural pass.
#[must_use]
pub fn parse(source: &str) -> Parse<'_> {
    let lexed = lex(source);
    let tokens = lexed.tokens();
    let source_text = lexed.source();
    let mut diagnostics: Vec<Diagnostic> = Vec::new();
    let mut attribute_names = HashSet::new();
    let mut block_types = HashSet::new();
    let mut block_labels = HashSet::new();

    // Faults the lexer already flagged, one diagnostic per flagged span.
    // Heredoc faults are handled structurally below instead, so a heredoc
    // with no body bytes at all still gets exactly one report.
    for token in tokens {
        if !token.has_error() {
            continue;
        }
        let kind = match token.kind {
            SyntaxKind::String => DiagnosticKind::UnterminatedString,
            SyntaxKind::Escape => DiagnosticKind::InvalidEscape,
            SyntaxKind::Interpolation => DiagnosticKind::UnterminatedInterpolation,
            SyntaxKind::BlockComment => DiagnosticKind::UnterminatedComment,
            SyntaxKind::HeredocBody | SyntaxKind::HeredocOpen => continue,
            _ => DiagnosticKind::UnexpectedToken,
        };
        diagnostics.push(kind.to_diagnostic(token.span));
    }

    // Heredoc pairing: opens and closes are strictly sequential because the
    // body scan swallows everything up to the terminator. An unmatched open
    // is the unterminated heredoc, even with no body bytes at all.
    let mut open_heredocs: Vec<Span> = Vec::new();
    for token in tokens {
        match token.kind {
            SyntaxKind::HeredocOpen => open_heredocs.push(token.span),
            SyntaxKind::HeredocClose => {
                let _ = open_heredocs.pop();
            }
            _ => {}
        }
    }
    for span in open_heredocs {
        diagnostics.push(DiagnosticKind::UnterminatedHeredoc.to_diagnostic(span));
    }

    // Bracket balance over punctuation.
    let mut open_stack: Vec<Span> = Vec::new();
    for token in tokens {
        if token.kind != SyntaxKind::Punctuation {
            continue;
        }
        let Some(text) = token.text(source_text) else {
            continue;
        };
        match text {
            "{" | "[" | "(" => open_stack.push(token.span),
            "}" | "]" | ")" => {
                let expected = closer_opener(text);
                let matched =
                    open_stack.last().and_then(|span| span.slice(source_text)) == Some(expected);
                if matched {
                    let _ = open_stack.pop();
                } else if open_stack.is_empty() {
                    // A closer with nothing open: the token has no place here.
                    diagnostics.push(DiagnosticKind::UnexpectedToken.to_diagnostic(token.span));
                } else {
                    // Wrong closer for the innermost opener: fault on both.
                    let opener = open_stack.pop().unwrap_or(token.span);
                    diagnostics
                        .push(unclosed_kind(opener.slice(source_text)).to_diagnostic(opener));
                    diagnostics.push(DiagnosticKind::UnexpectedToken.to_diagnostic(token.span));
                }
            }
            _ => {}
        }
    }
    for span in open_stack {
        diagnostics.push(unclosed_kind(span.slice(source_text)).to_diagnostic(span));
    }

    // Roles: an identifier is an attribute name when the rest of its line
    // starts with ` = `, and a block type when quoted labels and a `{`
    // follow it before the line ends.
    for (index, token) in tokens.iter().enumerate() {
        if token.kind != SyntaxKind::Identifier || token.has_error() {
            continue;
        }
        let mut labels: Vec<usize> = Vec::new();
        let mut saw_brace = false;
        let mut saw_equals = false;
        let mut cursor = index + 1;
        while cursor < tokens.len() {
            let later = tokens[cursor];
            match later.kind {
                SyntaxKind::Whitespace | SyntaxKind::LineComment | SyntaxKind::BlockComment => {}
                SyntaxKind::Newline => break,
                SyntaxKind::Operator if later.text(source_text) == Some("=") => {
                    saw_equals = true;
                    break;
                }
                SyntaxKind::String | SyntaxKind::Escape | SyntaxKind::Interpolation => {
                    labels.push(later.span.start);
                }
                SyntaxKind::Punctuation if later.text(source_text) == Some("{") => {
                    saw_brace = true;
                    break;
                }
                _ => break,
            }
            cursor += 1;
        }
        if saw_equals {
            attribute_names.insert(token.span.start);
        } else if saw_brace {
            block_types.insert(token.span.start);
            block_labels.extend(labels);
        }
    }

    diagnostics.sort_by_key(|diagnostic| (diagnostic.span.start, diagnostic.code));

    Parse {
        lexed,
        diagnostics,
        attribute_names,
        block_types,
        block_labels,
    }
}

/// Diagnostics only: the same pass, projected onto its fault list.
#[must_use]
pub fn validate(source: &str) -> Vec<Diagnostic> {
    parse(source).diagnostics().to_vec()
}

fn closer_opener(closer: &str) -> &'static str {
    match closer {
        "}" => "{",
        "]" => "[",
        _ => "(",
    }
}

fn unclosed_kind(opener: Option<&str>) -> DiagnosticKind {
    match opener {
        Some("[") => DiagnosticKind::UnclosedBracket,
        Some("(") => DiagnosticKind::UnclosedParen,
        _ => DiagnosticKind::UnclosedBrace,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::LexToken;

    fn codes(source: &str) -> Vec<&'static str> {
        validate(source).iter().map(|d| d.code).collect()
    }

    fn flagged(tokens: &[LexToken]) -> Vec<SyntaxKind> {
        tokens
            .iter()
            .filter(|token| token.has_error())
            .map(|token| token.kind)
            .collect()
    }

    fn find(parsed: &Parse<'_>, text: &str) -> LexToken {
        parsed
            .lexed()
            .tokens()
            .iter()
            .copied()
            .find(|token| token.text(parsed.lexed().source()) == Some(text))
            .unwrap_or_else(|| panic!("no token {text:?}"))
    }

    #[test]
    fn clean_documents_produce_no_diagnostics() {
        for source in [
            "a = 1\n",
            "block {\n  x = true\n}\n",
            "block \"label\" {\n  x = \"v\"\n}\n",
            "a = [1, 2, 3]\nb = { c = 1 }\n",
            "",
            "# only a comment\n",
            "a = \"x ${y} z\"\n",
            "b = <<EOT\ntext\nEOT\n",
            "a = b && !c\n",
            "x = y == z ? 1 : 2\n",
        ] {
            assert_eq!(validate(source), Vec::<Diagnostic>::new(), "{source:?}");
        }
    }

    #[test]
    fn unterminated_string_is_reported() {
        assert_eq!(codes("a = \"oops"), ["unterminated-string"]);
        assert_eq!(codes("a = \"oops\nb = 1"), ["unterminated-string"]);
    }

    #[test]
    fn unterminated_heredoc_is_reported() {
        assert_eq!(codes("b = <<EOT\nno end here\n"), ["unterminated-heredoc"]);
        // Even with no body bytes at all, the open itself carries the fault.
        assert_eq!(codes("b = <<EOT"), ["unterminated-heredoc"]);
    }

    #[test]
    fn unterminated_interpolation_is_reported() {
        assert_eq!(codes("a = \"${x"), ["unterminated-interpolation"]);
        assert_eq!(
            codes("b = <<EOT\nt ${x\nEOT\n"),
            ["unterminated-interpolation"]
        );
    }

    #[test]
    fn unterminated_comment_is_reported() {
        assert_eq!(codes("/* forever"), ["unterminated-comment"]);
    }

    #[test]
    fn invalid_escape_is_reported() {
        assert_eq!(codes("a = \"\\q\""), ["invalid-escape"]);
        assert_eq!(codes("a = \"\\u12\""), ["invalid-escape"]);
    }

    #[test]
    fn unclosed_brackets_are_reported() {
        assert_eq!(codes("a = { b = 1"), ["unclosed-brace"]);
        assert_eq!(codes("a = [1, 2"), ["unclosed-bracket"]);
        assert_eq!(codes("a = f(1"), ["unclosed-paren"]);
    }

    #[test]
    fn mismatched_closers_are_reported() {
        assert_eq!(codes("a = [1 }"), ["unclosed-bracket", "unexpected-token"]);
        assert_eq!(codes("a = 1 }"), ["unexpected-token"]);
    }

    #[test]
    fn stray_bytes_are_unexpected_tokens() {
        assert_eq!(codes("a = @ 1"), ["unexpected-token"]);
        assert_eq!(codes("a = $ b"), ["unexpected-token"]);
        assert_eq!(codes("b = <<\n"), ["unexpected-token"]);
    }

    #[test]
    fn roles_separate_attributes_blocks_and_labels() {
        let parsed = parse("service \"api\" \"eu\" {\n  port = 8080\n}\nlabel = 1\n");
        assert!(parsed.is_valid(), "{:?}", parsed.diagnostics());
        assert!(parsed.is_block_type(find(&parsed, "service").span));
        assert!(parsed.is_block_label(find(&parsed, "\"api\"").span));
        assert!(parsed.is_block_label(find(&parsed, "\"eu\"").span));
        assert!(parsed.is_attribute_name(find(&parsed, "port").span));
        assert!(parsed.is_attribute_name(find(&parsed, "label").span));
    }

    #[test]
    fn object_attributes_inside_expressions_are_names_too() {
        let parsed = parse("a = { b = 1 }\n");
        assert!(parsed.is_valid());
        assert!(parsed.is_attribute_name(find(&parsed, "b").span));
        assert!(!parsed.is_block_type(find(&parsed, "a").span));
    }

    #[test]
    fn equality_and_ternary_do_not_create_attribute_names() {
        let parsed = parse("c = a == b\nx = a ? b : c\n");
        assert!(parsed.is_valid(), "{:?}", parsed.diagnostics());
        let operators: Vec<&str> = parsed
            .lexed()
            .tokens()
            .iter()
            .filter(|t| t.kind == SyntaxKind::Operator)
            .map(|t| t.text(parsed.lexed().source()).unwrap())
            .collect();
        assert_eq!(operators, vec!["=", "==", "=", "?", ":"]);
        let src = parsed.lexed().source();
        let second_a = parsed
            .lexed()
            .tokens()
            .iter()
            .filter(|t| t.text(src) == Some("a"))
            .nth(1)
            .copied()
            .expect("second a");
        assert!(
            !parsed.is_attribute_name(second_a.span),
            "a after ? is a value"
        );
    }

    #[test]
    fn block_head_must_stay_on_one_line() {
        let parsed = parse("foo\n{\n}\n");
        assert!(parsed.is_valid());
        assert!(!parsed.is_block_type(find(&parsed, "foo").span));
    }

    #[test]
    fn flags_survive_on_error_tokens() {
        assert_eq!(
            flagged(lex("a = \"oops").tokens()),
            vec![SyntaxKind::String]
        );
        assert_eq!(
            flagged(lex("/* x").tokens()),
            vec![SyntaxKind::BlockComment]
        );
    }
}
