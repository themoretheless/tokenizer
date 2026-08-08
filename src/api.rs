//! Convenient multi-language analysis API.
//!
//! # Mental model (what you already have)
//!
//! | Everyday name | Engine layer | This module | Advanced path |
//! |---------------|--------------|-------------|----------------|
//! | **Lexer / syntax tokens** | lossless split of the text into exact pieces | [`Source::syntax`] | `json::lex`, URL tokenize |
//! | **Parser / structure** | recovering tree (AST/CST) | typed path: `json::parse` | not type-erased here |
//! | **Highlight / semantic tokens** | colouring with structure context | [`Source::highlight`] | `json::tokenize` |
//! | **Errors / diagnostics** | soft problems without refusing highlight | [`Source::errors`] | lex/parse diagnostics |
//!
//! Incomplete editor input always produces tokens when the language can lex.
//! Diagnostics describe problems; they do not delete spans.
//!
//! # Quick start
//!
//! ```
//! use themoretheless_tokenizer::api::Source;
//!
//! let analysis = Source::new("json", r#"{"ok": true}"#)
//!     .dialect("strict")
//!     .highlight()
//!     .expect("json feature enabled");
//!
//! assert!(analysis.is_valid());
//! assert!(analysis.tokens().iter().any(|t| t.kind == "property"));
//! ```

use std::borrow::Cow;

use crate::{
    HostAnalysisOptions, HostDiagnostic, HostError, HostLanguage, HostToken, HostTokenization,
    InputLimits, TokenLayer, analyze_host, builtin_registry,
};

/// Fluent analysis request for one source buffer and one language.
///
/// This is the multi-language convenience surface. For a full typed JSON AST,
/// keep using [`crate::json::parse`].
#[derive(Debug, Clone)]
pub struct Source<'a> {
    language: Cow<'a, str>,
    source: &'a str,
    dialect: Cow<'a, str>,
    limits: InputLimits,
}

impl<'a> Source<'a> {
    /// Start analysis for `language` (`"json"`, `"url"`, …) over `source`.
    #[must_use]
    pub fn new(language: impl Into<Cow<'a, str>>, source: &'a str) -> Self {
        Self {
            language: language.into(),
            source,
            dialect: Cow::Borrowed("default"),
            limits: InputLimits::conservative(),
        }
    }

    /// Dialect / mode inside the language (`"strict"`, `"jsonc"`, `"default"`, …).
    #[must_use]
    pub fn dialect(mut self, dialect: impl Into<Cow<'a, str>>) -> Self {
        self.dialect = dialect.into();
        self
    }

    /// Override resource limits (bytes, tokens, diagnostics, depth).
    #[must_use]
    pub fn limits(mut self, limits: InputLimits) -> Self {
        self.limits = limits;
        self
    }

    /// Language id string (`json`, `url`, …).
    #[must_use]
    pub fn language(&self) -> &str {
        self.language.as_ref()
    }

    /// Borrowed source text.
    #[must_use]
    pub fn text(&self) -> &'a str {
        self.source
    }

    fn options(&self) -> HostAnalysisOptions {
        HostAnalysisOptions {
            dialect: Cow::Owned(self.dialect.as_ref().to_owned()),
            limits: self.limits,
        }
    }

    fn engine(&self) -> Result<&'static dyn HostLanguage, HostError> {
        builtin_registry()
            .get_str(self.language.as_ref())
            .ok_or_else(|| HostError::UnknownLanguage {
                language: self.language.as_ref().to_owned(),
            })
    }

    /// **Lexer layer**: exact syntax tokens (punctuation, strings, whitespace, …).
    ///
    /// Also called “syntax highlighting tokens” when kinds map to CSS classes.
    /// Spans cover the full source when the engine is lossless.
    pub fn syntax(&self) -> Result<Analysis, HostError> {
        self.run(TokenLayer::Syntax)
    }

    /// Alias of [`Self::syntax`] (lexer pass). Prefer this name when talking about lexing.
    pub fn lex(&self) -> Result<Analysis, HostError> {
        self.syntax()
    }

    /// **Highlight layer**: semantic / context-aware tokens (e.g. property vs string).
    ///
    /// For engines without a separate semantic pass this may equal [`Self::syntax`].
    pub fn highlight(&self) -> Result<Analysis, HostError> {
        self.run(TokenLayer::Semantic)
    }

    /// Alias of [`Self::highlight`] for users who say “tokenize the file”.
    ///
    /// Product verb “tokenize” means **semantic/highlight**, not the raw lexer.
    pub fn tokenize(&self) -> Result<Analysis, HostError> {
        self.highlight()
    }

    /// **Error finding**: diagnostics only (no token list materialisation beyond
    /// what the engine needs internally).
    pub fn errors(&self) -> Result<Vec<HostDiagnostic>, HostError> {
        let engine = self.engine()?;
        let opts = self.options();
        if opts.limits.exceeds_input_bytes(self.source.len()) {
            return Err(HostError::InputTooLarge {
                max: opts.limits.max_input_bytes,
                actual: self.source.len(),
            });
        }
        engine.diagnose(self.source, &opts)
    }

    /// Whether the buffer is free of error-severity diagnostics for the default
    /// (semantic) layer when available.
    pub fn is_valid(&self) -> Result<bool, HostError> {
        Ok(self.highlight()?.is_valid())
    }

    /// Run a chosen token layer and return tokens + diagnostics.
    pub fn run(&self, layer: TokenLayer) -> Result<Analysis, HostError> {
        let tokens = analyze_host(
            self.language.as_ref(),
            self.source,
            self.dialect.as_ref(),
            layer,
        )?;
        Ok(Analysis {
            language: self.language.as_ref().to_owned(),
            dialect: self.dialect.as_ref().to_owned(),
            layer,
            source_len: self.source.len(),
            inner: tokens,
        })
    }
}

/// Result of a convenience analysis pass (tokens + diagnostics).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Analysis {
    language: String,
    dialect: String,
    layer: TokenLayer,
    source_len: usize,
    inner: HostTokenization,
}

impl Analysis {
    #[must_use]
    pub fn language(&self) -> &str {
        &self.language
    }

    #[must_use]
    pub fn dialect(&self) -> &str {
        &self.dialect
    }

    #[must_use]
    pub fn layer(&self) -> TokenLayer {
        self.layer
    }

    #[must_use]
    pub fn source_bytes(&self) -> usize {
        self.source_len
    }

    /// True when the engine reported no structural/lexical errors for this pass.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.inner.valid
    }

    #[must_use]
    pub fn tokens(&self) -> &[HostToken] {
        &self.inner.tokens
    }

    #[must_use]
    pub fn diagnostics(&self) -> &[HostDiagnostic] {
        &self.inner.diagnostics
    }

    /// Token covering `offset` (UTF-8 byte), if any.
    #[must_use]
    pub fn token_at(&self, offset: usize) -> Option<&HostToken> {
        self.inner
            .tokens
            .iter()
            .find(|token| token.span.start <= offset && offset < token.span.end)
    }

    /// Diagnostics whose span contains `offset`.
    pub fn diagnostics_at(&self, offset: usize) -> impl Iterator<Item = &HostDiagnostic> {
        self.inner.diagnostics.iter().filter(move |diagnostic| {
            diagnostic.span.start <= offset && offset < diagnostic.span.end
                || (diagnostic.span.start == diagnostic.span.end && diagnostic.span.start == offset)
        })
    }

    /// Slice token text from the original source.
    #[must_use]
    pub fn token_text<'s>(&self, token: &HostToken, source: &'s str) -> Option<&'s str> {
        source.get(token.span.start..token.span.end)
    }

    /// Consume into the lower-level host DTO.
    #[must_use]
    pub fn into_host(self) -> HostTokenization {
        self.inner
    }
}

/// One-shot helpers matching everyday vocabulary.
pub mod quick {
    use super::*;

    /// Syntax (lexer) tokens for a language.
    pub fn syntax(language: &str, source: &str) -> Result<Analysis, HostError> {
        Source::new(language, source).syntax()
    }

    /// Semantic highlight tokens for a language.
    pub fn highlight(language: &str, source: &str) -> Result<Analysis, HostError> {
        Source::new(language, source).highlight()
    }

    /// Diagnostics only.
    pub fn errors(language: &str, source: &str) -> Result<Vec<HostDiagnostic>, HostError> {
        Source::new(language, source).errors()
    }

    /// JSON strict highlight (most common case).
    #[cfg(feature = "json")]
    pub fn highlight_json(source: &str) -> Result<Analysis, HostError> {
        Source::new("json", source).dialect("strict").highlight()
    }

    /// JSONC highlight.
    #[cfg(feature = "json")]
    pub fn highlight_jsonc(source: &str) -> Result<Analysis, HostError> {
        Source::new("json", source).dialect("jsonc").highlight()
    }

    /// URL highlight.
    #[cfg(feature = "url")]
    pub fn highlight_url(source: &str) -> Result<Analysis, HostError> {
        Source::new("url", source).highlight()
    }
}

/// Small prelude for application code.
pub mod prelude {
    pub use super::quick;
    pub use super::{Analysis, Source};
    pub use crate::{
        Diagnostic, HostDiagnostic, HostError, HostToken, LanguageId, Span, TokenLayer,
    };

    #[cfg(feature = "json")]
    pub use crate::json::{
        LexerOptions, ParseOptions, SyntaxKind, lex, lex_with, parse, parse_with, tokenize,
        tokenize_with,
    };
}

/// Glossary for docs and UI copy (Russian + English).
pub mod glossary {
    /// Lexer / лексический разбор: режет текст на токены без дерева.
    pub const LEXER: &str = "lexer";
    /// Parser / синтаксический разбор: строит структуру (AST/CST).
    pub const PARSER: &str = "parser";
    /// Syntax tokens / синтаксические токены: exact kinds from the lexer.
    pub const SYNTAX_TOKENS: &str = "syntax";
    /// Semantic / highlight tokens / подсветка: kinds with structural context.
    pub const HIGHLIGHT: &str = "highlight";
    /// Diagnostics / поиск ошибок: soft messages with spans.
    pub const DIAGNOSTICS: &str = "diagnostics";
    /// Span / диапазон: half-open UTF-8 byte offsets `[start, end)`.
    pub const SPAN: &str = "span";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "json")]
    #[test]
    fn highlight_json_in_three_lines() {
        let analysis = Source::new("json", r#"{"name":"Denis"}"#)
            .dialect("strict")
            .highlight()
            .unwrap();
        assert!(analysis.is_valid());
        assert!(analysis.tokens().iter().any(|t| t.kind == "property"));
        assert!(analysis.token_at(1).is_some());
    }

    #[cfg(feature = "json")]
    #[test]
    fn errors_only_on_invalid_json() {
        let diagnostics = Source::new("json", "{,}")
            .dialect("strict")
            .errors()
            .unwrap();
        assert!(!diagnostics.is_empty());
    }

    #[cfg(feature = "url")]
    #[test]
    fn url_syntax_and_highlight() {
        let source = "https://example.com/a?x=1";
        let syntax = Source::new("url", source).syntax().unwrap();
        let highlight = Source::new("url", source).highlight().unwrap();
        assert_eq!(syntax.tokens().len(), highlight.tokens().len());
        assert!(syntax.tokens().iter().any(|t| t.kind == "u-scheme"));
    }

    #[cfg(feature = "json")]
    #[test]
    fn quick_helpers() {
        let a = quick::highlight_json(r#"[1,2]"#).unwrap();
        assert!(a.is_valid());
        let b = quick::syntax("json", "null").unwrap();
        assert!(b.tokens().iter().any(|t| t.kind == "null"));
    }

    #[test]
    fn unknown_language_is_hard_error() {
        let err = Source::new("nope", "x").highlight().unwrap_err();
        assert!(matches!(err, HostError::UnknownLanguage { .. }));
    }

    #[test]
    fn language_id_constants_match_registry() {
        use crate::LanguageId;
        assert_eq!(LanguageId::JSON.as_str(), "json");
        assert_eq!(LanguageId::URL.as_str(), "url");
    }
}
