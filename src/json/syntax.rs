//! Minimal lossless concrete syntax tree and validated text edits.

use std::{error::Error, fmt};

use crate::Span;

use super::{LexToken, Member, Parse, ParseOptions, Value, parse, parse_with};

/// Opaque identity of a node within one [`SyntaxTree`] snapshot.
///
/// IDs have no meaning in another tree or after applying edits and reparsing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NodeId(usize);

/// Opaque identity of a lexer token within one [`SyntaxTree`] snapshot.
///
/// IDs have no meaning in another tree or after applying edits and reparsing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TokenId(usize);

/// Kind of an AST-backed CST node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SyntaxNodeKind {
    Root,
    Object,
    Member,
    Property,
    Array,
    String,
    Number,
    Boolean,
    Null,
}

/// One direct lossless child of a [`SyntaxNode`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SyntaxElement {
    Node(NodeId),
    Token(TokenId),
}

impl SyntaxElement {
    #[must_use]
    pub const fn as_node(self) -> Option<NodeId> {
        match self {
            Self::Node(node) => Some(node),
            Self::Token(_) => None,
        }
    }

    #[must_use]
    pub const fn as_token(self) -> Option<TokenId> {
        match self {
            Self::Node(_) => None,
            Self::Token(token) => Some(token),
        }
    }
}

/// One immutable node in a [`SyntaxTree`] arena.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyntaxNode {
    id: NodeId,
    kind: SyntaxNodeKind,
    span: Span,
    parent: Option<NodeId>,
    elements: Vec<SyntaxElement>,
}

impl SyntaxNode {
    #[must_use]
    pub const fn id(&self) -> NodeId {
        self.id
    }

    #[must_use]
    pub const fn kind(&self) -> SyntaxNodeKind {
        self.kind
    }

    #[must_use]
    pub const fn span(&self) -> Span {
        self.span
    }

    #[must_use]
    pub const fn parent(&self) -> Option<NodeId> {
        self.parent
    }

    #[must_use]
    pub fn elements(&self) -> &[SyntaxElement] {
        &self.elements
    }
}

/// A lossless CST backed by a recovering [`Parse`].
///
/// The root always spans the complete source, even when no AST root could be
/// recovered. AST nodes form an arena below it. Every lexer token occurs as a
/// [`SyntaxElement::Token`] exactly once, at the deepest arena node not covered
/// by one of that node's direct AST children.
///
/// This is an immutable, AST-backed projection, not a green tree or an
/// incremental parser. [`SyntaxTree::apply_edits`] returns new source text;
/// reparsing it creates a new snapshot and invalidates all prior IDs.
///
/// ```
/// use themoretheless_tokenizer::json::{
///     ParseOptions, SyntaxElement, SyntaxNodeKind, syntax_tree_with,
/// };
///
/// let source = "/* note */ {\"x\": 1,}";
/// let tree = syntax_tree_with(source, ParseOptions::jsonc());
/// assert_eq!(tree.root().kind(), SyntaxNodeKind::Root);
/// assert_eq!(tree.root().span().end, source.len());
/// assert!(tree.root().elements().iter().any(|element| {
///     matches!(element, SyntaxElement::Token(token) if tree.token(*token).unwrap().text(source) == Some("/* note */"))
/// }));
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct SyntaxTree<'source> {
    parse: Parse<'source>,
    nodes: Vec<SyntaxNode>,
}

impl<'source> SyntaxTree<'source> {
    #[must_use]
    pub fn from_parse(parse: Parse<'source>) -> Self {
        let nodes = ArenaBuilder::new(&parse).build();
        Self { parse, nodes }
    }

    #[must_use]
    pub fn source(&self) -> &'source str {
        self.parse.source()
    }

    #[must_use]
    pub const fn parse(&self) -> &Parse<'source> {
        &self.parse
    }

    #[must_use]
    pub const fn root_id(&self) -> NodeId {
        NodeId(0)
    }

    #[must_use]
    pub fn root(&self) -> &SyntaxNode {
        &self.nodes[0]
    }

    #[must_use]
    pub fn node(&self, id: NodeId) -> Option<&SyntaxNode> {
        self.nodes.get(id.0)
    }

    #[must_use]
    pub fn token(&self, id: TokenId) -> Option<LexToken> {
        self.parse.lexed().tokens().get(id.0).copied()
    }

    #[must_use]
    pub fn nodes(&self) -> &[SyntaxNode] {
        &self.nodes
    }

    pub fn apply_edits(&self, edits: &[TextEdit]) -> Result<String, EditError> {
        apply_edits(self.source(), edits)
    }
}

/// Parses strict JSON and builds its lossless syntax tree.
#[must_use]
pub fn syntax_tree(source: &str) -> SyntaxTree<'_> {
    SyntaxTree::from_parse(parse(source))
}

/// Parses with explicit JSON/JSONC options and builds its lossless syntax tree.
#[must_use]
pub fn syntax_tree_with(source: &str, options: ParseOptions) -> SyntaxTree<'_> {
    SyntaxTree::from_parse(parse_with(source, options))
}

#[derive(Debug)]
struct NodeDraft {
    kind: SyntaxNodeKind,
    span: Span,
    parent: Option<NodeId>,
    children: Vec<NodeId>,
    tokens: Vec<TokenId>,
}

struct ArenaBuilder<'parse, 'source> {
    parse: &'parse Parse<'source>,
    drafts: Vec<NodeDraft>,
}

impl<'parse, 'source> ArenaBuilder<'parse, 'source> {
    fn new(parse: &'parse Parse<'source>) -> Self {
        Self {
            parse,
            drafts: Vec::new(),
        }
    }

    fn build(mut self) -> Vec<SyntaxNode> {
        let root = self.push(
            SyntaxNodeKind::Root,
            Span::new(0, self.parse.source().len()),
            None,
        );
        if let Some(value) = self.parse.value() {
            self.build_value(value, root);
        }

        for (index, token) in self.parse.lexed().tokens().iter().enumerate() {
            let owner = self.token_owner(root, token.span);
            self.drafts[owner.0].tokens.push(TokenId(index));
        }

        let mut nodes = Vec::with_capacity(self.drafts.len());
        for (index, draft) in self.drafts.iter().enumerate() {
            let mut elements = draft
                .children
                .iter()
                .copied()
                .map(SyntaxElement::Node)
                .chain(draft.tokens.iter().copied().map(SyntaxElement::Token))
                .collect::<Vec<_>>();
            elements.sort_by_key(|element| match *element {
                SyntaxElement::Node(id) => (self.drafts[id.0].span.start, 0_usize),
                SyntaxElement::Token(id) => (self.parse.lexed().tokens()[id.0].span.start, 1_usize),
            });
            nodes.push(SyntaxNode {
                id: NodeId(index),
                kind: draft.kind,
                span: draft.span,
                parent: draft.parent,
                elements,
            });
        }
        nodes
    }

    fn push(&mut self, kind: SyntaxNodeKind, span: Span, parent: Option<NodeId>) -> NodeId {
        let id = NodeId(self.drafts.len());
        self.drafts.push(NodeDraft {
            kind,
            span,
            parent,
            children: Vec::new(),
            tokens: Vec::new(),
        });
        if let Some(parent) = parent {
            self.drafts[parent.0].children.push(id);
        }
        id
    }

    fn build_value(&mut self, value: &Value<'source>, parent: NodeId) -> NodeId {
        let kind = match value {
            Value::Object(_) => SyntaxNodeKind::Object,
            Value::Array(_) => SyntaxNodeKind::Array,
            Value::String(_) => SyntaxNodeKind::String,
            Value::Number(_) => SyntaxNodeKind::Number,
            Value::Boolean(_) => SyntaxNodeKind::Boolean,
            Value::Null(_) => SyntaxNodeKind::Null,
        };
        let id = self.push(kind, value.span(), Some(parent));
        match value {
            Value::Object(object) => {
                for member in object.members() {
                    self.build_member(member, id);
                }
            }
            Value::Array(array) => {
                for element in array.elements() {
                    self.build_value(element, id);
                }
            }
            Value::String(_) | Value::Number(_) | Value::Boolean(_) | Value::Null(_) => {}
        }
        id
    }

    fn build_member(&mut self, member: &Member<'source>, parent: NodeId) {
        let member_id = self.push(SyntaxNodeKind::Member, member.span(), Some(parent));
        self.push(
            SyntaxNodeKind::Property,
            member.key().span(),
            Some(member_id),
        );
        self.build_value(member.value(), member_id);
    }

    fn token_owner(&self, node: NodeId, token_span: Span) -> NodeId {
        for child in &self.drafts[node.0].children {
            let child_span = self.drafts[child.0].span;
            if child_span.start <= token_span.start && token_span.end <= child_span.end {
                return self.token_owner(*child, token_span);
            }
        }
        node
    }
}

/// One source replacement in a validated batch.
///
/// Its span uses coordinates in the original source snapshot. Applying edits
/// returns a new `String`; it does not mutate, incrementally reparse, or reuse
/// the old [`SyntaxTree`], and IDs from that tree do not identify nodes in a
/// subsequently parsed tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextEdit {
    span: Span,
    replacement: String,
}

impl TextEdit {
    #[must_use]
    pub fn new(span: Span, replacement: impl Into<String>) -> Self {
        Self {
            span,
            replacement: replacement.into(),
        }
    }

    #[must_use]
    pub fn insert(offset: usize, text: impl Into<String>) -> Self {
        Self::new(Span::new(offset, offset), text)
    }

    #[must_use]
    pub fn delete(span: Span) -> Self {
        Self::new(span, "")
    }

    #[must_use]
    pub const fn span(&self) -> Span {
        self.span
    }

    #[must_use]
    pub fn replacement(&self) -> &str {
        &self.replacement
    }
}

/// Why a batch of text edits could not be applied safely.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum EditError {
    ReversedSpan {
        edit_index: usize,
        span: Span,
    },
    OutOfBounds {
        edit_index: usize,
        span: Span,
        source_len: usize,
    },
    NotCharBoundary {
        edit_index: usize,
        offset: usize,
    },
    Unsorted {
        previous_index: usize,
        edit_index: usize,
    },
    Overlapping {
        previous_index: usize,
        edit_index: usize,
    },
    OutputTooLarge,
}

impl fmt::Display for EditError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ReversedSpan { edit_index, span } => {
                write!(formatter, "edit {edit_index} has reversed span {span:?}")
            }
            Self::OutOfBounds {
                edit_index,
                span,
                source_len,
            } => write!(
                formatter,
                "edit {edit_index} span {span:?} exceeds source length {source_len}"
            ),
            Self::NotCharBoundary { edit_index, offset } => {
                write!(formatter, "edit {edit_index} offset {offset} splits UTF-8")
            }
            Self::Unsorted {
                previous_index,
                edit_index,
            } => write!(
                formatter,
                "edit {edit_index} is ordered before edit {previous_index}"
            ),
            Self::Overlapping {
                previous_index,
                edit_index,
            } => write!(
                formatter,
                "edit {edit_index} overlaps edit {previous_index}"
            ),
            Self::OutputTooLarge => formatter.write_str("edited output length overflows usize"),
        }
    }
}

impl Error for EditError {}

/// Applies an ordered, non-overlapping batch of edits to any UTF-8 string.
///
/// Zero-length-span insertions at the same offset are allowed and retain input
/// order. An insertion immediately before a replacement with the same start is
/// also allowed, so their input order is observable; putting that insertion
/// after the replacement is rejected as overlapping. Touching non-empty
/// replacement ranges are allowed.
///
/// ```
/// use themoretheless_tokenizer::{Span, json::{TextEdit, apply_edits}};
///
/// let source = "[\"old\", true]";
/// let edits = [
///     TextEdit::new(Span::new(2, 5), "new"),
///     TextEdit::new(Span::new(8, 12), "false"),
/// ];
/// assert_eq!(apply_edits(source, &edits).unwrap(), "[\"new\", false]");
/// ```
///
/// # Errors
///
/// Rejects reversed or out-of-bounds spans, offsets which split a UTF-8
/// character, edits not ordered by start offset, and edits which overlap or
/// move back into a range already consumed by an earlier replacement.
pub fn apply_edits(source: &str, edits: &[TextEdit]) -> Result<String, EditError> {
    for (index, edit) in edits.iter().enumerate() {
        if edit.span.start > edit.span.end {
            return Err(EditError::ReversedSpan {
                edit_index: index,
                span: edit.span,
            });
        }
        if edit.span.end > source.len() {
            return Err(EditError::OutOfBounds {
                edit_index: index,
                span: edit.span,
                source_len: source.len(),
            });
        }
        for offset in [edit.span.start, edit.span.end] {
            if !source.is_char_boundary(offset) {
                return Err(EditError::NotCharBoundary {
                    edit_index: index,
                    offset,
                });
            }
        }
        if index > 0 {
            let previous = &edits[index - 1];
            if edit.span.start < previous.span.start {
                return Err(EditError::Unsorted {
                    previous_index: index - 1,
                    edit_index: index,
                });
            }
            if edit.span.start < previous.span.end {
                return Err(EditError::Overlapping {
                    previous_index: index - 1,
                    edit_index: index,
                });
            }
        }
    }

    let output_length = edits.iter().try_fold(source.len(), |length, edit| {
        length
            .checked_sub(edit.span.len())
            .and_then(|length| length.checked_add(edit.replacement.len()))
            .ok_or(EditError::OutputTooLarge)
    })?;
    let mut output = String::with_capacity(output_length);
    let mut cursor = 0;
    for edit in edits {
        output.push_str(&source[cursor..edit.span.start]);
        output.push_str(&edit.replacement);
        cursor = edit.span.end;
    }
    output.push_str(&source[cursor..]);
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::json::SyntaxKind;

    fn flatten<'source>(
        tree: &SyntaxTree<'source>,
        node: NodeId,
        token_ids: &mut Vec<usize>,
        text: &mut String,
    ) {
        for element in tree.node(node).unwrap().elements() {
            match *element {
                SyntaxElement::Node(child) => flatten(tree, child, token_ids, text),
                SyntaxElement::Token(token) => {
                    let token_value = tree.token(token).unwrap();
                    token_ids.push(token.0);
                    text.push_str(token_value.text(tree.source()).unwrap());
                }
            }
        }
    }

    fn assert_geometry(tree: &SyntaxTree<'_>, node: NodeId) {
        let parent = tree.node(node).unwrap();
        let mut previous_end = parent.span().start;
        for element in parent.elements() {
            let span = match *element {
                SyntaxElement::Node(child) => {
                    let child = tree.node(child).unwrap();
                    assert_eq!(child.parent(), Some(node));
                    assert_geometry(tree, child.id());
                    child.span()
                }
                SyntaxElement::Token(token) => tree.token(token).unwrap().span,
            };
            assert!(parent.span().start <= span.start);
            assert!(span.end <= parent.span().end);
            assert!(previous_end <= span.start);
            previous_end = span.end;
        }
    }

    #[test]
    fn every_token_occurs_once_in_lossless_nested_jsonc_recovery() {
        let source =
            "\u{feff}/*head*/ {\"a\":[1, /*inside*/ 2], \"bad\": @, \"tail\":true,} trailing";
        let tree = syntax_tree_with(source, ParseOptions::jsonc());
        assert!(!tree.parse().is_valid());
        assert_eq!(tree.root().span(), Span::new(0, source.len()));

        let mut token_ids = Vec::new();
        let mut rebuilt = String::new();
        flatten(&tree, tree.root_id(), &mut token_ids, &mut rebuilt);
        assert_eq!(rebuilt, source);
        assert_eq!(
            token_ids,
            (0..tree.parse().lexed().tokens().len()).collect::<Vec<_>>()
        );
        assert_geometry(&tree, tree.root_id());

        let kinds = tree
            .nodes()
            .iter()
            .map(SyntaxNode::kind)
            .collect::<Vec<_>>();
        assert!(kinds.contains(&SyntaxNodeKind::Object));
        assert!(kinds.contains(&SyntaxNodeKind::Member));
        assert!(kinds.contains(&SyntaxNodeKind::Property));
        assert!(kinds.contains(&SyntaxNodeKind::Array));
        assert!(kinds.contains(&SyntaxNodeKind::Number));
        assert!(kinds.contains(&SyntaxNodeKind::Boolean));
        assert!(tree.parse().lexed().tokens().iter().any(|token| {
            token.kind == SyntaxKind::BlockComment && token.text(source) == Some("/*inside*/")
        }));

        for node in tree.nodes().iter().skip(1) {
            assert!(tree.node(node.parent().unwrap()).is_some());
        }
    }

    #[test]
    fn rootless_tree_still_owns_all_recovery_tokens() {
        let source = " @ /* no root */ ";
        let tree = syntax_tree_with(source, ParseOptions::jsonc());
        assert!(tree.parse().value().is_none());
        assert_eq!(tree.nodes().len(), 1);

        let mut token_ids = Vec::new();
        let mut rebuilt = String::new();
        flatten(&tree, tree.root_id(), &mut token_ids, &mut rebuilt);
        assert_eq!(rebuilt, source);
        assert_eq!(token_ids.len(), tree.parse().lexed().tokens().len());
    }

    #[test]
    fn nested_nodes_and_unicode_spans_are_stable() {
        let source = r#"{"город":{"имя":"Москва"},"emoji":"😀"}"#;
        let tree = syntax_tree(source);
        let string_nodes = tree
            .nodes()
            .iter()
            .filter(|node| node.kind() == SyntaxNodeKind::String)
            .collect::<Vec<_>>();
        assert_eq!(string_nodes.len(), 2);
        for node in string_nodes {
            assert!(node.span().is_valid_for(source));
            assert_eq!(tree.node(node.id()).unwrap(), node);
        }
        assert_eq!(tree.root().parent(), None);
    }

    #[test]
    fn applies_ordered_unicode_safe_batch_edits() {
        let source = "a😀bc";
        let edits = [
            TextEdit::insert(0, "^"),
            TextEdit::new(Span::new(1, 5), "🙂"),
            TextEdit::delete(Span::new(5, 6)),
            TextEdit::insert(source.len(), "$"),
        ];
        assert_eq!(apply_edits(source, &edits).unwrap(), "^a🙂c$");
        assert_eq!(syntax_tree("null").apply_edits(&[]).unwrap(), "null");

        let same_offset = [TextEdit::insert(1, "X"), TextEdit::insert(1, "Y")];
        assert_eq!(apply_edits("ab", &same_offset).unwrap(), "aXYb");
        let insert_then_replace = [
            TextEdit::insert(1, "X"),
            TextEdit::new(Span::new(1, 2), "Y"),
        ];
        assert_eq!(apply_edits("ab", &insert_then_replace).unwrap(), "aXY");
        assert!(matches!(
            apply_edits(
                "ab",
                &[insert_then_replace[1].clone(), same_offset[0].clone()]
            ),
            Err(EditError::Overlapping { .. })
        ));
    }

    #[test]
    fn rejects_invalid_edit_batches_without_panicking() {
        let source = "a😀bc";
        assert!(matches!(
            apply_edits(source, &[TextEdit::delete(Span::new(5, 1))]),
            Err(EditError::ReversedSpan { .. })
        ));
        assert!(matches!(
            apply_edits(source, &[TextEdit::delete(Span::new(0, 99))]),
            Err(EditError::OutOfBounds { .. })
        ));
        assert!(matches!(
            apply_edits(source, &[TextEdit::insert(2, "x")]),
            Err(EditError::NotCharBoundary { .. })
        ));
        assert!(matches!(
            apply_edits(
                source,
                &[TextEdit::insert(5, "x"), TextEdit::insert(1, "y"),]
            ),
            Err(EditError::Unsorted { .. })
        ));
        assert!(matches!(
            apply_edits(
                source,
                &[TextEdit::delete(Span::new(0, 5)), TextEdit::insert(1, "x"),]
            ),
            Err(EditError::Overlapping { .. })
        ));
    }
}
