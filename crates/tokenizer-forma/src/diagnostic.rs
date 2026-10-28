//! Structured diagnostics shared by the evaluator and contract semantics.
//! `code` is a stable kebab-case identifier; `span` is an optional UTF-16
//! offset, as in `crate::frontend::Error`, and is set where the position is
//! known. `Display` prints only the message, so hosts that consumed the
//! previous string errors keep working unchanged.
use serde_json::{Value as V, json};
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub code: &'static str,
    pub message: String,
    pub span: Option<usize>,
}
impl Diagnostic {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            span: None,
        }
    }
    /// Fill the span from a known source position unless already set.
    pub fn or_span(mut self, span: Option<usize>) -> Self {
        if self.span.is_none() {
            self.span = span;
        }
        self
    }
    /// JSON form for the Studio/WASM bridge: the previous `message` string is
    /// preserved; `code` and `span` are additive fields.
    pub fn to_json(&self) -> V {
        match self.span {
            Some(span) => json!({"code":self.code,"message":self.message,"span":span}),
            None => json!({"code":self.code,"message":self.message}),
        }
    }
}
impl std::fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}
impl std::error::Error for Diagnostic {}
/// Host/resolver failures arrive as plain strings; they get a generic code.
impl From<String> for Diagnostic {
    fn from(message: String) -> Self {
        Self::new("host-error", message)
    }
}
impl From<&str> for Diagnostic {
    fn from(message: &str) -> Self {
        Self::new("host-error", message)
    }
}
/// Parser errors gain a stable code and keep their UTF-16 offset.
impl From<crate::frontend::Error> for Diagnostic {
    fn from(e: crate::frontend::Error) -> Self {
        Self {
            code: "syntax-error",
            message: e.message,
            span: Some(e.offset),
        }
    }
}
