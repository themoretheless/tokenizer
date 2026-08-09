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
            let mut diagnostics: Vec<HostDiagnostic> = result
                .diagnostics
                .iter()
                .map(|diagnostic| HostDiagnostic::from_diagnostic(*diagnostic))
                .collect();
            for diagnostic in validate(source) {
                diagnostics.push(HostDiagnostic::from_diagnostic(diagnostic));
            }
            let tokens = result
                .tokens
                .iter()
                .map(|token| HostToken {
                    kind: Cow::Borrowed(token.kind.class_name()),
                    span: HostSpan::from(token.span),
                    error: false,
                })
                .collect();
            Ok(HostTokenization {
                tokens,
                valid: diagnostics.is_empty(),
                diagnostics,
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
    #[cfg(feature = "xml")]
    {
        builder.register(&themoretheless_tokenizer_xml::ENGINE)?;
    }
    #[cfg(feature = "html")]
    {
        builder.register(&themoretheless_tokenizer_html::ENGINE)?;
    }
    #[cfg(feature = "css")]
    {
        builder.register(&themoretheless_tokenizer_css::ENGINE)?;
    }
    #[cfg(feature = "yaml")]
    {
        builder.register(&themoretheless_tokenizer_yaml::ENGINE)?;
    }
    #[cfg(feature = "toml")]
    {
        builder.register(&themoretheless_tokenizer_toml::ENGINE)?;
    }
    #[cfg(feature = "markdown")]
    {
        builder.register(&themoretheless_tokenizer_markdown::ENGINE)?;
    }
    #[cfg(feature = "sql")]
    {
        builder.register(&themoretheless_tokenizer_sql::ENGINE)?;
    }
    #[cfg(feature = "mongo")]
    {
        builder.register(&themoretheless_tokenizer_mongo::ENGINE)?;
    }
    #[cfg(feature = "bash")]
    {
        builder.register(&themoretheless_tokenizer_bash::ENGINE)?;
    }
    #[cfg(feature = "powershell")]
    {
        builder.register(&themoretheless_tokenizer_powershell::ENGINE)?;
    }
    #[cfg(feature = "javascript")]
    {
        builder.register(&themoretheless_tokenizer_javascript::ENGINE)?;
    }
    #[cfg(feature = "typescript")]
    {
        builder.register(&themoretheless_tokenizer_typescript::ENGINE)?;
    }
    #[cfg(feature = "python")]
    {
        builder.register(&themoretheless_tokenizer_python::ENGINE)?;
    }
    #[cfg(feature = "java")]
    {
        builder.register(&themoretheless_tokenizer_java::ENGINE)?;
    }
    #[cfg(feature = "csharp")]
    {
        builder.register(&themoretheless_tokenizer_csharp::ENGINE)?;
    }
    #[cfg(feature = "go")]
    {
        builder.register(&themoretheless_tokenizer_go::ENGINE)?;
    }
    #[cfg(feature = "php")]
    {
        builder.register(&themoretheless_tokenizer_php::ENGINE)?;
    }
    #[cfg(feature = "ruby")]
    {
        builder.register(&themoretheless_tokenizer_ruby::ENGINE)?;
    }
    #[cfg(feature = "c")]
    {
        builder.register(&themoretheless_tokenizer_c::ENGINE)?;
    }
    #[cfg(feature = "cpp")]
    {
        builder.register(&themoretheless_tokenizer_cpp::ENGINE)?;
    }
    #[cfg(feature = "rust")]
    {
        builder.register(&themoretheless_tokenizer_rust::ENGINE)?;
    }
    #[cfg(feature = "kotlin")]
    {
        builder.register(&themoretheless_tokenizer_kotlin::ENGINE)?;
    }
    #[cfg(feature = "swift")]
    {
        builder.register(&themoretheless_tokenizer_swift::ENGINE)?;
    }
    #[cfg(feature = "dart")]
    {
        builder.register(&themoretheless_tokenizer_dart::ENGINE)?;
    }
    #[cfg(feature = "r")]
    {
        builder.register(&themoretheless_tokenizer_r::ENGINE)?;
    }
    #[cfg(feature = "visualbasic")]
    {
        builder.register(&themoretheless_tokenizer_visualbasic::ENGINE)?;
    }
    #[cfg(feature = "fortran")]
    {
        builder.register(&themoretheless_tokenizer_fortran::ENGINE)?;
    }
    #[cfg(feature = "matlab")]
    {
        builder.register(&themoretheless_tokenizer_matlab::ENGINE)?;
    }
    #[cfg(feature = "delphi")]
    {
        builder.register(&themoretheless_tokenizer_delphi::ENGINE)?;
    }
    #[cfg(feature = "scala")]
    {
        builder.register(&themoretheless_tokenizer_scala::ENGINE)?;
    }
    #[cfg(feature = "lua")]
    {
        builder.register(&themoretheless_tokenizer_lua::ENGINE)?;
    }
    #[cfg(feature = "perl")]
    {
        builder.register(&themoretheless_tokenizer_perl::ENGINE)?;
    }
    #[cfg(feature = "objectivec")]
    {
        builder.register(&themoretheless_tokenizer_objectivec::ENGINE)?;
    }
    #[cfg(feature = "julia")]
    {
        builder.register(&themoretheless_tokenizer_julia::ENGINE)?;
    }
    #[cfg(feature = "assembly")]
    {
        builder.register(&themoretheless_tokenizer_assembly::ENGINE)?;
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

    #[cfg(feature = "python")]
    #[test]
    fn python_host_highlights_keywords() {
        let result = analyze_host("python", "def f():\n    return 1\n", "default", TokenLayer::Semantic)
            .unwrap();
        assert!(result.tokens.iter().any(|t| t.kind == "keyword"));
    }

    #[cfg(feature = "all-languages")]
    #[test]
    fn all_languages_register() {
        // json + url + 35 language crates
        assert_eq!(builtin_registry().len(), 37);
    }

    #[cfg(feature = "top20")]
    #[test]
    fn top20_register() {
        let reg = builtin_registry();
        for id in [
            "python",
            "c",
            "cpp",
            "java",
            "csharp",
            "javascript",
            "visualbasic",
            "sql",
            "go",
            "fortran",
            "matlab",
            "php",
            "rust",
            "r",
            "ruby",
            "kotlin",
            "swift",
            "typescript",
            "delphi",
            "assembly",
        ] {
            assert!(reg.get_str(id).is_some(), "missing top20 id {id}");
        }
    }
}
