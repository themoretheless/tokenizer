//! Table-driven [`HostLanguage`] adapter and declarative engine macro.

use crate::{
    Capabilities, Diagnostic, HostAnalysisOptions, HostDiagnostic, HostError, HostLanguage,
    HostTokenization, LanguageDescriptor,
};

use super::{
    ast::Parse,
    lexer::{Lexed, lex_full_with_lookup, lex_to_host},
    lookup::KeywordLookup,
    parser::parse_full_with_lookup,
    profile::FullProfile,
    semantic::{SemanticTokenization, semantic_full_owned},
};

/// Full engine capability bits. `VALIDATE` is deliberately absent: what this
/// pipeline can prove about a document is bracket balance and closed literals,
/// not that the language's full grammar was followed.
pub const FULL_ENGINE_CAPS: Capabilities = Capabilities::LEX
    .union(Capabilities::PARSE)
    .union(Capabilities::SEMANTIC);

/// Table-driven host engine for a profiled wave language.
#[derive(Debug, Clone, Copy)]
pub struct FullEngine {
    descriptor: &'static LanguageDescriptor,
    profile: &'static FullProfile,
    lookup: &'static KeywordLookup,
}

impl FullEngine {
    #[must_use]
    pub const fn new(
        descriptor: &'static LanguageDescriptor,
        profile: &'static FullProfile,
        lookup: &'static KeywordLookup,
    ) -> Self {
        Self {
            descriptor,
            profile,
            lookup,
        }
    }

    #[must_use]
    pub const fn profile(&self) -> &'static FullProfile {
        self.profile
    }

    #[must_use]
    pub fn lex_raw(&self, source: &str) -> Lexed {
        lex_full_with_lookup(source, self.profile, Some(self.lookup))
    }

    #[must_use]
    pub fn parse_raw<'s>(&self, source: &'s str) -> Parse<'s> {
        parse_full_with_lookup(source, self.profile, Some(self.lookup))
    }

    #[must_use]
    pub fn tokenize_raw(&self, source: &str) -> SemanticTokenization {
        semantic_full_owned(self.parse_raw(source))
    }

    #[must_use]
    pub fn validate_raw(&self, source: &str) -> Vec<Diagnostic> {
        self.lex_raw(source).diagnostics
    }
}

impl HostLanguage for FullEngine {
    fn descriptor(&self) -> &'static LanguageDescriptor {
        self.descriptor
    }

    fn lex(&self, source: &str, opts: &HostAnalysisOptions) -> Result<HostTokenization, HostError> {
        let _ = self.check_entry(source, opts)?;
        Ok(lex_to_host(self.lex_raw(source)))
    }

    fn semantic_tokens(
        &self,
        source: &str,
        opts: &HostAnalysisOptions,
    ) -> Result<HostTokenization, HostError> {
        let _ = self.check_entry(source, opts)?;
        Ok(self.tokenize_raw(source).to_host())
    }

    fn diagnose(
        &self,
        source: &str,
        opts: &HostAnalysisOptions,
    ) -> Result<Vec<HostDiagnostic>, HostError> {
        let _ = self.check_entry(source, opts)?;
        Ok(self
            .validate_raw(source)
            .into_iter()
            .map(HostDiagnostic::from_diagnostic)
            .collect())
    }
}

/// Declare a complete profiled wave-language module from its descriptor metadata and [`FullProfile`].
#[macro_export]
macro_rules! define_full_engine {
    (
        id: $id:expr,
        name: $name:expr,
        aliases: $aliases:expr,
        extensions: $extensions:expr,
        mime_types: $mime_types:expr,
        profile: $profile:expr $(,)?
    ) => {
        pub const LANGUAGE_ID: $crate::LanguageId = $id;

        pub static PROFILE: $crate::FullProfile = $profile;

        pub static LOOKUP: $crate::KeywordLookup = $crate::KeywordLookup::from_profile(&PROFILE);

        pub static DESCRIPTOR: $crate::LanguageDescriptor = $crate::full_descriptor(
            LANGUAGE_ID,
            $name,
            $aliases,
            $extensions,
            $mime_types,
            env!("CARGO_PKG_VERSION"),
        );

        pub type Host = $crate::FullEngine;

        pub static ENGINE: $crate::FullEngine =
            $crate::FullEngine::new(&DESCRIPTOR, &PROFILE, &LOOKUP);

        /// Lossless lexer.
        #[must_use]
        pub fn lex(source: &str) -> $crate::Lexed {
            ENGINE.lex_raw(source)
        }

        /// Recovering parse with borrowing AST.
        #[must_use]
        pub fn parse(source: &str) -> $crate::Parse<'_> {
            ENGINE.parse_raw(source)
        }

        /// Parser-aware semantic tokens.
        #[must_use]
        pub fn tokenize(source: &str) -> $crate::SemanticTokenization {
            ENGINE.tokenize_raw(source)
        }

        /// Diagnostics from the full pipeline.
        #[must_use]
        pub fn validate(source: &str) -> ::std::vec::Vec<$crate::Diagnostic> {
            ENGINE.validate_raw(source)
        }

        #[cfg(test)]
        mod tests {
            use super::*;
            use $crate::{HostAnalysisOptions, HostError, HostLanguage, InputLimits};

            #[test]
            fn lossless_lex() {
                let source = "fn main() { return 1; }";
                assert!(lex(source).is_lossless(source));
            }

            #[test]
            fn parse_smoke() {
                let source = "function f(x) { return x + 1; }";
                let p = parse(source);
                assert!(p.lexed.is_lossless(source));
                assert!(!p.module.items.is_empty() || source.is_empty());
                let sem = tokenize(source);
                assert_eq!(sem.tokens.len(), p.lexed.tokens.len());
            }

            #[test]
            fn host_enforces_limits_on_all_layers() {
                let mut opts = HostAnalysisOptions::default();
                opts.limits = InputLimits {
                    max_input_bytes: 4,
                    ..InputLimits::default()
                };
                let sample = "12345";
                assert!(matches!(
                    ENGINE.lex(sample, &opts),
                    Err(HostError::InputTooLarge { .. })
                ));
                assert!(matches!(
                    ENGINE.semantic_tokens(sample, &opts),
                    Err(HostError::InputTooLarge { .. })
                ));
                assert!(matches!(
                    ENGINE.diagnose(sample, &opts),
                    Err(HostError::InputTooLarge { .. })
                ));
            }
        }
    };
}
