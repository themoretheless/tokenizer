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
pub mod family;
pub mod fullkit;
pub mod host;
pub mod langkit;
pub mod language;
pub mod limits;
pub mod lossless;
pub mod markup_full;
pub mod plugin_host;
pub mod registry;
pub mod source;
pub mod span;

pub use capabilities::{Capabilities, Capability, CapabilityError};
pub use diagnostic::{Diagnostic, DiagnosticKind, Severity};
pub use family::{FORMAT_IDS, Family, NEXT20_IDS, Preset, TOP20_IDS, presets_of};
pub use fullkit::{
    Block, Expr, FULL_ENGINE_CAPS, FullProfile, Item, LexToken, Lexed, LitKind, Module, Parse,
    SemanticToken, SemanticTokenization, Stmt, SyntaxKind, analyze_full_host, lex_full,
    lex_to_host, parse_full, semantic_full,
};
pub use host::{
    HostAnalysisOptions, HostDiagnostic, HostError, HostLanguage, HostSpan, HostToken,
    HostTokenization, TokenLayer,
};
pub use langkit::{
    CLikeProfile, Highlighted, HlToken, IdentStyle, StringStyle, highlight_c_like, highlight_css,
    highlight_markdown, highlight_markup, highlight_toml, highlight_yaml,
};
pub use language::{DialectDescriptor, DialectId, LanguageDescriptor, LanguageId, LanguageKey};
pub use limits::{InputLimits, LimitExceeded};
pub use lossless::{LosslessViolation, verify_lossless_spans};
pub use markup_full::{
    MarkupAttr, MarkupDoc, MarkupNode, MarkupParse, markup_to_host, parse_markup,
};
pub use plugin_host::{
    DEFAULT_DIALECTS, FULL_CAPS, HIGHLIGHT_CAPS, full_descriptor, highlight_descriptor,
    host_diagnostics, host_tokenization, language_descriptor, require_default_dialect,
    run_diagnose_host, run_highlight_host,
};
pub use registry::{LanguageRegistry, RegisterError, RegistryBuilder};
pub use source::{ColumnEncoding, LineColumn, LineIndex, PositionError};
pub use span::Span;
