//! Recovering Markdown structural validation over the lossless token stream.
//!
//! The pass reads tokens only: a diagnostic points at bytes the lexer already
//! emitted, so recovery never consumes or drops input. Even the most broken
//! document keeps a lossless token stream while [`Parse::is_valid`] reports
//! `false`.

use std::fmt;

use themoretheless_tokenizer_core::{Diagnostic, DiagnosticKind as _, Span};

use crate::lexer::{LexToken, Lexed, SyntaxKind, lex};

/// A Markdown structural violation with a stable kebab-case code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum DiagnosticKind {
    UnclosedCodeFence,
    UnclosedEmphasis,
    UnclosedCodeSpan,
    UnclosedHtmlTag,
    UnclosedLink,
    InvalidHeadingLevel,
    EmptyHeading,
    TableColumnMismatch,
    UndefinedFootnoteReference,
    DuplicateLinkDefinition,
}

impl DiagnosticKind {
    /// Stable wire code.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::UnclosedCodeFence => "unclosed-code-fence",
            Self::UnclosedEmphasis => "unclosed-emphasis",
            Self::UnclosedCodeSpan => "unclosed-code-span",
            Self::UnclosedHtmlTag => "unclosed-html-tag",
            Self::UnclosedLink => "unclosed-link",
            Self::InvalidHeadingLevel => "invalid-heading-level",
            Self::EmptyHeading => "empty-heading",
            Self::TableColumnMismatch => "table-column-mismatch",
            Self::UndefinedFootnoteReference => "undefined-footnote-reference",
            Self::DuplicateLinkDefinition => "duplicate-link-definition",
        }
    }

    /// Human-readable one-liner.
    #[must_use]
    pub const fn message(self) -> &'static str {
        match self {
            Self::UnclosedCodeFence => "code fence is never closed",
            Self::UnclosedEmphasis => "emphasis marker is never closed",
            Self::UnclosedCodeSpan => "code span is never closed",
            Self::UnclosedHtmlTag => "raw HTML tag is never closed",
            Self::UnclosedLink => "link destination is never closed",
            Self::InvalidHeadingLevel => "heading level must be between 1 and 6",
            Self::EmptyHeading => "heading has no text",
            Self::TableColumnMismatch => "table row has a different column count",
            Self::UndefinedFootnoteReference => "footnote reference has no definition",
            Self::DuplicateLinkDefinition => "link definition label is reused",
        }
    }

    /// Diagnostic for an abandonement the lexer already tokenized.
    ///
    /// Flagged tokens keep their construct's kind, which is how an unclosed
    /// fence is told apart from an unclosed code span without re-scanning.
    fn from_flagged_token(kind: SyntaxKind) -> Option<Self> {
        Some(match kind {
            SyntaxKind::CodeFenceMarker => Self::UnclosedCodeFence,
            SyntaxKind::Strong | SyntaxKind::Emphasis | SyntaxKind::Strikethrough => {
                Self::UnclosedEmphasis
            }
            SyntaxKind::CodeSpan => Self::UnclosedCodeSpan,
            SyntaxKind::HtmlInline | SyntaxKind::HtmlBlock => Self::UnclosedHtmlTag,
            SyntaxKind::LinkText | SyntaxKind::LinkDestination => Self::UnclosedLink,
            _ => return None,
        })
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

/// A block-level construct, merged across its consecutive lines.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Block {
    pub kind: BlockKind,
    pub span: Span,
}

/// Block categories the structural pass can name without a tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum BlockKind {
    Heading,
    Paragraph,
    Quote,
    List,
    CodeBlock,
    Table,
    ThematicBreak,
    FrontMatter,
    Html,
}

impl BlockKind {
    /// Kebab-case name for host renderers.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Heading => "heading",
            Self::Paragraph => "paragraph",
            Self::Quote => "quote",
            Self::List => "list",
            Self::CodeBlock => "code-block",
            Self::Table => "table",
            Self::ThematicBreak => "thematic-break",
            Self::FrontMatter => "front-matter",
            Self::Html => "html",
        }
    }
}

/// Lossless tokens plus Markdown diagnostics and a light block list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parse<'source> {
    lexed: Lexed<'source>,
    diagnostics: Vec<Diagnostic>,
    blocks: Vec<Block>,
}

impl<'source> Parse<'source> {
    #[must_use]
    pub const fn lexed(&self) -> &Lexed<'source> {
        &self.lexed
    }

    #[must_use]
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    #[must_use]
    pub fn blocks(&self) -> &[Block] {
        &self.blocks
    }

    /// Whether the document is structurally clean. An error-flagged token is
    /// always paired with a diagnostic, so both must be empty.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.diagnostics.is_empty() && !self.lexed.has_errors()
    }

    #[must_use]
    pub fn into_diagnostics(self) -> Vec<Diagnostic> {
        self.diagnostics
    }
}

/// Runs the lossless lexer and the recovering structural pass.
#[must_use]
pub fn parse(source: &str) -> Parse<'_> {
    let lexed = lex(source);
    let lines = read_lines(&lexed);
    let mut diagnostics = Vec::new();
    flagged_diagnostics(&lexed, &mut diagnostics);
    heading_diagnostics(&lexed, &lines, &mut diagnostics);
    table_diagnostics(&lines, &mut diagnostics);
    label_diagnostics(&lexed, &mut diagnostics);
    diagnostics.sort_by_key(|diagnostic| diagnostic.span.start);
    Parse {
        lexed,
        diagnostics,
        blocks: blocks(&lines),
    }
}

/// Structural diagnostics only.
#[must_use]
pub fn validate(source: &str) -> Vec<Diagnostic> {
    parse(source).into_diagnostics()
}

/// One line as the token stream exposes it. `None` kind means blank.
#[derive(Debug, Clone, Copy)]
struct LineInfo<'source> {
    start: usize,
    content_end: usize,
    first_start: usize,
    first_end: usize,
    first_kind: SyntaxKind,
    kind: Option<BlockKind>,
    text: &'source str,
}

impl LineInfo<'_> {
    #[must_use]
    const fn span(&self) -> Span {
        Span::new(self.start, self.content_end)
    }
}

/// Splits the token stream into lines at their `LineBreak` tokens.
fn read_lines<'source>(lexed: &Lexed<'source>) -> Vec<LineInfo<'source>> {
    let tokens = lexed.tokens();
    let source = lexed.source();
    let mut lines = Vec::new();
    let mut start = 0usize;
    let mut first = 0usize;
    for (index, token) in tokens.iter().enumerate() {
        if token.kind != SyntaxKind::LineBreak {
            continue;
        }
        lines.push(line_info(
            tokens,
            first..index + 1,
            start,
            token.span.start,
            source,
        ));
        start = token.span.end;
        first = index + 1;
    }
    if first < tokens.len() {
        let content_end = tokens.last().map_or(start, |token| token.span.end);
        lines.push(line_info(
            tokens,
            first..tokens.len(),
            start,
            content_end,
            source,
        ));
    }
    lines
}

fn line_info<'source>(
    tokens: &[LexToken],
    range: std::ops::Range<usize>,
    start: usize,
    content_end: usize,
    source: &'source str,
) -> LineInfo<'source> {
    let region = tokens.get(range).unwrap_or_default();
    let leader = region.iter().find(|token| !token.kind.is_trivia()).copied();
    LineInfo {
        start,
        content_end,
        first_start: leader.map_or(content_end, |token| token.span.start),
        first_end: leader.map_or(content_end, |token| token.span.end),
        first_kind: leader.map_or(SyntaxKind::LineBreak, |token| token.kind),
        kind: classify(leader.map_or(SyntaxKind::LineBreak, |token| token.kind)),
        text: source.get(start..content_end).unwrap_or(source),
    }
}

/// Block category of a line, from the kind that decides its leading context.
#[must_use]
const fn classify(kind: SyntaxKind) -> Option<BlockKind> {
    Some(match kind {
        SyntaxKind::HeadingMarker | SyntaxKind::SetextUnderline => BlockKind::Heading,
        SyntaxKind::ThematicBreak => BlockKind::ThematicBreak,
        SyntaxKind::BlockquoteMarker => BlockKind::Quote,
        SyntaxKind::ListMarker => BlockKind::List,
        SyntaxKind::CodeFenceMarker | SyntaxKind::CodeBlockLine => BlockKind::CodeBlock,
        SyntaxKind::TablePipe | SyntaxKind::TableDelimiter | SyntaxKind::TableCell => {
            BlockKind::Table
        }
        SyntaxKind::FrontMatter | SyntaxKind::FrontMatterDelimiter => BlockKind::FrontMatter,
        SyntaxKind::HtmlBlock => BlockKind::Html,
        SyntaxKind::Whitespace | SyntaxKind::LineBreak => return None,
        _ => BlockKind::Paragraph,
    })
}

fn blocks(lines: &[LineInfo<'_>]) -> Vec<Block> {
    let mut output = Vec::new();
    let mut current: Option<Block> = None;
    for line in lines {
        let Some(kind) = line.kind else {
            output.extend(current.take());
            continue;
        };
        match current {
            Some(block) if block.kind == kind => {
                current = Some(Block {
                    kind,
                    span: Span::new(block.span.start, line.content_end),
                });
            }
            block => {
                output.extend(block);
                current = Some(Block {
                    kind,
                    span: line.span(),
                });
            }
        }
    }
    output.extend(current);
    output
}

fn flagged_diagnostics(lexed: &Lexed<'_>, output: &mut Vec<Diagnostic>) {
    for token in lexed.tokens() {
        if !token.has_error() {
            continue;
        }
        if let Some(kind) = DiagnosticKind::from_flagged_token(token.kind) {
            output.push(kind.to_diagnostic(token.span));
        }
    }
}

fn heading_diagnostics(lexed: &Lexed<'_>, lines: &[LineInfo<'_>], output: &mut Vec<Diagnostic>) {
    for token in lexed.tokens() {
        if token.kind == SyntaxKind::HeadingMarker && token.span.len() >= 7 {
            output.push(DiagnosticKind::InvalidHeadingLevel.to_diagnostic(token.span));
        }
    }
    for line in lines {
        if line.kind != Some(BlockKind::Heading) || line.first_kind != SyntaxKind::HeadingMarker {
            continue;
        }
        let rest = line.text.get(line.first_end - line.start..).unwrap_or("");
        if rest.trim_end().is_empty() {
            let span = Span::new(line.first_start, line.first_end);
            output.push(DiagnosticKind::EmptyHeading.to_diagnostic(span));
        }
    }
}

fn table_diagnostics(lines: &[LineInfo<'_>], output: &mut Vec<Diagnostic>) {
    let mut index = 0;
    while index < lines.len() {
        if lines[index].kind != Some(BlockKind::Table) {
            index += 1;
            continue;
        }
        let mut end = index;
        while end < lines.len() && lines[end].kind == Some(BlockKind::Table) {
            end += 1;
        }
        check_table(&lines[index..end], output);
        index = end;
    }
}

fn check_table(rows: &[LineInfo<'_>], output: &mut Vec<Diagnostic>) {
    let Some(delimiter) = rows
        .iter()
        .position(|row| row.first_kind == SyntaxKind::TableDelimiter)
    else {
        return;
    };
    // A delimiter row on the first line never forms a GFM table, so the run has
    // no header to state a column count against.
    if delimiter == 0 {
        return;
    }
    let expected = cell_count(rows[0].text);
    for row in rows.iter().skip(delimiter) {
        if cell_count(row.text) != expected {
            output.push(DiagnosticKind::TableColumnMismatch.to_diagnostic(row.span()));
        }
    }
}

/// GFM cell count of a row: one more cell than pipes, minus the optional
/// leading and trailing pipes.
#[must_use]
fn cell_count(text: &str) -> usize {
    let row = text.trim();
    if row.is_empty() {
        return 0;
    }
    let pipes = row.chars().filter(|&ch| ch == '|').count();
    let mut cells = pipes + 1;
    if row.starts_with('|') {
        cells -= 1;
    }
    if row.ends_with('|') {
        cells -= 1;
    }
    cells.max(1)
}

/// Footnote references without a definition, and reused definition labels.
fn label_diagnostics(lexed: &Lexed<'_>, output: &mut Vec<Diagnostic>) {
    let mut defined: Vec<&str> = Vec::new();
    for token in lexed.tokens() {
        if token.kind == SyntaxKind::FootnoteDefinitionLabel
            && let Some(label) = footnote_label(token.text(lexed.source()))
        {
            defined.push(label);
        }
    }
    let mut seen_labels: Vec<&str> = Vec::new();
    for token in lexed.tokens() {
        match token.kind {
            SyntaxKind::FootnoteRef => {
                let Some(label) = footnote_label(token.text(lexed.source())) else {
                    continue;
                };
                if !defined.contains(&label) {
                    output
                        .push(DiagnosticKind::UndefinedFootnoteReference.to_diagnostic(token.span));
                }
            }
            SyntaxKind::LinkLabel => {
                let Some(label) = link_label(token.text(lexed.source())) else {
                    continue;
                };
                if seen_labels.iter().any(|seen| eq_label(seen, label)) {
                    output.push(DiagnosticKind::DuplicateLinkDefinition.to_diagnostic(token.span));
                } else {
                    seen_labels.push(label);
                }
            }
            _ => {}
        }
    }
}

/// `[^label]:` / `[^label]` → `label`.
#[must_use]
fn footnote_label(text: Option<&str>) -> Option<&str> {
    let text = text?.strip_prefix("[^")?;
    Some(text.strip_suffix("]:").or(text.strip_suffix(']'))?.trim())
}

/// `[label]:` → `label`.
#[must_use]
fn link_label(text: Option<&str>) -> Option<&str> {
    let text = text?.strip_prefix('[')?;
    Some(text.strip_suffix("]:").or(text.strip_suffix(']'))?.trim())
}

/// CommonMark label equality: ASCII case-insensitive, edges already trimmed.
#[must_use]
fn eq_label(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn codes(source: &str) -> Vec<&'static str> {
        validate(source).iter().map(|d| d.code).collect()
    }

    /// One document exercising every supported construct; it must produce zero
    /// diagnostics, so anything reported here is a regression.
    const VALID: &str = concat!(
        "---\n",
        "title: Notes\n",
        "tags:\n",
        "  - markdown\n",
        "---\n",
        "\n",
        "# Heading one\n",
        "\n",
        "Prose with **strong**, *em*, `code`, ~~struck~~ and a [link](https://example.com).\n",
        "\n",
        "Setext\n",
        "======\n",
        "\n",
        "- first item\n",
        "- [x] checked\n",
        "1. ordered\n",
        "\n",
        "> quoted with **bold**\n",
        "\n",
        "<details>\n",
        "<summary>more</summary>\n",
        "</details>\n",
        "\n",
        "```rust\n",
        "fn main() {}\n",
        "```\n",
        "\n",
        "| a | b |\n",
        "| --- | :-: |\n",
        "| 1 | 2 |\n",
        "\n",
        "Terminates with a footnote[^1] and an image ![alt](x.png).\n",
        "\n",
        "[^1]: the note\n",
        "[ref]: https://example.com \"title\"\n",
    );

    #[test]
    fn valid_document_has_no_diagnostics() {
        let parsed = parse(VALID);
        assert!(parsed.lexed().is_lossless());
        assert!(
            parsed.diagnostics().is_empty(),
            "{:?}",
            parsed
                .diagnostics()
                .iter()
                .map(|diagnostic| (diagnostic.code, diagnostic.span))
                .collect::<Vec<_>>()
        );
        assert!(parsed.is_valid());
    }

    #[test]
    fn valid_document_lists_every_block_kind() {
        let parsed = parse(VALID);
        let kinds: Vec<BlockKind> = parsed.blocks().iter().map(|block| block.kind).collect();
        for expected in [
            BlockKind::FrontMatter,
            BlockKind::Heading,
            BlockKind::Paragraph,
            BlockKind::List,
            BlockKind::Quote,
            BlockKind::CodeBlock,
            BlockKind::Table,
            BlockKind::Html,
        ] {
            assert!(kinds.contains(&expected), "{kinds:?}");
        }
    }

    #[test]
    fn unclosed_code_fence_is_reported() {
        assert_eq!(
            codes("```rust\nfn main() {}\n"),
            vec!["unclosed-code-fence"]
        );
    }

    #[test]
    fn unclosed_inline_constructs_are_reported() {
        assert_eq!(codes("text **more\n"), vec!["unclosed-emphasis"]);
        assert_eq!(codes("text `more\n"), vec!["unclosed-code-span"]);
        assert_eq!(codes("<b attr\n"), vec!["unclosed-html-tag"]);
        assert_eq!(codes("[a](b\n"), vec!["unclosed-link"]);
    }

    #[test]
    fn heading_levels_and_emptiness_are_reported() {
        assert_eq!(codes("####### seven\n"), vec!["invalid-heading-level"]);
        assert_eq!(codes("##\n"), vec!["empty-heading"]);
        assert_eq!(codes("###   \n"), vec!["empty-heading"]);
        assert_eq!(codes("## real title\n"), Vec::<&str>::new());
    }

    #[test]
    fn table_column_mismatch_is_reported() {
        assert_eq!(
            codes("| a | b |\n| --- |\n| 1 | 2 |\n"),
            vec!["table-column-mismatch"]
        );
        assert_eq!(
            codes("| a | b |\n| --- | --- |\n| 1 |\n"),
            vec!["table-column-mismatch"]
        );
        assert_eq!(
            codes("| a | b |\n| --- | --- |\n| 1 | 2 |\n"),
            Vec::<&str>::new()
        );
    }

    #[test]
    fn footnote_and_link_definition_labels_are_reported() {
        assert_eq!(
            codes("See [^missing].\n\n[^present]: note\n"),
            vec!["undefined-footnote-reference"]
        );
        assert_eq!(
            codes("[a]: /1\n\n[a]: /2\n"),
            vec!["duplicate-link-definition"]
        );
        assert_eq!(codes("[a]: /1\n\n[b]: /2\n"), Vec::<&str>::new());
    }

    #[test]
    fn recovery_keeps_the_stream_lossless() {
        let broken = concat!(
            "#\n",
            "```\n",
            "text **unclosed `span\n",
            "| a | b |\n",
            "| --- |\n",
            "[x]: /y\n",
            "[x]: /z\n",
            "[^q]\n",
            "<div\n",
            "####### deep\n",
        );
        let parsed = parse(broken);
        assert!(parsed.lexed().is_lossless());
        assert!(!parsed.is_valid());
        assert!(!parsed.diagnostics().is_empty());
        for diagnostic in parsed.diagnostics() {
            assert!(
                diagnostic.span.is_valid_for(broken),
                "{diagnostic:?} escapes the source"
            );
        }
    }

    #[test]
    fn diagnostics_are_in_source_order() {
        let source = "```\n**x\n\n[^a]\n";
        let diagnostics = validate(source);
        assert!(
            diagnostics
                .windows(2)
                .all(|pair| pair[0].span.start <= pair[1].span.start),
            "{diagnostics:?}"
        );
    }

    #[test]
    fn blocks_cover_the_document_without_overlap() {
        let parsed = parse(VALID);
        let blocks = parsed.blocks();
        assert!(!blocks.is_empty());
        for pair in blocks.windows(2) {
            assert!(
                pair[0].span.end <= pair[1].span.start,
                "{:?} overlaps {:?}",
                pair[0],
                pair[1]
            );
        }
    }

    #[test]
    fn empty_document_is_valid() {
        let parsed = parse("");
        assert!(parsed.is_valid());
        assert!(parsed.blocks().is_empty());
        assert!(parsed.lexed().tokens().is_empty());
    }
}
