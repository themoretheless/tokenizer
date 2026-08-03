//! Byte-offset navigation through a parsed JSON value.

use std::{error::Error, fmt};

use crate::Span;

use super::{Member, Parse, StringValue, Value};

/// Why an AST navigation offset could not be interpreted safely.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum NavigationError {
    OffsetOutOfBounds { offset: usize, source_len: usize },
    OffsetNotCharBoundary { offset: usize },
}

impl fmt::Display for NavigationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OffsetOutOfBounds { offset, source_len } => write!(
                formatter,
                "byte offset {offset} is outside the source of length {source_len}"
            ),
            Self::OffsetNotCharBoundary { offset } => {
                write!(formatter, "byte offset {offset} splits a UTF-8 character")
            }
        }
    }
}

impl Error for NavigationError {}

/// The most specific AST node containing a byte offset.
///
/// Object members and keys are nodes in their own right. Container values are
/// returned when the offset falls on their punctuation or between children.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub enum NodeRef<'ast, 'source> {
    Value(&'ast Value<'source>),
    ObjectMember(&'ast Member<'source>),
    ObjectKey(&'ast StringValue<'source>),
}

impl<'ast, 'source> NodeRef<'ast, 'source> {
    #[must_use]
    pub const fn span(self) -> Span {
        match self {
            Self::Value(value) => value.span(),
            Self::ObjectMember(member) => member.span(),
            Self::ObjectKey(key) => key.span(),
        }
    }

    #[must_use]
    pub const fn as_value(self) -> Option<&'ast Value<'source>> {
        match self {
            Self::Value(value) => Some(value),
            Self::ObjectMember(_) | Self::ObjectKey(_) => None,
        }
    }

    #[must_use]
    pub const fn as_object_member(self) -> Option<&'ast Member<'source>> {
        match self {
            Self::ObjectMember(member) => Some(member),
            Self::Value(_) | Self::ObjectKey(_) => None,
        }
    }

    #[must_use]
    pub const fn as_object_key(self) -> Option<&'ast StringValue<'source>> {
        match self {
            Self::Value(_) | Self::ObjectMember(_) => None,
            Self::ObjectKey(key) => Some(key),
        }
    }
}

/// One unambiguous step from the root to a JSON node.
///
/// Object member indices preserve source order and distinguish duplicate keys.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub enum AstPathSegment<'ast, 'source> {
    ObjectMember {
        member_index: usize,
        key: &'ast StringValue<'source>,
    },
    ArrayElement {
        index: usize,
    },
}

/// A root-relative path to the node selected by [`path_at_offset`].
#[derive(Debug, Clone, PartialEq, Default)]
pub struct AstPath<'ast, 'source> {
    segments: Vec<AstPathSegment<'ast, 'source>>,
}

impl<'ast, 'source> AstPath<'ast, 'source> {
    #[must_use]
    pub fn segments(&self) -> &[AstPathSegment<'ast, 'source>] {
        &self.segments
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.segments.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.segments.is_empty()
    }

    pub fn iter(&self) -> impl ExactSizeIterator<Item = AstPathSegment<'ast, 'source>> + '_ {
        self.segments.iter().copied()
    }

    #[must_use]
    pub fn into_segments(self) -> Vec<AstPathSegment<'ast, 'source>> {
        self.segments
    }
}

/// Finds the most specific AST node containing `offset`.
///
/// Offsets and spans are half-open UTF-8 byte ranges: a node includes its
/// `span().start` but excludes its `span().end`. Trivia outside the root and
/// parser-recovery text inside a container selects the smallest enclosing AST
/// node when it has no more specific node of its own.
///
/// ```
/// use themoretheless_tokenizer::json::{NodeRef, node_at_offset, parse};
///
/// let source = r#"{"items":[10, 20]}"#;
/// let parsed = parse(source);
/// let offset = source.find("20").unwrap();
/// let node = node_at_offset(&parsed, offset).unwrap().unwrap();
/// assert!(matches!(node, NodeRef::Value(value) if value.as_number().is_some()));
/// ```
///
/// # Errors
///
/// Returns [`NavigationError`] when the byte offset is outside the source or
/// splits a UTF-8 character.
pub fn node_at_offset<'ast, 'source>(
    parsed: &'ast Parse<'source>,
    offset: usize,
) -> Result<Option<NodeRef<'ast, 'source>>, NavigationError> {
    validate_offset(parsed.source(), offset)?;
    Ok(parsed
        .value()
        .and_then(|root| locate(root, offset, &mut Vec::new())))
}

/// Returns the object-member and array-element path containing `offset`.
///
/// The root value has an empty path. Selecting either an object key or its
/// value produces the same member segment. `member_index` distinguishes
/// duplicate keys without discarding their decoded or raw spelling.
///
/// # Errors
///
/// Returns [`NavigationError`] when the byte offset is outside the source or
/// splits a UTF-8 character.
pub fn path_at_offset<'ast, 'source>(
    parsed: &'ast Parse<'source>,
    offset: usize,
) -> Result<Option<AstPath<'ast, 'source>>, NavigationError> {
    validate_offset(parsed.source(), offset)?;
    let Some(root) = parsed.value() else {
        return Ok(None);
    };
    let mut segments = Vec::new();
    if locate(root, offset, &mut segments).is_none() {
        return Ok(None);
    }
    Ok(Some(AstPath { segments }))
}

impl<'source> Parse<'source> {
    pub fn node_at_offset<'ast>(
        &'ast self,
        offset: usize,
    ) -> Result<Option<NodeRef<'ast, 'source>>, NavigationError> {
        node_at_offset(self, offset)
    }

    pub fn path_at_offset<'ast>(
        &'ast self,
        offset: usize,
    ) -> Result<Option<AstPath<'ast, 'source>>, NavigationError> {
        path_at_offset(self, offset)
    }
}

fn validate_offset(source: &str, offset: usize) -> Result<(), NavigationError> {
    if offset > source.len() {
        return Err(NavigationError::OffsetOutOfBounds {
            offset,
            source_len: source.len(),
        });
    }
    if !source.is_char_boundary(offset) {
        return Err(NavigationError::OffsetNotCharBoundary { offset });
    }
    Ok(())
}

fn locate<'ast, 'source>(
    value: &'ast Value<'source>,
    offset: usize,
    path: &mut Vec<AstPathSegment<'ast, 'source>>,
) -> Option<NodeRef<'ast, 'source>> {
    if !value.span().contains(offset) {
        return None;
    }

    match value {
        Value::Object(object) => {
            for (member_index, member) in object.members().iter().enumerate() {
                if !member.span().contains(offset) {
                    continue;
                }
                let segment = AstPathSegment::ObjectMember {
                    member_index,
                    key: member.key(),
                };
                if member.key().span().contains(offset) {
                    path.push(segment);
                    return Some(NodeRef::ObjectKey(member.key()));
                }
                if member.value().span().contains(offset) {
                    path.push(segment);
                    return locate(member.value(), offset, path);
                }
                path.push(segment);
                return Some(NodeRef::ObjectMember(member));
            }
        }
        Value::Array(array) => {
            for (index, element) in array.elements().iter().enumerate() {
                if element.span().contains(offset) {
                    path.push(AstPathSegment::ArrayElement { index });
                    return locate(element, offset, path);
                }
            }
        }
        Value::String(_) | Value::Number(_) | Value::Boolean(_) | Value::Null(_) => {}
    }

    Some(NodeRef::Value(value))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::json::{ValueKind, parse};

    #[test]
    fn finds_deepest_keys_and_values_with_exact_boundaries() {
        let source = r#" {"items":[10,{"name":"Москва"}]} "#;
        let parsed = parse(source);
        let root = parsed.value().unwrap();

        assert!(node_at_offset(&parsed, 0).unwrap().is_none());
        assert_eq!(node_at_offset(&parsed, root.span().end).unwrap(), None);
        assert_eq!(
            node_at_offset(&parsed, root.span().start)
                .unwrap()
                .unwrap()
                .span(),
            root.span()
        );

        let key_offset = source.find("name").unwrap();
        let key = node_at_offset(&parsed, key_offset)
            .unwrap()
            .unwrap()
            .as_object_key()
            .unwrap();
        assert_eq!(key.decoded(), Some("name"));

        let colon_offset = source[key.span().end..].find(':').unwrap() + key.span().end;
        let member = node_at_offset(&parsed, colon_offset)
            .unwrap()
            .unwrap()
            .as_object_member()
            .unwrap();
        assert_eq!(member.key().decoded(), Some("name"));
        assert_eq!(
            path_at_offset(&parsed, colon_offset)
                .unwrap()
                .unwrap()
                .len(),
            3
        );

        let value_offset = source.find("Москва").unwrap();
        let value = node_at_offset(&parsed, value_offset)
            .unwrap()
            .unwrap()
            .as_value()
            .unwrap();
        assert_eq!(value.kind(), ValueKind::String);

        let path = path_at_offset(&parsed, value_offset).unwrap().unwrap();
        assert_eq!(path.len(), 3);
        assert!(matches!(
            path.segments(),
            [
                AstPathSegment::ObjectMember {
                    member_index: 0,
                    ..
                },
                AstPathSegment::ArrayElement { index: 1 },
                AstPathSegment::ObjectMember {
                    member_index: 0,
                    ..
                },
            ]
        ));
    }

    #[test]
    fn duplicate_keys_are_disambiguated_by_member_index() {
        let source = r#"{"x":1,"x":{"x":2}}"#;
        let parsed = parse(source);
        let second_key_offset = source.rfind("\"x\"").unwrap();
        let path = path_at_offset(&parsed, second_key_offset).unwrap().unwrap();

        assert!(matches!(
            path.segments(),
            [
                AstPathSegment::ObjectMember { member_index: 1, key },
                AstPathSegment::ObjectMember { member_index: 0, .. },
            ] if key.decoded() == Some("x")
        ));
    }

    #[test]
    fn navigates_values_retained_by_recovery() {
        let source = r#"{"items":[1 01 false],"tail":3}"#;
        let parsed = parse(source);
        assert!(!parsed.is_valid());

        let false_offset = source.find("false").unwrap();
        let path = path_at_offset(&parsed, false_offset).unwrap().unwrap();
        assert!(matches!(
            path.segments(),
            [
                AstPathSegment::ObjectMember {
                    member_index: 0,
                    ..
                },
                AstPathSegment::ArrayElement { index: 2 },
            ]
        ));
        assert_eq!(
            node_at_offset(&parsed, false_offset)
                .unwrap()
                .and_then(NodeRef::as_value)
                .and_then(Value::as_bool),
            Some(false)
        );
    }

    #[test]
    fn validates_offsets_and_handles_rootless_or_unretained_syntax() {
        let source = r#" {"a\nb":"Москва","missing":} "#;
        let parsed = parse(source);
        let continuation = source.find("Москва").unwrap() + 1;
        assert_eq!(
            parsed.node_at_offset(continuation),
            Err(NavigationError::OffsetNotCharBoundary {
                offset: continuation
            })
        );
        assert_eq!(
            parsed.path_at_offset(source.len() + 1),
            Err(NavigationError::OffsetOutOfBounds {
                offset: source.len() + 1,
                source_len: source.len(),
            })
        );
        assert_eq!(parsed.node_at_offset(source.len()).unwrap(), None);

        let escaped_key_offset = source.find("a\\nb").unwrap();
        let path = parsed.path_at_offset(escaped_key_offset).unwrap().unwrap();
        assert!(matches!(
            path.segments(),
            [AstPathSegment::ObjectMember { key, .. }] if key.decoded() == Some("a\nb")
        ));

        let missing_key_offset = source.find("missing").unwrap();
        assert!(matches!(
            parsed.node_at_offset(missing_key_offset).unwrap(),
            Some(NodeRef::Value(Value::Object(_)))
        ));
        assert!(
            parsed
                .path_at_offset(missing_key_offset)
                .unwrap()
                .unwrap()
                .is_empty()
        );

        let rootless = parse("");
        assert_eq!(rootless.node_at_offset(0).unwrap(), None);
        assert_eq!(rootless.path_at_offset(0).unwrap(), None);
    }
}
