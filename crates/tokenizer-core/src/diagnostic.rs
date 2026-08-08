//! Shared diagnostic shape for language plugins and the host facade.

use crate::Span;

/// How serious a diagnostic is for the host UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum Severity {
    #[default]
    Error,
    Warning,
    Info,
    Hint,
}

/// A source-attached diagnostic.
///
/// Field layout stays compatible with the pre-plugin `Diagnostic` used by URL
/// and the legacy JSON highlighter: `code` and `message` are static strings.
/// Host DTOs may attach [`Severity`] separately when needed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Diagnostic {
    pub span: Span,
    pub code: &'static str,
    pub message: &'static str,
}

impl Diagnostic {
    #[must_use]
    pub const fn new(span: Span, code: &'static str, message: &'static str) -> Self {
        Self {
            span,
            code,
            message,
        }
    }
}

/// Language-local diagnostic kind that maps to a stable code.
pub trait DiagnosticKind: Copy {
    fn code(self) -> &'static str;
    fn message(self) -> &'static str;
    fn severity(self) -> Severity {
        Severity::Error
    }

    fn to_diagnostic(self, span: Span) -> Diagnostic {
        Diagnostic {
            span,
            code: self.code(),
            message: self.message(),
        }
    }
}
