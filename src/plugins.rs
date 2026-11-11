//! Built-in language host adapters and registry wiring.

use std::borrow::Cow;
use std::sync::OnceLock;

use themoretheless_tokenizer_core::{
    HostAnalysisOptions, HostError, HostTokenization, LanguageRegistry, RegistryBuilder, TokenLayer,
};

macro_rules! register_all {
    ($builder:ident; $( $feat:literal => $engine:path ),* $(,)?) => {
        $(
            #[cfg(feature = $feat)]
            {
                $builder.register(&$engine)?;
            }
        )*
    };
}

/// Register all Cargo-feature-enabled built-in languages in deterministic priority order.
pub fn register_builtins(
    builder: &mut RegistryBuilder,
) -> Result<(), themoretheless_tokenizer_core::RegisterError> {
    register_all! { builder;
        // Formats
        "json" => themoretheless_tokenizer_json::ENGINE,
        "json5" => themoretheless_tokenizer_json::JSON5_ENGINE,
        "jsonl" => themoretheless_tokenizer_json::JSONL_ENGINE,
        "url" => themoretheless_tokenizer_url::ENGINE,
        "xml" => themoretheless_tokenizer_xml::ENGINE,
        "html" => themoretheless_tokenizer_html::ENGINE,
        "css" => themoretheless_tokenizer_css::ENGINE,
        "yaml" => themoretheless_tokenizer_yaml::ENGINE,
        "toml" => themoretheless_tokenizer_toml::ENGINE,
        "markdown" => themoretheless_tokenizer_markdown::ENGINE,
        "csv" => themoretheless_tokenizer_csv::ENGINE,
        "tsv" => themoretheless_tokenizer_csv::TSV_ENGINE,
        "logfmt" => themoretheless_tokenizer_logfmt::ENGINE,
        "ini" => themoretheless_tokenizer_ini::ENGINE,
        "properties" => themoretheless_tokenizer_ini::PROPERTIES_ENGINE,
        "hcl" => themoretheless_tokenizer_hcl::ENGINE,
        "edn" => themoretheless_tokenizer_edn::ENGINE,
        "ics" => themoretheless_tokenizer_ics::ENGINE,
        "srt" => themoretheless_tokenizer_srt::ENGINE,
        "vtt" => themoretheless_tokenizer_srt::VTT_ENGINE,

        // Wave 2 + Rush
        "sql" => themoretheless_tokenizer_languages::sql::ENGINE,
        "mongo" => themoretheless_tokenizer_languages::mongo::ENGINE,
        "bash" => themoretheless_tokenizer_languages::bash::ENGINE,
        "rush" => themoretheless_tokenizer_rush::ENGINE,
        "powershell" => themoretheless_tokenizer_languages::powershell::ENGINE,

        // Wave 3–5
        "javascript" => themoretheless_tokenizer_languages::javascript::ENGINE,
        "typescript" => themoretheless_tokenizer_languages::typescript::ENGINE,
        "python" => themoretheless_tokenizer_languages::python::ENGINE,
        "java" => themoretheless_tokenizer_languages::java::ENGINE,
        "csharp" => themoretheless_tokenizer_languages::csharp::ENGINE,
        "go" => themoretheless_tokenizer_languages::go::ENGINE,
        "php" => themoretheless_tokenizer_languages::php::ENGINE,
        "ruby" => themoretheless_tokenizer_languages::ruby::ENGINE,
        "c" => themoretheless_tokenizer_languages::c::ENGINE,
        "cpp" => themoretheless_tokenizer_languages::cpp::ENGINE,
        "rust" => themoretheless_tokenizer_languages::rust::ENGINE,
        "kotlin" => themoretheless_tokenizer_languages::kotlin::ENGINE,
        "swift" => themoretheless_tokenizer_languages::swift::ENGINE,
        "dart" => themoretheless_tokenizer_languages::dart::ENGINE,
        "r" => themoretheless_tokenizer_languages::r::ENGINE,

        // Wave 6
        "visualbasic" => themoretheless_tokenizer_languages::visualbasic::ENGINE,
        "fortran" => themoretheless_tokenizer_languages::fortran::ENGINE,
        "matlab" => themoretheless_tokenizer_languages::matlab::ENGINE,
        "delphi" => themoretheless_tokenizer_languages::delphi::ENGINE,
        "scala" => themoretheless_tokenizer_languages::scala::ENGINE,
        "lua" => themoretheless_tokenizer_languages::lua::ENGINE,
        "perl" => themoretheless_tokenizer_languages::perl::ENGINE,
        "objectivec" => themoretheless_tokenizer_languages::objectivec::ENGINE,
        "julia" => themoretheless_tokenizer_languages::julia::ENGINE,
        "assembly" => themoretheless_tokenizer_languages::assembly::ENGINE,

        // Wave 7 (next20)
        "groovy" => themoretheless_tokenizer_languages::groovy::ENGINE,
        "haskell" => themoretheless_tokenizer_languages::haskell::ENGINE,
        "elixir" => themoretheless_tokenizer_languages::elixir::ENGINE,
        "erlang" => themoretheless_tokenizer_languages::erlang::ENGINE,
        "clojure" => themoretheless_tokenizer_languages::clojure::ENGINE,
        "fsharp" => themoretheless_tokenizer_languages::fsharp::ENGINE,
        "ocaml" => themoretheless_tokenizer_languages::ocaml::ENGINE,
        "lisp" => themoretheless_tokenizer_languages::lisp::ENGINE,
        "scheme" => themoretheless_tokenizer_languages::scheme::ENGINE,
        "solidity" => themoretheless_tokenizer_languages::solidity::ENGINE,
        "zig" => themoretheless_tokenizer_languages::zig::ENGINE,
        "nim" => themoretheless_tokenizer_languages::nim::ENGINE,
        "dlang" => themoretheless_tokenizer_languages::dlang::ENGINE,
        "cobol" => themoretheless_tokenizer_languages::cobol::ENGINE,
        "ada" => themoretheless_tokenizer_languages::ada::ENGINE,
        "prolog" => themoretheless_tokenizer_languages::prolog::ENGINE,
        "abap" => themoretheless_tokenizer_languages::abap::ENGINE,
        "vhdl" => themoretheless_tokenizer_languages::vhdl::ENGINE,
        "verilog" => themoretheless_tokenizer_languages::verilog::ENGINE,
        "graphql" => themoretheless_tokenizer_languages::graphql::ENGINE,

        // Forma UI / Scene language
        "forma" => themoretheless_tokenizer_forma::ENGINE,
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

/// Analyze source through the builtin registry with explicit [`HostAnalysisOptions`].
pub fn analyze_host_with_options(
    language: &str,
    source: &str,
    opts: &HostAnalysisOptions,
    layer: TokenLayer,
) -> Result<HostTokenization, HostError> {
    let engine =
        builtin_registry()
            .get_str(language)
            .ok_or_else(|| HostError::UnknownLanguage {
                language: language.to_owned(),
            })?;
    if opts.limits.exceeds_input_bytes(source.len()) {
        return Err(HostError::InputTooLarge {
            max: opts.limits.max_input_bytes,
            actual: source.len(),
        });
    }
    if opts.dialect.is_empty() {
        let resolved = HostAnalysisOptions {
            dialect: Cow::Borrowed(engine.descriptor().default_dialect.as_str()),
            limits: opts.limits,
        };
        engine.tokenize_layer(source, &resolved, layer)
    } else {
        engine.tokenize_layer(source, opts, layer)
    }
}

/// Analyze source through the builtin registry using conservative input limits.
pub fn analyze_host(
    language: &str,
    source: &str,
    dialect: &str,
    layer: TokenLayer,
) -> Result<HostTokenization, HostError> {
    let opts = HostAnalysisOptions {
        dialect: Cow::Owned(dialect.to_owned()),
        limits: themoretheless_tokenizer_core::InputLimits::conservative(),
    };
    analyze_host_with_options(language, source, &opts, layer)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(any(feature = "json", feature = "url"))]
    use themoretheless_tokenizer_core::LanguageId;

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
        let result = analyze_host(
            "python",
            "def f():\n    return 1\n",
            "default",
            TokenLayer::Semantic,
        )
        .unwrap();
        assert!(result.tokens.iter().any(|t| t.kind == "keyword"));
    }

    #[cfg(feature = "all-languages")]
    #[test]
    fn all_languages_register() {
        assert_eq!(builtin_registry().len(), 71);
    }

    #[cfg(feature = "top20")]
    #[test]
    fn top20_register() {
        let reg = builtin_registry();
        for id in themoretheless_tokenizer_core::TOP20_IDS {
            assert!(reg.get_str(id.as_str()).is_some(), "missing top20 id {id}");
        }
    }

    #[cfg(feature = "next20")]
    #[test]
    fn next20_register() {
        let reg = builtin_registry();
        for id in themoretheless_tokenizer_core::NEXT20_IDS {
            assert!(reg.get_str(id.as_str()).is_some(), "missing next20 id {id}");
        }
    }

    #[cfg(feature = "formats")]
    #[test]
    fn formats_register() {
        use themoretheless_tokenizer_core::Family;
        let reg = builtin_registry();
        for id in themoretheless_tokenizer_core::FORMAT_IDS {
            assert!(reg.get_str(id.as_str()).is_some(), "missing format id {id}");
            assert_eq!(Family::of(*id), Family::FORMAT);
        }
    }
}
