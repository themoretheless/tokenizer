//! Shared primitives for themoretheless-tokenizer language plugins.
//!
//! Spans are UTF-8 byte offsets. Language engines store only [`Span`]; hosts
//! convert to line/column or UTF-16 at their boundary via [`LineIndex`].
//!
//! See `docs/plugin-api-design.md` in the repository for the full plugin
//! architecture contract.

#![forbid(unsafe_code)]

pub mod capabilities;
pub mod diagnostic;
pub mod host;
pub mod language;
pub mod limits;
pub mod lossless;
pub mod registry;
pub mod source;
pub mod span;

pub use capabilities::{Capabilities, Capability, CapabilityError};
pub use diagnostic::{Diagnostic, DiagnosticKind, Severity};
pub use host::{
    HostAnalysisOptions, HostDiagnostic, HostError, HostLanguage, HostSpan, HostToken,
    HostTokenization, TokenLayer,
};
pub use language::{DialectDescriptor, DialectId, LanguageDescriptor, LanguageId, LanguageKey};
pub use limits::{InputLimits, LimitExceeded};
pub use lossless::{LosslessViolation, verify_lossless_spans};
pub use registry::{LanguageRegistry, RegisterError, RegistryBuilder};
pub use source::{ColumnEncoding, LineColumn, LineIndex, PositionError};
pub use span::Span;
