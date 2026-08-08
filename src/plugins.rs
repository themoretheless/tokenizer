//! Built-in language host adapters and registry wiring.

use std::borrow::Cow;
use std::sync::OnceLock;

use themoretheless_tokenizer_core::{
    Capabilities, DialectDescriptor, DialectId, HostAnalysisOptions, HostDiagnostic, HostError,
    HostLanguage, HostSpan, HostToken, HostTokenization, LanguageDescriptor, LanguageId,
    LanguageRegistry, RegistryBuilder, Severity, TokenLayer,
};

#[cfg(feature = "json")]
mod json_host {
    use super::*;
    use themoretheless_tokenizer_json::{LexerOptions, ParseOptions, lex_with, tokenize_with};

    pub struct JsonHost;

    static DIALECTS: &[DialectDescriptor] = &[
        DialectDescriptor {
            id: DialectId::STRICT,
            display_name: "Strict JSON",
            aliases: &[],
        },
        DialectDescriptor {
            id: DialectId::JSONC,
            display_name: "JSONC",
            aliases: &["json-with-comments"],
        },
    ];

    pub static DESCRIPTOR: LanguageDescriptor = LanguageDescriptor {
        language: LanguageId::JSON,
        display_name: "JSON",
        dialects: DIALECTS,
        default_dialect: DialectId::STRICT,
        aliases: &[],
        extensions: &[".json", ".jsonc"],
        mime_types: &["application/json", "application/jsonc"],
        capabilities: Capabilities::JSON_FULL,
        engine_version: env!("CARGO_PKG_VERSION"),
    };

    pub static ENGINE: JsonHost = JsonHost;

    impl HostLanguage for JsonHost {
        fn descriptor(&self) -> &'static LanguageDescriptor {
            &DESCRIPTOR
        }

        fn lex(
            &self,
            source: &str,
            opts: &HostAnalysisOptions,
        ) -> Result<HostTokenization, HostError> {
            let dialect = self.resolve_dialect(opts.dialect.as_ref())?;
            let options = match dialect {
                DialectId::JSONC => LexerOptions::jsonc(),
                _ => LexerOptions::strict(),
            };
            let result = lex_with(source, options);
            let mut tokens = Vec::with_capacity(result.tokens().len());
            for token in result.tokens() {
                tokens.push(HostToken {
                    kind: Cow::Owned(debug_kebab(token.kind)),
                    span: token.span.into(),
                    error: token.has_error(),
                });
            }
            let diagnostics = result
                .diagnostics()
                .iter()
                .map(|diagnostic| HostDiagnostic {
                    code: Cow::Borrowed(diagnostic.kind.code()),
                    message: Cow::Owned(diagnostic.kind.to_string()),
                    span: diagnostic.span.into(),
                    severity: Severity::Error,
                })
                .collect();
            Ok(HostTokenization {
                tokens,
                diagnostics,
                valid: !result.has_errors(),
            })
        }

        fn semantic_tokens(
            &self,
            source: &str,
            opts: &HostAnalysisOptions,
        ) -> Result<HostTokenization, HostError> {
            let dialect = self.resolve_dialect(opts.dialect.as_ref())?;
            let options = match dialect {
                DialectId::JSONC => ParseOptions::jsonc(),
                _ => ParseOptions::strict(),
            };
            let result = tokenize_with(source, options);
            let mut tokens = Vec::with_capacity(result.tokens.len());
            for token in &result.tokens {
                tokens.push(HostToken {
                    kind: Cow::Owned(debug_kebab(token.kind)),
                    span: token.span.into(),
                    error: matches!(
                        token.kind,
                        themoretheless_tokenizer_json::SemanticKind::Invalid
                    ),
                });
            }
            let diagnostics = result
                .diagnostics
                .iter()
                .map(|diagnostic| HostDiagnostic {
                    code: Cow::Borrowed(diagnostic.kind.code()),
                    message: Cow::Owned(diagnostic.kind.to_string()),
                    span: diagnostic.span.into(),
                    severity: Severity::Error,
                })
                .collect();
            Ok(HostTokenization {
                tokens,
                diagnostics,
                valid: result.is_valid(),
            })
        }

        fn diagnose(
            &self,
            source: &str,
            opts: &HostAnalysisOptions,
        ) -> Result<Vec<HostDiagnostic>, HostError> {
            Ok(self.semantic_tokens(source, opts)?.diagnostics)
        }
    }
}

#[cfg(feature = "url")]
mod url_host {
    use super::*;
    use themoretheless_tokenizer_url::{tokenize, validate};

    pub struct UrlHost;

    static DIALECTS: &[DialectDescriptor] = &[DialectDescriptor {
        id: DialectId::DEFAULT,
        display_name: "Default",
        aliases: &[],
    }];

    pub static DESCRIPTOR: LanguageDescriptor = LanguageDescriptor {
        language: LanguageId::URL,
        display_name: "URL",
        dialects: DIALECTS,
        default_dialect: DialectId::DEFAULT,
        aliases: &["uri"],
        extensions: &[],
        mime_types: &[],
        capabilities: Capabilities::URL_TODAY,
        engine_version: env!("CARGO_PKG_VERSION"),
    };

    pub static ENGINE: UrlHost = UrlHost;

    impl HostLanguage for UrlHost {
        fn descriptor(&self) -> &'static LanguageDescriptor {
            &DESCRIPTOR
        }

        fn lex(
            &self,
            source: &str,
            opts: &HostAnalysisOptions,
        ) -> Result<HostTokenization, HostError> {
            let _ = self.resolve_dialect(opts.dialect.as_ref())?;
            let result = tokenize(source);
            let tokens = result
                .tokens
                .iter()
                .map(|token| HostToken {
                    kind: Cow::Borrowed(token.kind.class_name()),
                    span: HostSpan::from(token.span),
                    error: false,
                })
                .collect();
            let diagnostics = result
                .diagnostics
                .iter()
                .map(|diagnostic| HostDiagnostic::from_diagnostic(*diagnostic))
                .collect();
            Ok(HostTokenization {
                tokens,
                diagnostics,
                valid: result.diagnostics.is_empty(),
            })
        }

        fn semantic_tokens(
            &self,
            source: &str,
            opts: &HostAnalysisOptions,
        ) -> Result<HostTokenization, HostError> {
            // URL has highlight kinds only; semantic layer reuses lex tokens.
            self.lex(source, opts)
        }

        fn diagnose(
            &self,
            source: &str,
            opts: &HostAnalysisOptions,
        ) -> Result<Vec<HostDiagnostic>, HostError> {
            let _ = self.resolve_dialect(opts.dialect.as_ref())?;
            Ok(validate(source)
                .into_iter()
                .map(HostDiagnostic::from_diagnostic)
                .collect())
        }
    }
}

/// Register all Cargo-feature-enabled built-in languages.
pub fn register_builtins(
    builder: &mut RegistryBuilder,
) -> Result<(), themoretheless_tokenizer_core::RegisterError> {
    #[cfg(feature = "json")]
    {
        builder.register(&json_host::ENGINE)?;
    }
    #[cfg(feature = "url")]
    {
        builder.register(&url_host::ENGINE)?;
    }
    let _ = builder;
    Ok(())
}

/// Built-in registry for enabled language features.
#[must_use]
pub fn builtin_registry() -> &'static LanguageRegistry {
    static REGISTRY: OnceLock<LanguageRegistry> = OnceLock::new();
    REGISTRY.get_or_init(|| {
        let mut builder = RegistryBuilder::new();
        register_builtins(&mut builder).expect("builtin language registration must succeed");
        builder.build()
    })
}

/// Analyze source through the builtin registry.
pub fn analyze_host(
    language: &str,
    source: &str,
    dialect: &str,
    layer: TokenLayer,
) -> Result<HostTokenization, HostError> {
    let engine =
        builtin_registry()
            .get_str(language)
            .ok_or_else(|| HostError::UnknownLanguage {
                language: language.to_owned(),
            })?;
    let dialect = if dialect.is_empty() {
        engine.descriptor().default_dialect.as_str()
    } else {
        dialect
    };
    let opts = HostAnalysisOptions::new(Cow::Owned(dialect.to_owned()));
    if opts.limits.exceeds_input_bytes(source.len()) {
        return Err(HostError::InputTooLarge {
            max: opts.limits.max_input_bytes,
            actual: source.len(),
        });
    }
    engine.tokenize_layer(source, &opts, layer)
}

fn debug_kebab(value: impl std::fmt::Debug) -> String {
    let value = format!("{value:?}");
    let mut result = String::with_capacity(value.len() + 4);
    for (index, character) in value.chars().enumerate() {
        if character.is_ascii_uppercase() && index != 0 {
            result.push('-');
        }
        result.push(character.to_ascii_lowercase());
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtins_include_enabled_languages() {
        let registry = builtin_registry();
        #[cfg(feature = "json")]
        assert!(registry.get(LanguageId::JSON).is_some());
        #[cfg(feature = "url")]
        assert!(registry.get(LanguageId::URL).is_some());
    }

    #[cfg(feature = "json")]
    #[test]
    fn json_host_tokenizes_semantic_layer() {
        let result = analyze_host("json", r#"{"a":1}"#, "strict", TokenLayer::Semantic).unwrap();
        assert!(result.valid);
        assert!(result.tokens.iter().any(|token| token.kind == "property"));
    }

    #[cfg(feature = "url")]
    #[test]
    fn url_host_tokenizes_syntax_layer() {
        let result = analyze_host(
            "url",
            "https://example.com/x",
            "default",
            TokenLayer::Syntax,
        )
        .unwrap();
        assert!(result.valid);
        assert!(result.tokens.iter().any(|token| token.kind == "u-scheme"));
    }
}
