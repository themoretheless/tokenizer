//! Source-position utilities for editor and diagnostic integrations.

use std::{error::Error, fmt};

/// The unit used for a [`LineColumn::column`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ColumnEncoding {
    /// UTF-8 bytes, matching [`crate::Span`].
    Utf8Bytes,
    /// Unicode scalar values (`char`s), not grapheme clusters.
    UnicodeScalars,
    /// UTF-16 code units, as used by the Language Server Protocol and browsers.
    Utf16CodeUnits,
}

/// A zero-based line and column.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LineColumn {
    pub line: u32,
    pub column: u32,
}

impl LineColumn {
    #[must_use]
    pub const fn new(line: u32, column: u32) -> Self {
        Self { line, column }
    }
}

/// Why a source-position conversion failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum PositionError {
    OffsetOutOfBounds,
    NotCharBoundary,
    InsideCrLf,
    LineOutOfBounds,
    ColumnOutOfBounds,
}

impl fmt::Display for PositionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::OffsetOutOfBounds => "the byte offset is outside the source",
            Self::NotCharBoundary => "the byte offset is not a UTF-8 character boundary",
            Self::InsideCrLf => "the byte offset points between CR and LF",
            Self::LineOutOfBounds => "the line is outside the source",
            Self::ColumnOutOfBounds => "the column is outside the line or splits a character",
        };
        formatter.write_str(message)
    }
}

impl Error for PositionError {}

/// Precomputed line starts for fast byte/line-column conversion.
///
/// `LF`, `CRLF`, and lone `CR` are all treated as one line break. An offset
/// between the two bytes of a `CRLF` sequence is deliberately rejected because
/// it has no unique line/column round trip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineIndex<'source> {
    source: &'source str,
    line_starts: Vec<usize>,
    line_ends: Vec<usize>,
}

impl<'source> LineIndex<'source> {
    #[must_use]
    pub fn new(source: &'source str) -> Self {
        let bytes = source.as_bytes();
        let mut line_starts = vec![0];
        let mut line_ends = Vec::new();
        let mut cursor = 0;

        while cursor < bytes.len() {
            match bytes[cursor] {
                b'\r' => {
                    line_ends.push(cursor);
                    cursor += 1;
                    if bytes.get(cursor) == Some(&b'\n') {
                        cursor += 1;
                    }
                    line_starts.push(cursor);
                }
                b'\n' => {
                    line_ends.push(cursor);
                    cursor += 1;
                    line_starts.push(cursor);
                }
                _ => {
                    cursor += 1;
                }
            }
        }
        line_ends.push(source.len());

        Self {
            source,
            line_starts,
            line_ends,
        }
    }

    #[must_use]
    pub fn line_count(&self) -> usize {
        self.line_starts.len()
    }

    #[must_use]
    pub fn line_start(&self, line: usize) -> Option<usize> {
        self.line_starts.get(line).copied()
    }

    #[must_use]
    pub fn line_end(&self, line: usize) -> Option<usize> {
        self.line_ends.get(line).copied()
    }

    pub fn line_column(
        &self,
        offset: usize,
        encoding: ColumnEncoding,
    ) -> Result<LineColumn, PositionError> {
        let source = self.source;
        if offset > source.len() {
            return Err(PositionError::OffsetOutOfBounds);
        }
        if !source.is_char_boundary(offset) {
            return Err(PositionError::NotCharBoundary);
        }

        let line = self.line_starts.partition_point(|start| *start <= offset) - 1;
        let start = self.line_starts[line];
        let end = self.line_ends[line];
        if offset > end {
            return Err(PositionError::InsideCrLf);
        }
        let prefix = &source[start..offset];
        let column = measure(prefix, encoding);
        let line = u32::try_from(line).map_err(|_| PositionError::LineOutOfBounds)?;
        let column = u32::try_from(column).map_err(|_| PositionError::ColumnOutOfBounds)?;
        Ok(LineColumn { line, column })
    }

    pub fn offset(
        &self,
        position: LineColumn,
        encoding: ColumnEncoding,
    ) -> Result<usize, PositionError> {
        let source = self.source;
        let line = usize::try_from(position.line).map_err(|_| PositionError::LineOutOfBounds)?;
        let start = *self
            .line_starts
            .get(line)
            .ok_or(PositionError::LineOutOfBounds)?;
        let end = self.line_ends[line];
        let text = &source[start..end];
        let wanted =
            usize::try_from(position.column).map_err(|_| PositionError::ColumnOutOfBounds)?;

        match encoding {
            ColumnEncoding::Utf8Bytes => {
                let offset = start
                    .checked_add(wanted)
                    .ok_or(PositionError::ColumnOutOfBounds)?;
                if offset > end || !source.is_char_boundary(offset) {
                    Err(PositionError::ColumnOutOfBounds)
                } else {
                    Ok(offset)
                }
            }
            ColumnEncoding::UnicodeScalars | ColumnEncoding::Utf16CodeUnits => {
                let mut measured = 0;
                for (relative, character) in text.char_indices() {
                    if measured == wanted {
                        return Ok(start + relative);
                    }
                    measured += match encoding {
                        ColumnEncoding::UnicodeScalars => 1,
                        ColumnEncoding::Utf16CodeUnits => character.len_utf16(),
                        ColumnEncoding::Utf8Bytes => unreachable!(),
                    };
                    if measured > wanted {
                        return Err(PositionError::ColumnOutOfBounds);
                    }
                }
                if measured == wanted {
                    Ok(end)
                } else {
                    Err(PositionError::ColumnOutOfBounds)
                }
            }
        }
    }

    #[must_use]
    pub const fn source(&self) -> &'source str {
        self.source
    }
}

fn measure(text: &str, encoding: ColumnEncoding) -> usize {
    match encoding {
        ColumnEncoding::Utf8Bytes => text.len(),
        ColumnEncoding::UnicodeScalars => text.chars().count(),
        ColumnEncoding::Utf16CodeUnits => text.encode_utf16().count(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indexes_all_common_line_endings() {
        let source = "a\r\nb\nc\rd";
        let index = LineIndex::new(source);
        assert_eq!(index.line_count(), 4);
        assert_eq!(index.line_starts, vec![0, 3, 5, 7]);
        assert_eq!(index.line_ends, vec![1, 4, 6, 8]);
        assert_eq!(
            index.line_column(3, ColumnEncoding::Utf8Bytes),
            Ok(LineColumn::new(1, 0))
        );
        assert_eq!(
            index.line_column(2, ColumnEncoding::Utf8Bytes),
            Err(PositionError::InsideCrLf)
        );
    }

    #[test]
    fn converts_unicode_columns_in_each_encoding() {
        let source = "a😀é\nМосква";
        let index = LineIndex::new(source);
        let after_emoji = "a😀".len();
        assert_eq!(
            index.line_column(after_emoji, ColumnEncoding::Utf8Bytes),
            Ok(LineColumn::new(0, 5))
        );
        assert_eq!(
            index.line_column(after_emoji, ColumnEncoding::UnicodeScalars),
            Ok(LineColumn::new(0, 2))
        );
        assert_eq!(
            index.line_column(after_emoji, ColumnEncoding::Utf16CodeUnits),
            Ok(LineColumn::new(0, 3))
        );
    }

    #[test]
    fn valid_positions_round_trip() {
        let source = "😀x\r\nyé";
        let index = LineIndex::new(source);
        for encoding in [
            ColumnEncoding::Utf8Bytes,
            ColumnEncoding::UnicodeScalars,
            ColumnEncoding::Utf16CodeUnits,
        ] {
            for offset in (0..=source.len()).filter(|offset| source.is_char_boundary(*offset)) {
                if let Ok(position) = index.line_column(offset, encoding) {
                    assert_eq!(index.offset(position, encoding), Ok(offset));
                }
            }
        }
    }

    #[test]
    fn rejects_split_characters_and_utf16_surrogates() {
        let source = "😀";
        let index = LineIndex::new(source);
        assert_eq!(
            index.line_column(1, ColumnEncoding::Utf8Bytes),
            Err(PositionError::NotCharBoundary)
        );
        assert_eq!(
            index.offset(LineColumn::new(0, 1), ColumnEncoding::Utf16CodeUnits),
            Err(PositionError::ColumnOutOfBounds)
        );
    }

    #[test]
    fn retains_the_source_it_indexes() {
        let source = String::from("a\nb");
        let index = LineIndex::new(&source);
        assert_eq!(index.source(), source);
    }
}
