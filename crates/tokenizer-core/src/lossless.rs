//! Helpers for the lossless token-span contract.

use crate::Span;

/// Why a token stream is not a lossless cover of `source`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum LosslessViolation {
    EmptyToken {
        index: usize,
    },
    Gap {
        index: usize,
        expected_start: usize,
        actual_start: usize,
    },
    Overlap {
        index: usize,
        previous_end: usize,
        actual_start: usize,
    },
    InvalidSpan {
        index: usize,
        span: Span,
    },
    Incomplete {
        covered_end: usize,
        source_len: usize,
    },
}

impl core::fmt::Display for LosslessViolation {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::EmptyToken { index } => write!(f, "token {index} has an empty span"),
            Self::Gap {
                index,
                expected_start,
                actual_start,
            } => write!(
                f,
                "gap before token {index}: expected start {expected_start}, got {actual_start}"
            ),
            Self::Overlap {
                index,
                previous_end,
                actual_start,
            } => write!(
                f,
                "overlap at token {index}: previous end {previous_end}, start {actual_start}"
            ),
            Self::InvalidSpan { index, span } => {
                write!(
                    f,
                    "token {index} has invalid span {}..{}",
                    span.start, span.end
                )
            }
            Self::Incomplete {
                covered_end,
                source_len,
            } => write!(
                f,
                "tokens cover up to {covered_end}, source length is {source_len}"
            ),
        }
    }
}

impl std::error::Error for LosslessViolation {}

/// Verify that ordered spans form a lossless partition of `source`.
///
/// Empty sources may have zero tokens. Non-empty sources require contiguous
/// non-empty spans from `0` to `source.len()`.
pub fn verify_lossless_spans<I>(source: &str, spans: I) -> Result<(), LosslessViolation>
where
    I: IntoIterator<Item = Span>,
{
    let mut cursor = 0usize;
    let mut saw_token = false;

    for (index, span) in spans.into_iter().enumerate() {
        if !span.is_valid_for(source) {
            return Err(LosslessViolation::InvalidSpan { index, span });
        }
        if span.is_empty() {
            return Err(LosslessViolation::EmptyToken { index });
        }
        if span.start > cursor {
            return Err(LosslessViolation::Gap {
                index,
                expected_start: cursor,
                actual_start: span.start,
            });
        }
        if span.start < cursor {
            return Err(LosslessViolation::Overlap {
                index,
                previous_end: cursor,
                actual_start: span.start,
            });
        }
        cursor = span.end;
        saw_token = true;
    }

    if source.is_empty() {
        return Ok(());
    }
    if !saw_token || cursor != source.len() {
        return Err(LosslessViolation::Incomplete {
            covered_end: cursor,
            source_len: source.len(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_full_cover() {
        let source = "abc";
        let spans = [Span::new(0, 1), Span::new(1, 3)];
        assert!(verify_lossless_spans(source, spans).is_ok());
    }

    #[test]
    fn rejects_gap() {
        let source = "abc";
        let spans = [Span::new(0, 1), Span::new(2, 3)];
        assert!(matches!(
            verify_lossless_spans(source, spans),
            Err(LosslessViolation::Gap { .. })
        ));
    }
}
