//! Borrowing AST for YAML 1.2.
//!
//! Every node borrows from the source string — no copies are made. Decoded
//! scalars use `Cow<'source, str>` to stay zero-copy when no escape processing
//! is needed.

use std::borrow::Cow;

use crate::Span;

/// The kind of a YAML node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum NodeKind {
    Scalar,
    Sequence,
    Mapping,
    Alias,
}

/// A YAML node.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum Node<'source> {
    Scalar(Scalar<'source>),
    Sequence(Sequence<'source>),
    Mapping(Mapping<'source>),
    Alias(Alias<'source>),
}

impl<'source> Node<'source> {
    /// Returns the kind of this node.
    #[must_use]
    pub fn kind(&self) -> NodeKind {
        match self {
            Node::Scalar(_) => NodeKind::Scalar,
            Node::Sequence(_) => NodeKind::Sequence,
            Node::Mapping(_) => NodeKind::Mapping,
            Node::Alias(_) => NodeKind::Alias,
        }
    }

    /// Returns the span covering this node.
    #[must_use]
    pub fn span(&self) -> Span {
        match self {
            Node::Scalar(s) => s.span,
            Node::Sequence(s) => s.span,
            Node::Mapping(m) => m.span,
            Node::Alias(a) => a.span,
        }
    }

    /// Downcasts to a scalar if this is one.
    #[must_use]
    pub fn as_scalar(&self) -> Option<&Scalar<'source>> {
        match self {
            Node::Scalar(s) => Some(s),
            _ => None,
        }
    }

    /// Downcasts to a sequence if this is one.
    #[must_use]
    pub fn as_sequence(&self) -> Option<&Sequence<'source>> {
        match self {
            Node::Sequence(s) => Some(s),
            _ => None,
        }
    }

    /// Downcasts to a mapping if this is one.
    #[must_use]
    pub fn as_mapping(&self) -> Option<&Mapping<'source>> {
        match self {
            Node::Mapping(m) => Some(m),
            _ => None,
        }
    }

    /// Downcasts to an alias if this is one.
    #[must_use]
    pub fn as_alias(&self) -> Option<&Alias<'source>> {
        match self {
            Node::Alias(a) => Some(a),
            _ => None,
        }
    }

    /// Returns the decoded scalar text if this is a scalar node.
    #[must_use]
    pub fn as_str(&self) -> Option<&str> {
        self.as_scalar().and_then(|s| s.decoded())
    }
}

/// A YAML scalar value.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Scalar<'source> {
    raw: &'source str,
    decoded: Option<Cow<'source, str>>,
    pub(crate) anchor: Option<Anchor<'source>>,
    pub(crate) tag: Option<TagHandle<'source>>,
    span: Span,
    style: ScalarStyle,
    valid: bool,
}

impl<'source> Scalar<'source> {
    pub(crate) fn new(
        raw: &'source str,
        decoded: Option<Cow<'source, str>>,
        anchor: Option<Anchor<'source>>,
        tag: Option<TagHandle<'source>>,
        span: Span,
        style: ScalarStyle,
        valid: bool,
    ) -> Self {
        Self {
            raw,
            decoded,
            anchor,
            tag,
            span,
            style,
            valid,
        }
    }

    /// Returns the raw source text including delimiters.
    #[must_use]
    pub fn raw(&self) -> &'source str {
        self.raw
    }

    /// Returns the decoded text, or `None` if decoding failed.
    #[must_use]
    pub fn decoded(&self) -> Option<&str> {
        self.decoded.as_deref()
    }

    /// Returns the decoded text with fallback to raw.
    #[must_use]
    pub fn text(&self) -> &str {
        self.decoded.as_deref().unwrap_or(self.raw)
    }

    /// Returns the anchor if present.
    #[must_use]
    pub fn anchor(&self) -> Option<&Anchor<'source>> {
        self.anchor.as_ref()
    }

    /// Returns the tag if present.
    #[must_use]
    pub fn tag(&self) -> Option<&TagHandle<'source>> {
        self.tag.as_ref()
    }

    /// Returns the span covering this scalar.
    #[must_use]
    pub fn span(&self) -> Span {
        self.span
    }

    /// Returns the scalar style.
    #[must_use]
    pub fn style(&self) -> ScalarStyle {
        self.style
    }

    /// Returns `true` if the scalar is valid.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.valid
    }
}

/// The style of a YAML scalar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ScalarStyle {
    Plain,
    SingleQuoted,
    DoubleQuoted,
    Literal,
    Folded,
}

/// A YAML sequence (ordered list of nodes).
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Sequence<'source> {
    elements: Vec<Node<'source>>,
    pub(crate) anchor: Option<Anchor<'source>>,
    pub(crate) tag: Option<TagHandle<'source>>,
    span: Span,
    style: CollectionStyle,
}

impl<'source> Sequence<'source> {
    pub(crate) fn new(
        elements: Vec<Node<'source>>,
        anchor: Option<Anchor<'source>>,
        tag: Option<TagHandle<'source>>,
        span: Span,
        style: CollectionStyle,
    ) -> Self {
        Self {
            elements,
            anchor,
            tag,
            span,
            style,
        }
    }

    /// Returns the elements of this sequence.
    #[must_use]
    pub fn elements(&self) -> &[Node<'source>] {
        &self.elements
    }

    /// Returns the number of elements.
    #[must_use]
    pub fn len(&self) -> usize {
        self.elements.len()
    }

    /// Returns `true` if this sequence has no elements.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.elements.is_empty()
    }

    /// Returns the anchor if present.
    #[must_use]
    pub fn anchor(&self) -> Option<&Anchor<'source>> {
        self.anchor.as_ref()
    }

    /// Returns the tag if present.
    #[must_use]
    pub fn tag(&self) -> Option<&TagHandle<'source>> {
        self.tag.as_ref()
    }

    /// Returns the span covering this sequence.
    #[must_use]
    pub fn span(&self) -> Span {
        self.span
    }

    /// Returns the collection style.
    #[must_use]
    pub fn style(&self) -> CollectionStyle {
        self.style
    }
}

/// A YAML mapping (ordered list of key-value pairs).
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Mapping<'source> {
    entries: Vec<Entry<'source>>,
    pub(crate) anchor: Option<Anchor<'source>>,
    pub(crate) tag: Option<TagHandle<'source>>,
    span: Span,
    style: CollectionStyle,
}

impl<'source> Mapping<'source> {
    pub(crate) fn new(
        entries: Vec<Entry<'source>>,
        anchor: Option<Anchor<'source>>,
        tag: Option<TagHandle<'source>>,
        span: Span,
        style: CollectionStyle,
    ) -> Self {
        Self {
            entries,
            anchor,
            tag,
            span,
            style,
        }
    }

    /// Returns the entries of this mapping.
    #[must_use]
    pub fn entries(&self) -> &[Entry<'source>] {
        &self.entries
    }

    /// Returns the number of entries.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns `true` if this mapping has no entries.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Returns the anchor if present.
    #[must_use]
    pub fn anchor(&self) -> Option<&Anchor<'source>> {
        self.anchor.as_ref()
    }

    /// Returns the tag if present.
    #[must_use]
    pub fn tag(&self) -> Option<&TagHandle<'source>> {
        self.tag.as_ref()
    }

    /// Returns the span covering this mapping.
    #[must_use]
    pub fn span(&self) -> Span {
        self.span
    }

    /// Returns the collection style.
    #[must_use]
    pub fn style(&self) -> CollectionStyle {
        self.style
    }

    /// Looks up a value by string key (linear scan).
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&Node<'source>> {
        self.entries
            .iter()
            .find(|entry| entry.key().as_str() == Some(key))
            .map(|entry| entry.value())
            .flatten()
    }
}

/// A key-value pair in a YAML mapping.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Entry<'source> {
    key: Node<'source>,
    value: Option<Node<'source>>,
    span: Span,
}

impl<'source> Entry<'source> {
    pub(crate) fn new(key: Node<'source>, value: Option<Node<'source>>, span: Span) -> Self {
        Self { key, value, span }
    }

    /// Returns the key node.
    #[must_use]
    pub fn key(&self) -> &Node<'source> {
        &self.key
    }

    /// Returns the value node, or `None` if the value is implicit null.
    #[must_use]
    pub fn value(&self) -> Option<&Node<'source>> {
        self.value.as_ref()
    }

    /// Returns the span covering this entry.
    #[must_use]
    pub fn span(&self) -> Span {
        self.span
    }
}

/// An anchor attachment on a node.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Anchor<'source> {
    name: &'source str,
    span: Span,
}

impl<'source> Anchor<'source> {
    pub(crate) fn new(name: &'source str, span: Span) -> Self {
        Self { name, span }
    }

    /// Returns the anchor name (without the `&` prefix).
    #[must_use]
    pub fn name(&self) -> &'source str {
        self.name
    }

    /// Returns the span covering the anchor including the `&`.
    #[must_use]
    pub fn span(&self) -> Span {
        self.span
    }
}

/// An alias reference to an anchor.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Alias<'source> {
    name: &'source str,
    span: Span,
}

impl<'source> Alias<'source> {
    pub(crate) fn new(name: &'source str, span: Span) -> Self {
        Self { name, span }
    }

    /// Returns the alias name (without the `*` prefix).
    #[must_use]
    pub fn name(&self) -> &'source str {
        self.name
    }

    /// Returns the span covering the alias including the `*`.
    #[must_use]
    pub fn span(&self) -> Span {
        self.span
    }
}

/// A tag attachment on a node.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct TagHandle<'source> {
    raw: &'source str,
    span: Span,
}

impl<'source> TagHandle<'source> {
    pub(crate) fn new(raw: &'source str, span: Span) -> Self {
        Self { raw, span }
    }

    /// Returns the raw tag text including the `!` prefix.
    #[must_use]
    pub fn raw(&self) -> &'source str {
        self.raw
    }

    /// Returns the span covering the tag.
    #[must_use]
    pub fn span(&self) -> Span {
        self.span
    }
}

/// A YAML directive (e.g., `%YAML 1.2`).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Directive<'source> {
    raw: &'source str,
    span: Span,
}

impl<'source> Directive<'source> {
    pub(crate) fn new(raw: &'source str, span: Span) -> Self {
        Self { raw, span }
    }

    /// Returns the raw directive text including the `%` prefix.
    #[must_use]
    pub fn raw(&self) -> &'source str {
        self.raw
    }

    /// Returns the span covering the directive.
    #[must_use]
    pub fn span(&self) -> Span {
        self.span
    }
}

/// The style of a YAML collection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum CollectionStyle {
    Block,
    Flow,
}

/// A YAML document within a stream.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Document<'source> {
    directives: Vec<Directive<'source>>,
    root: Option<Node<'source>>,
    start_marker: Option<Span>,
    end_marker: Option<Span>,
    span: Span,
}

impl<'source> Document<'source> {
    pub(crate) fn new(
        directives: Vec<Directive<'source>>,
        root: Option<Node<'source>>,
        start_marker: Option<Span>,
        end_marker: Option<Span>,
        span: Span,
    ) -> Self {
        Self {
            directives,
            root,
            start_marker,
            end_marker,
            span,
        }
    }

    /// Returns the directives in this document.
    #[must_use]
    pub fn directives(&self) -> &[Directive<'source>] {
        &self.directives
    }

    /// Returns the root node, or `None` if the document is empty.
    #[must_use]
    pub fn root(&self) -> Option<&Node<'source>> {
        self.root.as_ref()
    }

    /// Returns the span of the `---` marker, if present.
    #[must_use]
    pub fn start_marker(&self) -> Option<Span> {
        self.start_marker
    }

    /// Returns the span of the `...` marker, if present.
    #[must_use]
    pub fn end_marker(&self) -> Option<Span> {
        self.end_marker
    }

    /// Returns the span covering this document.
    #[must_use]
    pub fn span(&self) -> Span {
        self.span
    }
}
