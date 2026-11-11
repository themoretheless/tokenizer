//! Zero-allocation lexical scanning primitives shared across language and format engines.

/// Returns `true` when `text` is a non-empty ASCII integer with an optional leading `-`.
#[inline]
#[must_use]
pub fn is_ascii_integer(text: &str) -> bool {
    let digits = text.strip_prefix('-').unwrap_or(text);
    !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit())
}

/// Returns `true` when `text` is a non-integer ASCII decimal/float literal
/// (optional sign, digits with at least one `.` and/or an `e`/`E` exponent).
#[inline]
#[must_use]
pub fn is_ascii_decimal(text: &str) -> bool {
    let body = text.strip_prefix(['-', '+']).unwrap_or(text);
    let (mantissa, exponent) = match body.split_once(['e', 'E']) {
        Some((mantissa, exponent)) => (mantissa, Some(exponent)),
        None => (body, None),
    };
    let mantissa_ok = if let Some((head, tail)) = mantissa.split_once('.') {
        !tail.contains('.')
            && (head.is_empty() || head.bytes().all(|byte| byte.is_ascii_digit()))
            && (tail.is_empty() || tail.bytes().all(|byte| byte.is_ascii_digit()))
            && !(head.is_empty() && tail.is_empty())
    } else {
        !mantissa.is_empty() && mantissa.bytes().all(|byte| byte.is_ascii_digit())
    };
    if !mantissa_ok || !(mantissa.contains('.') || exponent.is_some()) {
        return false;
    }
    let Some(exponent) = exponent else {
        return true;
    };
    let digits = exponent.strip_prefix(['-', '+']).unwrap_or(exponent);
    !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit())
}

/// Fast first-byte dispatch for multi-character C-family / scripting operators.
#[inline]
#[must_use]
pub fn match_common_operator(bytes: &[u8], i: usize) -> Option<usize> {
    let b0 = *bytes.get(i)?;
    let b1 = *bytes.get(i + 1)?;
    match b0 {
        b'.' if b1 == b'.' && bytes.get(i + 2) == Some(&b'.') => Some(3),
        b'<' => match b1 {
            b'<' if bytes.get(i + 2) == Some(&b'=') => Some(3),
            b'<' | b'=' => Some(2),
            _ => None,
        },
        b'>' => match b1 {
            b'>' if bytes.get(i + 2) == Some(&b'=') => Some(3),
            b'>' | b'=' => Some(2),
            _ => None,
        },
        b'=' => match b1 {
            b'=' if bytes.get(i + 2) == Some(&b'=') => Some(3),
            b'=' | b'>' => Some(2),
            _ => None,
        },
        b'!' if b1 == b'=' => {
            if bytes.get(i + 2) == Some(&b'=') {
                Some(3)
            } else {
                Some(2)
            }
        }
        b'?' if b1 == b'?' => {
            if bytes.get(i + 2) == Some(&b'=') {
                Some(3)
            } else {
                Some(2)
            }
        }
        b'*' => match b1 {
            b'*' if bytes.get(i + 2) == Some(&b'=') => Some(3),
            b'*' | b'=' => Some(2),
            _ => None,
        },
        b'&' if matches!(b1, b'&' | b'=') => Some(2),
        b'|' if matches!(b1, b'|' | b'=') => Some(2),
        b'+' if matches!(b1, b'+' | b'=') => Some(2),
        b'-' if matches!(b1, b'-' | b'=' | b'>') => Some(2),
        b'/' | b'%' | b'^' if b1 == b'=' => Some(2),
        b':' if b1 == b':' => Some(2),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_integers_and_decimals() {
        assert!(is_ascii_integer("0"));
        assert!(is_ascii_integer("-42"));
        assert!(!is_ascii_integer(""));
        assert!(!is_ascii_integer("-"));
        assert!(!is_ascii_integer("3.14"));

        assert!(is_ascii_decimal("3.14"));
        assert!(is_ascii_decimal("-0.5e+3"));
        assert!(is_ascii_decimal("1e5"));
        assert!(is_ascii_decimal(".5"));
        assert!(!is_ascii_decimal("."));
        assert!(!is_ascii_decimal("42"));
        assert!(!is_ascii_decimal("1.2.3"));
    }

    #[test]
    fn matches_all_multi_char_operators() {
        for op in [
            "...", "<<=", ">>=", "===", "!==", "??=", "**=", "<<", ">>", "<=", ">=", "==", "!=",
            "&&", "||", "**", "+=", "-=", "*=", "/=", "%=", "&=", "|=", "^=", "->", "=>", "::",
            "++", "--", "??",
        ] {
            assert_eq!(
                match_common_operator(op.as_bytes(), 0),
                Some(op.len()),
                "failed on {op}"
            );
        }
        assert_eq!(match_common_operator(b"+", 0), None);
        assert_eq!(match_common_operator(b"..x", 0), None);
    }
}
