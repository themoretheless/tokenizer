//! Parser-aware semantic tokens for syntax highlighting.

use crate::Span;

use super::{Parse, ParseDiagnostic, ParseOptions, SyntaxKind, parse, parse_with};

/// A high-level highlighting category derived from syntax and parser context.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SemanticKind {
    Property,
    String,
    Number,
    Boolean,
    Null,
    Punctuation,
    Comment,
    Whitespace,
    Invalid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct SemanticToken {
    pub kind: SemanticKind,
    pub span: Span,
}

impl SemanticToken {
    #[must_use]
    pub fn text(self, source: &str) -> Option<&str> {
        self.span.slice(source)
    }
}

/// Parser-aware, lossless highlighting output.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct SemanticTokenization {
    pub tokens: Vec<SemanticToken>,
    pub diagnostics: Vec<ParseDiagnostic>,
}

impl SemanticTokenization {
    #[must_use]
    pub fn tokens(&self) -> &[SemanticToken] {
        &self.tokens
    }

    #[must_use]
    pub fn diagnostics(&self) -> &[ParseDiagnostic] {
        &self.diagnostics
    }

    #[must_use]
    pub fn has_errors(&self) -> bool {
        !self.diagnostics.is_empty()
    }

    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.diagnostics.is_empty()
    }
}

/// Tokenizes strict JSON using structural parser context.
#[must_use]
pub fn tokenize(source: &str) -> SemanticTokenization {
    semantic_tokens(&parse(source))
}

/// Tokenizes JSON/JSONC using structural parser context and explicit limits.
#[must_use]
pub fn tokenize_with(source: &str, options: ParseOptions) -> SemanticTokenization {
    semantic_tokens(&parse_with(source, options))
}

/// Builds semantic tokens from an existing parse without reparsing.
#[must_use]
pub fn semantic_tokens(parsed: &Parse<'_>) -> SemanticTokenization {
    let property_spans = parsed.property_spans();
    let mut property_cursor = 0;
    let invalid_value_spans = parsed.invalid_value_spans();
    let mut invalid_value_cursor = 0;
    let lexical_diagnostics = parsed.lexed().diagnostics();
    let mut diagnostic_cursor = 0;
    let mut tokens = Vec::with_capacity(parsed.lexed().tokens().len());
    for token in parsed.lexed().tokens() {
        while property_spans
            .get(property_cursor)
            .is_some_and(|span| span.end <= token.span.start)
        {
            property_cursor += 1;
        }
        while invalid_value_spans
            .get(invalid_value_cursor)
            .is_some_and(|span| span.end <= token.span.start)
        {
            invalid_value_cursor += 1;
        }
        while lexical_diagnostics
            .get(diagnostic_cursor)
            .is_some_and(|diagnostic| diagnostic.span.end <= token.span.start)
        {
            diagnostic_cursor += 1;
        }
        let lexically_invalid = token.has_error()
            || lexical_diagnostics[diagnostic_cursor..]
                .iter()
                .take_while(|diagnostic| diagnostic.span.start < token.span.end)
                .any(|diagnostic| {
                    diagnostic.span.start < token.span.end
                        && token.span.start < diagnostic.span.end
                        && !matches!(
                            token.kind,
                            SyntaxKind::LineComment | SyntaxKind::BlockComment | SyntaxKind::Bom
                        )
                });
        let decoded_string_invalid =
            invalid_value_spans.get(invalid_value_cursor).copied() == Some(token.span);
        let kind = if lexically_invalid || decoded_string_invalid {
            SemanticKind::Invalid
        } else {
            match token.kind {
                SyntaxKind::Whitespace | SyntaxKind::Bom => SemanticKind::Whitespace,
                SyntaxKind::LineComment | SyntaxKind::BlockComment => SemanticKind::Comment,
                kind if kind.is_punctuation() => SemanticKind::Punctuation,
                SyntaxKind::String
                    if property_spans.get(property_cursor).copied() == Some(token.span) =>
                {
                    SemanticKind::Property
                }
                SyntaxKind::String => SemanticKind::String,
                SyntaxKind::Number => SemanticKind::Number,
                SyntaxKind::True | SyntaxKind::False => SemanticKind::Boolean,
                SyntaxKind::Null => SemanticKind::Null,
                SyntaxKind::Error => SemanticKind::Invalid,
                _ => SemanticKind::Invalid,
            }
        };
        tokens.push(SemanticToken {
            kind,
            span: token.span,
        });
    }

    SemanticTokenization {
        tokens,
        diagnostics: parsed.diagnostics().to_vec(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn significant(source: &str) -> Vec<(SemanticKind, &str)> {
        tokenize(source)
            .tokens
            .into_iter()
            .filter(|token| {
                !matches!(
                    token.kind,
                    SemanticKind::Whitespace | SemanticKind::Punctuation
                )
            })
            .map(|token| (token.kind, token.text(source).unwrap()))
            .collect()
    }

    #[test]
    fn properties_come_from_object_structure_not_colon_lookahead() {
        assert_eq!(
            significant(r#"{"x":1}"#),
            vec![
                (SemanticKind::Property, "\"x\""),
                (SemanticKind::Number, "1")
            ]
        );
        assert_eq!(
            significant(r#"["x":1]"#),
            vec![(SemanticKind::String, "\"x\""), (SemanticKind::Number, "1")]
        );
    }

    #[test]
    fn semantic_tokens_are_lossless_and_include_comments() {
        let source = "{/* hello */\"ok\":true}";
        let result = tokenize_with(source, ParseOptions::jsonc());
        let rebuilt: String = result
            .tokens
            .iter()
            .map(|token| token.text(source).unwrap())
            .collect();
        assert_eq!(rebuilt, source);
        assert!(
            result
                .tokens
                .iter()
                .any(|token| token.kind == SemanticKind::Comment)
        );
    }

    #[test]
    fn incomplete_object_keys_still_receive_property_highlighting() {
        assert_eq!(
            significant(r#"{"x":}"#),
            vec![(SemanticKind::Property, "\"x\"")]
        );
    }

    #[test]
    fn recovery_does_not_promote_a_bad_keys_value_to_property() {
        assert_eq!(
            significant(r#"{wat:"v","x":1}"#),
            vec![
                (SemanticKind::Invalid, "wat"),
                (SemanticKind::String, "\"v\""),
                (SemanticKind::Property, "\"x\""),
                (SemanticKind::Number, "1"),
            ]
        );
    }

    #[test]
    fn extra_root_objects_still_have_contextual_highlighting() {
        let result = significant(r#"{"a":1}{"b":2}"#);
        assert!(result.contains(&(SemanticKind::Property, "\"a\"")));
        assert!(result.contains(&(SemanticKind::Property, "\"b\"")));
    }

    #[test]
    fn token_flags_survive_diagnostic_truncation() {
        let source = "[01,02,03]";
        let result = tokenize_with(source, ParseOptions::strict().max_diagnostics(1));
        let invalid_numbers = result
            .tokens
            .iter()
            .filter(|token| token.kind == SemanticKind::Invalid)
            .filter(|token| token.text(source).is_some_and(|text| text.starts_with('0')))
            .count();
        assert_eq!(invalid_numbers, 3);
    }

    #[test]
    fn invalid_unicode_surrogates_are_semantically_invalid() {
        let source = r#""\uD800""#;
        assert_eq!(significant(source), vec![(SemanticKind::Invalid, source)]);
        let truncated = tokenize_with(source, ParseOptions::strict().max_diagnostics(0));
        assert_eq!(truncated.tokens()[0].kind, SemanticKind::Invalid);
    }
}
