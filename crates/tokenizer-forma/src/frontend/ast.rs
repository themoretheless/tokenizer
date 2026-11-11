//! Transport AST helpers, pure-function table, and UTF-16 expression span tables.

use serde_json::{Value as V, json};

pub(super) fn empty_node(kind: &str, start: usize) -> V {
    json!({"type":kind,"start":start,"props":{},"propertyRanges":{},"statementRanges":{},"bindings":{},"events":{},"children":[],"slots":{},"matches":[],"forward":[]})
}
pub(super) fn object() -> V {
    json!({})
}
pub(super) fn array() -> V {
    json!([])
}
pub(super) fn push(v: &mut V, key: &str, item: V) {
    v[key].as_array_mut().unwrap().push(item)
}
pub(super) fn has(v: &V, key: &str) -> bool {
    v.as_object().is_some_and(|o| o.contains_key(key))
}
pub(super) fn nonempty(v: &V) -> bool {
    v.as_array().is_some_and(|a| !a.is_empty()) || v.as_object().is_some_and(|o| !o.is_empty())
}
#[inline]
pub(super) fn name(s: &str) -> bool {
    let [first, rest @ ..] = s.as_bytes() else {
        return false;
    };
    (first.is_ascii_alphabetic() || *first == b'_')
        && rest.iter().all(|b| b.is_ascii_alphanumeric() || *b == b'_')
}
#[inline]
pub(super) fn path(s: &str) -> bool {
    let bytes = s.as_bytes();
    if bytes.is_empty() {
        return false;
    }
    let mut expect_start = true;
    for &b in bytes {
        if expect_start {
            if !(b.is_ascii_alphabetic() || b == b'_') {
                return false;
            }
            expect_start = false;
        } else if b == b'.' {
            expect_start = true;
        } else if !(b.is_ascii_alphanumeric() || b == b'_') {
            return false;
        }
    }
    !expect_start
}
#[inline]
pub(crate) fn is_pure_function(s: &str) -> bool {
    matches!(
        s,
        "min"
            | "max"
            | "clamp"
            | "abs"
            | "round"
            | "floor"
            | "ceil"
            | "pow"
            | "sqrt"
            | "sign"
            | "lerp"
            | "remap"
            | "avg"
            | "len"
            | "String"
            | "Number"
            | "Bool"
            | "trim"
            | "upper"
            | "lower"
            | "capitalize"
            | "padStart"
            | "padEnd"
            | "truncate"
            | "repeat"
            | "contains"
            | "includes"
            | "indexOf"
            | "startsWith"
            | "endsWith"
            | "replace"
            | "slice"
            | "split"
            | "join"
            | "reverse"
            | "take"
            | "skip"
            | "sum"
            | "range"
            | "formatNumber"
            | "sortBy"
            | "pluck"
            | "count"
            | "any"
            | "all"
            | "alpha"
            | "mix"
            | "lighten"
            | "darken"
            | "contrast"
            | "invert"
            | "saturate"
            | "desaturate"
            | "unique"
            | "find"
            | "compact"
            | "flatten"
            | "chunk"
            | "zip"
            | "first"
            | "last"
            | "groupBy"
            | "keys"
            | "values"
            | "entries"
            | "fromEntries"
            | "merge"
            | "pick"
            | "omit"
            | "isEmpty"
            | "coalesce"
            | "plural"
            | "isValid"
    )
}
#[derive(Debug, Clone)]
pub(crate) struct ExpressionSpan {
    pub value: V,
    pub from: usize,
    pub to: usize,
}
#[derive(Debug, Clone, Default)]
pub(crate) struct SpanTable {
    pub entries: Vec<ExpressionSpan>,
}
