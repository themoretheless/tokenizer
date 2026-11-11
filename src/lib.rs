//! Dependency-free tokenization primitives used by Peregon.
//!
//! Spans are UTF-8 byte offsets. This makes [`Token::text`] safe and keeps the
//! crate independent from a particular editor protocol. Browser adapters can
//! convert them to UTF-16 code-unit offsets at their boundary.
//!
//! Shared plugin primitives live in [`themoretheless_tokenizer_core`] and are
//! re-exported here.

#![forbid(unsafe_code)]
#![doc = include_str!("../README.md")]

// ─── Format & Specialized Language Crates ────────────────────────────────────

#[cfg(feature = "json")]
pub use themoretheless_tokenizer_json as json;
#[cfg(feature = "json5")]
pub use themoretheless_tokenizer_json::{JSON5_ENGINE, Json5Host};
#[cfg(feature = "jsonl")]
pub use themoretheless_tokenizer_json::{JSONL_ENGINE, JsonlHost};

#[cfg(feature = "css")]
pub use themoretheless_tokenizer_css as css;
#[cfg(feature = "csv")]
pub use themoretheless_tokenizer_csv as csv;
#[cfg(feature = "edn")]
pub use themoretheless_tokenizer_edn as edn;
#[cfg(feature = "hcl")]
pub use themoretheless_tokenizer_hcl as hcl;
#[cfg(feature = "html")]
pub use themoretheless_tokenizer_html as html;
#[cfg(feature = "ics")]
pub use themoretheless_tokenizer_ics as ics;
#[cfg(feature = "ini")]
pub use themoretheless_tokenizer_ini as ini;
#[cfg(feature = "properties")]
pub use themoretheless_tokenizer_ini::{PROPERTIES_ENGINE, PropertiesHost};
#[cfg(feature = "logfmt")]
pub use themoretheless_tokenizer_logfmt as logfmt;
#[cfg(feature = "markdown")]
pub use themoretheless_tokenizer_markdown as markdown;
#[cfg(feature = "srt")]
pub use themoretheless_tokenizer_srt as srt;
#[cfg(feature = "vtt")]
pub use themoretheless_tokenizer_srt::{VTT_ENGINE, VttHost};
#[cfg(feature = "toml")]
pub use themoretheless_tokenizer_toml as toml;
#[cfg(feature = "url")]
pub use themoretheless_tokenizer_url as url;
#[cfg(feature = "xml")]
pub use themoretheless_tokenizer_xml as xml;
#[cfg(feature = "yaml")]
pub use themoretheless_tokenizer_yaml as yaml;

#[cfg(feature = "forma")]
pub use themoretheless_tokenizer_forma as forma;
#[cfg(feature = "rush")]
pub use themoretheless_tokenizer_rush as rush;

// ─── Consolidated Profiled Languages (`crates/tokenizer-languages`) ──────────

macro_rules! reexport_wave_languages {
    ($( $feat:literal => $name:ident ),* $(,)?) => {
        $(
            #[cfg(feature = $feat)]
            pub use themoretheless_tokenizer_languages::$name as $name;
        )*
    };
}

reexport_wave_languages! {
    "abap" => abap,
    "ada" => ada,
    "assembly" => assembly,
    "bash" => bash,
    "c" => c,
    "clojure" => clojure,
    "cobol" => cobol,
    "cpp" => cpp,
    "csharp" => csharp,
    "dart" => dart,
    "delphi" => delphi,
    "dlang" => dlang,
    "elixir" => elixir,
    "erlang" => erlang,
    "fortran" => fortran,
    "fsharp" => fsharp,
    "go" => go,
    "graphql" => graphql,
    "groovy" => groovy,
    "haskell" => haskell,
    "java" => java,
    "javascript" => javascript,
    "julia" => julia,
    "kotlin" => kotlin,
    "lisp" => lisp,
    "lua" => lua,
    "matlab" => matlab,
    "mongo" => mongo,
    "nim" => nim,
    "objectivec" => objectivec,
    "ocaml" => ocaml,
    "perl" => perl,
    "php" => php,
    "powershell" => powershell,
    "prolog" => prolog,
    "python" => python,
    "r" => r,
    "ruby" => ruby,
    "rust" => rust,
    "scala" => scala,
    "scheme" => scheme,
    "solidity" => solidity,
    "sql" => sql,
    "swift" => swift,
    "typescript" => typescript,
    "verilog" => verilog,
    "vhdl" => vhdl,
    "visualbasic" => visualbasic,
    "zig" => zig,
}

// ─── Facade Modules & Re-exports ─────────────────────────────────────────────

pub mod api;
mod legacy_json;
pub mod plugins;

#[cfg(feature = "web-bridge")]
mod rush_bridge;
#[cfg(feature = "web-bridge")]
#[doc(hidden)]
pub mod web_bridge;

pub use api::{Analysis, Source};
pub use legacy_json::{JsonTokenizer, Token, TokenKind, Tokenization, Tokenizer, tokenize_json};
pub use plugins::{analyze_host, analyze_host_with_options, builtin_registry, register_builtins};

// Plugin core (spans, diagnostics, registry, host facade).
pub use themoretheless_tokenizer_core as core;
pub use themoretheless_tokenizer_core::{
    Capabilities, Capability, CapabilityError, ColumnEncoding, Diagnostic, DiagnosticKind,
    DialectDescriptor, DialectId, HostAnalysisOptions, HostDiagnostic, HostError, HostLanguage,
    HostSpan, HostToken, HostTokenization, InputLimits, LanguageDescriptor, LanguageId,
    LanguageKey, LanguageRegistry, LimitExceeded, LineColumn, LineIndex, LosslessViolation,
    PositionError, RegisterError, RegistryBuilder, Severity, Span, TokenLayer,
    verify_lossless_spans,
};

#[cfg(feature = "url")]
pub use themoretheless_tokenizer_url::{
    UrlKind, UrlToken, UrlTokenization, tokenize as tokenize_url,
    tokenize_and_validate as tokenize_url_validated, validate as validate_url,
};

/// Compatibility alias for the previous `source` module path.
pub mod source {
    pub use themoretheless_tokenizer_core::{ColumnEncoding, LineColumn, LineIndex, PositionError};
}
