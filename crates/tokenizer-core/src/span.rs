//! Half-open UTF-8 byte spans.

use std::ops::Range;

/// Half-open UTF-8 byte range in the source string.
///
/// This is the only position unit used inside language engines. Hosts convert
/// to UTF-16 or line/column via [`crate::LineIndex`] at their boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

impl Span {
    #[must_use]
    pub const fn new(start: usize, end: usize) -> Self {
        Self { start, end }
    }

    #[must_use]
    pub const fn len(self) -> usize {
        self.end.saturating_sub(self.start)
    }

    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.start == self.end
    }

    /// Returns this span as a standard half-open range.
    #[must_use]
    pub const fn range(self) -> Range<usize> {
        self.start..self.end
    }

    /// Whether the half-open span contains a byte offset.
    #[must_use]
    pub const fn contains(self, offset: usize) -> bool {
        self.start <= offset && offset < self.end
    }

    /// The smallest span covering both inputs.
    #[must_use]
    pub const fn cover(self, other: Self) -> Self {
        Self::new(
            if self.start < other.start {
                self.start
            } else {
                other.start
            },
            if self.end > other.end {
                self.end
            } else {
                other.end
            },
        )
    }

    /// Whether both endpoints can safely index the given UTF-8 source.
    #[must_use]
    pub fn is_valid_for(self, source: &str) -> bool {
        self.start <= self.end
            && self.end <= source.len()
            && source.is_char_boundary(self.start)
            && source.is_char_boundary(self.end)
    }

    /// Returns the covered source text when the span is valid.
    #[must_use]
    pub fn slice(self, source: &str) -> Option<&str> {
        source.get(self.range())
    }
}

impl From<Span> for Range<usize> {
    fn from(span: Span) -> Self {
        span.start..span.end
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cover_and_contains() {
        let a = Span::new(0, 3);
        let b = Span::new(2, 5);
        assert_eq!(a.cover(b), Span::new(0, 5));
        assert!(a.contains(2));
        assert!(!a.contains(3));
    }

    #[test]
    fn slice_respects_char_boundaries() {
        let source = "a😀b";
        let span = Span::new(1, "a😀".len());
        assert_eq!(span.slice(source), Some("😀"));
        assert!(!Span::new(1, 2).is_valid_for(source));
    }
}
