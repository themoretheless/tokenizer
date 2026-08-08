//! Object-safe host facade for playground, WASM, and editor adapters.

use std::borrow::Cow;

use crate::{
    capabilities::{Capabilities, CapabilityError},
    language::{DialectId, LanguageDescriptor, LanguageId},
    limits::InputLimits,
    span::Span,
};

/// Which token layer a host requests.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum TokenLayer {
    /// Exact syntax / lexer kinds.
    Syntax,
    /// Parser-aware semantic highlight kinds.
    Semantic,
}

impl TokenLayer {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Syntax => "syntax",
            Self::Semantic => "semantic",
        }
    }

    /// Parse a wire layer string (`syntax` / `semantic`).
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "syntax" => Some(Self::Syntax),
            "semantic" => Some(Self::Semantic),
            _ => None,
        }
    }
}

/// Host analysis options (dialect + limits).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostAnalysisOptions {
    pub dialect: Cow<'static, str>,
    pub limits: InputLimits,
}

impl HostAnalysisOptions {
    #[must_use]
    pub fn new(dialect: impl Into<Cow<'static, str>>) -> Self {
        Self {
            dialect: dialect.into(),
            limits: InputLimits::conservative(),
        }
    }

    #[must_use]
    pub fn with_limits(mut self, limits: InputLimits) -> Self {
        self.limits = limits;
        self
    }
}

impl Default for HostAnalysisOptions {
    fn default() -> Self {
        Self {
            dialect: Cow::Borrowed("default"),
            limits: InputLimits::conservative(),
        }
    }
}

/// UTF-8 span DTO for the host wire format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct HostSpan {
    pub start: usize,
    pub end: usize,
}

impl From<Span> for HostSpan {
    fn from(span: Span) -> Self {
        Self {
            start: span.start,
            end: span.end,
        }
    }
}

impl From<HostSpan> for Span {
    fn from(span: HostSpan) -> Self {
        Self::new(span.start, span.end)
    }
}

/// One host-facing token with a kebab-case kind name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostToken {
    pub kind: Cow<'static, str>,
    pub span: HostSpan,
    pub error: bool,
}

/// Host-facing diagnostic DTO.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostDiagnostic {
    pub code: Cow<'static, str>,
    pub message: Cow<'static, str>,
    pub span: HostSpan,
    pub severity: crate::diagnostic::Severity,
}

impl HostDiagnostic {
    #[must_use]
    pub fn from_diagnostic(diagnostic: crate::diagnostic::Diagnostic) -> Self {
        Self {
            code: Cow::Borrowed(diagnostic.code),
            message: Cow::Borrowed(diagnostic.message),
            span: diagnostic.span.into(),
            severity: crate::diagnostic::Severity::Error,
        }
    }
}

/// Token stream returned to a host.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct HostTokenization {
    pub tokens: Vec<HostToken>,
    pub diagnostics: Vec<HostDiagnostic>,
    pub valid: bool,
}

/// Why a host request failed (protocol / capability), not a soft parse error.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum HostError {
    UnsupportedCapability { capability: &'static str },
    Capability(CapabilityError),
    UnknownDialect { dialect: String },
    UnknownLanguage { language: String },
    InputTooLarge { max: usize, actual: usize },
    InvalidOffset { offset: usize, source_len: usize },
    InvalidOptions { message: String },
}

impl core::fmt::Display for HostError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::UnsupportedCapability { capability } => {
                write!(f, "unsupported capability `{capability}`")
            }
            Self::Capability(error) => write!(f, "{error}"),
            Self::UnknownDialect { dialect } => write!(f, "unknown dialect `{dialect}`"),
            Self::UnknownLanguage { language } => write!(f, "unknown language `{language}`"),
            Self::InputTooLarge { max, actual } => {
                write!(f, "input is {actual} bytes; max is {max}")
            }
            Self::InvalidOffset { offset, source_len } => write!(
                f,
                "offset {offset} is outside source of length {source_len}"
            ),
            Self::InvalidOptions { message } => write!(f, "invalid options: {message}"),
        }
    }
}

impl std::error::Error for HostError {}

impl From<CapabilityError> for HostError {
    fn from(value: CapabilityError) -> Self {
        Self::Capability(value)
    }
}

/// Object-safe multi-language host API.
///
/// Typed language crates implement this via a thin adapter. Hosts never see
/// language-specific AST types through this trait.
pub trait HostLanguage: Send + Sync + 'static {
    fn descriptor(&self) -> &'static LanguageDescriptor;

    fn id(&self) -> LanguageId {
        self.descriptor().language
    }

    fn capabilities(&self) -> Capabilities {
        self.descriptor().capabilities
    }

    fn dialects(&self) -> &'static [crate::language::DialectDescriptor] {
        self.descriptor().dialects
    }

    /// Lossless syntax (lexer) tokens.
    fn lex(&self, source: &str, opts: &HostAnalysisOptions) -> Result<HostTokenization, HostError>;

    /// Parser-aware semantic highlight tokens when capable.
    fn semantic_tokens(
        &self,
        source: &str,
        opts: &HostAnalysisOptions,
    ) -> Result<HostTokenization, HostError>;

    /// Diagnostics without requiring a specific token layer.
    fn diagnose(
        &self,
        source: &str,
        opts: &HostAnalysisOptions,
    ) -> Result<Vec<HostDiagnostic>, HostError>;

    /// Convenience: run the requested layer (`syntax` or `semantic`).
    fn tokenize_layer(
        &self,
        source: &str,
        opts: &HostAnalysisOptions,
        layer: TokenLayer,
    ) -> Result<HostTokenization, HostError> {
        match layer {
            TokenLayer::Syntax => self.lex(source, opts),
            TokenLayer::Semantic => self.semantic_tokens(source, opts),
        }
    }

    /// Ensure advertised capabilities include `required`.
    fn require(&self, required: Capabilities) -> Result<(), HostError> {
        let available = self.capabilities();
        if available.contains(required) {
            Ok(())
        } else {
            Err(HostError::Capability(CapabilityError {
                language_id: self.id(),
                required,
                available,
            }))
        }
    }

    /// Resolve dialect string against this language's descriptor.
    fn resolve_dialect(&self, raw: &str) -> Result<DialectId, HostError> {
        self.descriptor()
            .resolve_dialect(raw)
            .ok_or_else(|| HostError::UnknownDialect {
                dialect: raw.to_owned(),
            })
    }
}
