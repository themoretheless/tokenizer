//! A recovering structural pass over the CSS token stream.
//!
//! The pass reads the lexer's context-carrying tokens and reports CSS
//! diagnostics with stable kebab-case codes. It never consumes or drops bytes:
//! every diagnostic points at an existing token, so the token stream stays
//! lossless even when the stylesheet is badly broken.

use themoretheless_tokenizer_core::{Diagnostic, Span};

use crate::lexer::{LexToken, Lexed, SyntaxKind, lex};

/// A rule block the structural pass recognized, at any nesting depth.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rule<'source> {
    pub kind: RuleKind<'source>,
    /// Selector or at-rule prelude text, taken verbatim from the source.
    pub prelude: &'source str,
    /// The `{` of the block through its `}`, or through EOF when unclosed.
    pub block: Span,
    /// Zero for a top-level rule; one deeper for each nested rule.
    pub depth: usize,
    /// Property and custom-property names declared directly in this block.
    pub properties: Vec<&'source str>,
}

/// Which kind of `{ … }` block a [`Rule`] describes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuleKind<'source> {
    /// `selector { … }`, including a nested rule inside another block.
    Style,
    /// `@name prelude { … }`; the name keeps the source's spelling.
    At(&'source str),
}

/// Lossless tokens plus recovering CSS diagnostics and a light rule list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parse<'source> {
    source: &'source str,
    lexed: Lexed<'source>,
    diagnostics: Vec<Diagnostic>,
    rules: Vec<Rule<'source>>,
}

impl<'source> Parse<'source> {
    /// The exact source this parse came from.
    #[must_use]
    pub const fn source(&self) -> &'source str {
        self.source
    }

    /// The lossless token stream behind the diagnostics and rules.
    #[must_use]
    pub const fn lexed(&self) -> &Lexed<'source> {
        &self.lexed
    }

    /// Lexical and structural diagnostics, ordered by span start.
    #[must_use]
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    /// Rule blocks in source order, a parent before its nested rules.
    #[must_use]
    pub fn rules(&self) -> &[Rule<'source>] {
        &self.rules
    }

    /// Whether no diagnostic was raised and no token is error-flagged.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.diagnostics.is_empty() && !self.lexed.has_errors()
    }
}

/// Lexes then runs the structural pass.
#[must_use]
pub fn parse(source: &str) -> Parse<'_> {
    let lexed = lex(source);
    let (structural, rules) = analyze(&lexed);
    let mut diagnostics = Vec::with_capacity(lexed.diagnostics().len() + structural.len());
    diagnostics.extend_from_slice(lexed.diagnostics());
    diagnostics.extend(structural);
    diagnostics.sort_by_key(|diagnostic| (diagnostic.span.start, diagnostic.span.end));
    Parse {
        source,
        lexed,
        diagnostics,
        rules,
    }
}

/// Diagnostics from the lexer and the structural pass.
#[must_use]
pub fn validate(source: &str) -> Vec<Diagnostic> {
    parse(source).diagnostics
}

const EMPTY_DECLARATION: &str = "empty-declaration";

/// One open `{` and the rule text that preceded it.
struct OpenRule<'source> {
    open: LexToken,
    kind: RuleKind<'source>,
    prelude_start: usize,
    depth: usize,
    significant: usize,
    delimiters: usize,
    properties: Vec<&'source str>,
}

/// One unclosed `(` or `[`.
struct OpenDelimiter {
    token: LexToken,
    bracket: bool,
    function: bool,
}

/// The declaration or selector currently being read.
#[derive(Default)]
struct Item {
    count: usize,
    first: Option<LexToken>,
    property: Option<LexToken>,
    colon: Option<LexToken>,
    value_tokens: usize,
}

fn analyze<'source>(lexed: &Lexed<'source>) -> (Vec<Diagnostic>, Vec<Rule<'source>>) {
    let source = lexed.source();
    let mut diagnostics: Vec<Diagnostic> = Vec::new();
    let mut rules: Vec<Rule<'source>> = Vec::new();
    let mut blocks: Vec<OpenRule<'source>> = Vec::new();
    let mut delimiters: Vec<OpenDelimiter> = Vec::new();
    let mut item = Item::default();
    let mut item_start = 0usize;
    let mut previous: Option<LexToken> = None;

    for token in lexed.significant_tokens() {
        let continues_value = item.colon.is_some();
        // A `{` counts for the enclosing block; a `}` is not content of its own.
        if token.kind != SyntaxKind::RightBrace
            && let Some(block) = blocks.last_mut()
        {
            block.significant += 1;
        }
        item.count += 1;
        if item.count == 1 {
            item.first = Some(token);
        }
        match token.kind {
            SyntaxKind::LeftBrace => {
                if item.count == 1 {
                    push(
                        &mut diagnostics,
                        token.span,
                        "empty-selector",
                        "rule block has no selector",
                    );
                }
                let kind = match item.first {
                    Some(first) if first.kind == SyntaxKind::AtRule => {
                        RuleKind::At(at_rule_name(source, first.span))
                    }
                    _ => RuleKind::Style,
                };
                flush(&mut diagnostics, &item);
                blocks.push(OpenRule {
                    open: token,
                    kind,
                    prelude_start: item.first.map_or(item_start, |first| first.span.start),
                    depth: blocks.len(),
                    significant: 0,
                    delimiters: delimiters.len(),
                    properties: Vec::new(),
                });
                item = Item::default();
                item_start = token.span.end;
            }
            SyntaxKind::RightBrace => {
                flush(&mut diagnostics, &item);
                if let Some(block) = blocks.pop() {
                    if block.significant == 0 {
                        push(
                            &mut diagnostics,
                            Span::cover(block.open.span, token.span),
                            EMPTY_DECLARATION,
                            "block contains no declarations or nested rules",
                        );
                    }
                    rules.push(Rule {
                        kind: block.kind,
                        prelude: trimmed(source, block.prelude_start..block.open.span.start),
                        block: Span::new(block.open.span.start, token.span.end),
                        depth: block.depth,
                        properties: block.properties,
                    });
                } else {
                    push(
                        &mut diagnostics,
                        token.span,
                        "unexpected-close-brace",
                        "stray `}` with no open block",
                    );
                }
                reset_item(
                    &mut item,
                    &mut item_start,
                    token.span.end,
                    &mut delimiters,
                    &mut diagnostics,
                    &blocks,
                );
            }
            SyntaxKind::Semicolon => {
                if item.count == 1 {
                    push(
                        &mut diagnostics,
                        token.span,
                        EMPTY_DECLARATION,
                        "declaration has no property",
                    );
                } else {
                    flush(&mut diagnostics, &item);
                }
                reset_item(
                    &mut item,
                    &mut item_start,
                    token.span.end,
                    &mut delimiters,
                    &mut diagnostics,
                    &blocks,
                );
            }
            SyntaxKind::Property => {
                if item.colon.is_some() {
                    if let Some(previous) = previous {
                        push(
                            &mut diagnostics,
                            previous.span,
                            "missing-semicolon",
                            "declaration is not terminated before the next one",
                        );
                    }
                } else {
                    item.property = Some(token);
                }
                record_property(&mut blocks, source, token);
            }
            SyntaxKind::Variable => {
                if item.colon.is_none() {
                    item.property = Some(token);
                    record_property(&mut blocks, source, token);
                }
            }
            SyntaxKind::Colon => {
                if item.count == 1 {
                    push(
                        &mut diagnostics,
                        token.span,
                        EMPTY_DECLARATION,
                        "declaration has no property",
                    );
                } else if item.colon.is_none() {
                    item.colon = Some(token);
                }
            }
            SyntaxKind::LeftParen => {
                let function =
                    previous.is_some_and(|previous| previous.kind == SyntaxKind::Function);
                delimiters.push(OpenDelimiter {
                    token,
                    bracket: false,
                    function,
                });
            }
            SyntaxKind::LeftBracket => {
                delimiters.push(OpenDelimiter {
                    token,
                    bracket: true,
                    function: false,
                });
            }
            SyntaxKind::RightParen | SyntaxKind::RightBracket => {
                let matched = delimiters.pop().is_some();
                if !matched {
                    push(
                        &mut diagnostics,
                        token.span,
                        "unexpected-close-paren",
                        "stray closer with no open delimiter",
                    );
                }
            }
            _ => {}
        }
        if continues_value {
            item.value_tokens += 1;
        }
        previous = Some(token);
    }

    flush(&mut diagnostics, &item);
    close_delimiters(&mut diagnostics, &mut delimiters, 0);
    while let Some(block) = blocks.pop() {
        push(
            &mut diagnostics,
            block.open.span,
            "unclosed-block",
            "rule block is missing its closing `}`",
        );
        let end = source.len();
        rules.push(Rule {
            kind: block.kind,
            prelude: trimmed(source, block.prelude_start..block.open.span.start),
            block: Span::new(block.open.span.start, end),
            depth: block.depth,
            properties: block.properties,
        });
    }
    rules.sort_by_key(|rule| rule.block.start);
    (diagnostics, rules)
}

fn reset_item(
    item: &mut Item,
    item_start: &mut usize,
    boundary: usize,
    delimiters: &mut Vec<OpenDelimiter>,
    diagnostics: &mut Vec<Diagnostic>,
    blocks: &[OpenRule<'_>],
) {
    *item = Item::default();
    *item_start = boundary;
    close_delimiters(
        diagnostics,
        delimiters,
        blocks.last().map_or(0, |block| block.delimiters),
    );
}

fn close_delimiters(
    diagnostics: &mut Vec<Diagnostic>,
    delimiters: &mut Vec<OpenDelimiter>,
    keep: usize,
) {
    while delimiters.len() > keep {
        let Some(delimiter) = delimiters.pop() else {
            break;
        };
        let (code, message) = if delimiter.bracket {
            ("unclosed-bracket", "attribute selector is missing its `]`")
        } else if delimiter.function {
            ("unclosed-function", "function is missing its closing `)`")
        } else {
            ("unclosed-paren", "value is missing its closing `)`")
        };
        push(diagnostics, delimiter.token.span, code, message);
    }
}

fn flush(diagnostics: &mut Vec<Diagnostic>, item: &Item) {
    let Some(property) = item.property else {
        return;
    };
    match item.colon {
        None => push(
            diagnostics,
            property.span,
            "expected-colon",
            "property is missing its `:` separator",
        ),
        Some(colon) if item.value_tokens == 0 => push(
            diagnostics,
            colon.span,
            "expected-value",
            "declaration is missing a value",
        ),
        Some(_) => {}
    }
}

fn record_property<'source>(
    blocks: &mut [OpenRule<'source>],
    source: &'source str,
    token: LexToken,
) {
    if let Some(text) = source.get(token.span.range())
        && let Some(block) = blocks.last_mut()
    {
        block.properties.push(text);
    }
}

fn at_rule_name(source: &str, span: Span) -> &str {
    source
        .get(span.range())
        .and_then(|text| text.get(1..))
        .unwrap_or_default()
}

fn trimmed(source: &str, range: std::ops::Range<usize>) -> &str {
    source.get(range).map(str::trim).unwrap_or_default()
}

fn push(diagnostics: &mut Vec<Diagnostic>, span: Span, code: &'static str, message: &'static str) {
    diagnostics.push(Diagnostic::new(span, code, message));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn codes(source: &str) -> Vec<&'static str> {
        validate(source)
            .iter()
            .map(|diagnostic| diagnostic.code)
            .collect()
    }

    #[test]
    fn unclosed_block_is_reported_per_brace() {
        assert_eq!(codes("a { color: red;"), ["unclosed-block"]);
        assert_eq!(
            codes("a { b { color: red; "),
            ["unclosed-block", "unclosed-block"]
        );
    }

    #[test]
    fn stray_close_brace() {
        assert_eq!(codes("a { color: red; }}"), ["unexpected-close-brace"]);
    }

    #[test]
    fn unterminated_string_and_comment() {
        // The open string swallows the `}`, so the block really is unclosed too.
        assert_eq!(
            codes("a { content: \"oops }"),
            ["unclosed-block", "unclosed-string"]
        );
        assert_eq!(codes("/* never closed"), ["unclosed-comment"]);
    }

    #[test]
    fn missing_semicolon_between_declarations() {
        assert_eq!(
            codes("a { color: red background: blue; }"),
            ["missing-semicolon"]
        );
    }

    #[test]
    fn empty_declarations_and_selectors() {
        assert_eq!(codes("a { : ; }"), ["empty-declaration"]);
        assert_eq!(codes("a { }"), ["empty-declaration"]);
        assert_eq!(codes("{ color: red; }"), ["empty-selector"]);
    }

    #[test]
    fn expected_colon_and_value() {
        assert_eq!(codes("a { colorred; }"), ["expected-colon"]);
        assert_eq!(codes("a { color: }"), ["expected-value"]);
    }

    #[test]
    fn unclosed_delimiters() {
        assert_eq!(codes("a { width: rgb(0 0; }"), ["unclosed-function"]);
        assert_eq!(codes("a { width: (0 0; }"), ["unclosed-paren"]);
        assert_eq!(codes("a[href { color: red; }"), ["unclosed-bracket"]);
    }

    #[test]
    fn invalid_hex_color() {
        assert_eq!(codes("a { color: #gg; }"), ["invalid-hex-color"]);
        assert_eq!(codes("a { color: #12345; }"), ["invalid-hex-color"]);
    }

    #[test]
    fn statement_at_rule_needs_a_semicolon() {
        assert_eq!(
            codes("@import \"theme.css\"\nbody { color: red; }"),
            ["missing-semicolon"]
        );
        assert_eq!(codes("@import \"theme.css\";"), Vec::<&str>::new());
    }

    #[test]
    fn last_declaration_needs_no_semicolon() {
        assert_eq!(codes("a { color: red }"), Vec::<&str>::new());
    }

    #[test]
    fn rule_list_tracks_properties_and_depth() {
        let parsed = parse("a { color: red; & b { --x: 1px; } }");
        assert_eq!(parsed.diagnostics(), []);
        assert_eq!(parsed.rules().len(), 2);
        assert_eq!(parsed.rules()[0].properties, vec!["color"]);
        assert_eq!(parsed.rules()[0].prelude, "a");
        assert_eq!(parsed.rules()[1].depth, 1);
        assert_eq!(parsed.rules()[1].properties, vec!["--x"]);
        assert_eq!(parsed.rules()[1].prelude, "& b");
    }

    #[test]
    fn broken_input_stays_lossless() {
        for source in [
            "a {",
            "}}}",
            "a { color: \"red",
            "/*",
            "@media (",
            "@media { a { b: rgb(0 0",
            "a { color: #1; ; ; {",
            "\u{feff}$x { y: z",
        ] {
            let parsed = parse(source);
            assert!(!parsed.is_valid(), "{source:?}");
            assert!(parsed.lexed().verify_lossless().is_ok(), "{source:?}");
            assert_eq!(parsed.lexed().joined(), source, "{source:?}");
        }
    }
}
