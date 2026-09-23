//! Full D engine (lex → parse → AST → semantic).

use themoretheless_tokenizer_core::{
    Diagnostic, FullProfile, HostAnalysisOptions, HostDiagnostic, HostError, HostLanguage,
    HostTokenization, LanguageDescriptor, LanguageId, Lexed, Parse, SemanticTokenization,
    analyze_full_host, full_descriptor, lex_full, lex_to_host, parse_full, require_default_dialect,
    semantic_full,
};

fn profile() -> FullProfile {
    FullProfile {
        keywords: &[
            "abstract",
            "alias",
            "align",
            "asm",
            "assert",
            "auto",
            "body",
            "bool",
            "break",
            "byte",
            "case",
            "cast",
            "catch",
            "cdouble",
            "cent",
            "cfloat",
            "char",
            "class",
            "const",
            "continue",
            "creal",
            "dchar",
            "debug",
            "default",
            "delegate",
            "delete",
            "deprecated",
            "do",
            "double",
            "else",
            "enum",
            "export",
            "extern",
            "false",
            "final",
            "finally",
            "float",
            "for",
            "foreach",
            "foreach_reverse",
            "function",
            "goto",
            "idouble",
            "if",
            "ifloat",
            "immutable",
            "import",
            "in",
            "inout",
            "int",
            "interface",
            "invariant",
            "ireal",
            "is",
            "lazy",
            "long",
            "macro",
            "mixin",
            "module",
            "new",
            "nothrow",
            "null",
            "out",
            "override",
            "package",
            "pragma",
            "private",
            "protected",
            "public",
            "pure",
            "real",
            "ref",
            "return",
            "scope",
            "shared",
            "short",
            "static",
            "struct",
            "super",
            "switch",
            "synchronized",
            "template",
            "this",
            "throw",
            "true",
            "try",
            "typedef",
            "typeid",
            "typeof",
            "ubyte",
            "ucent",
            "uint",
            "ulong",
            "union",
            "unittest",
            "ushort",
            "version",
            "void",
            "volatile",
            "wchar",
            "while",
            "with",
        ],
        types: &[
            "bool", "byte", "ubyte", "short", "ushort", "int", "uint", "long", "ulong", "cent",
            "ucent", "float", "double", "real", "char", "wchar", "dchar", "void",
        ],
        line_comment: Some("//"),
        block_comment: Some(("/*", "*/")),
        hash_line_comment: false,
        dollar_ident: false,
        triple_strings: false,
        soft_indent_blocks: false,
    }
}

/// Lossless lexer.
#[must_use]
pub fn lex(source: &str) -> Lexed {
    lex_full(source, &profile())
}

/// Recovering parse with borrowing AST.
#[must_use]
pub fn parse(source: &str) -> Parse<'_> {
    parse_full(source, &profile())
}

/// Parser-aware semantic tokens.
#[must_use]
pub fn tokenize(source: &str) -> SemanticTokenization {
    semantic_full(&parse(source))
}

/// Diagnostics from the full pipeline.
#[must_use]
pub fn validate(source: &str) -> Vec<Diagnostic> {
    parse(source).diagnostics
}

/// Host adapter.
#[derive(Debug, Default, Clone, Copy)]
pub struct Host;

pub static ENGINE: Host = Host;

pub static DESCRIPTOR: LanguageDescriptor = full_descriptor(
    LanguageId::DLANG,
    "D",
    &["d"],
    &[".d", ".di"],
    &["text/x-d"],
    env!("CARGO_PKG_VERSION"),
);

impl HostLanguage for Host {
    fn descriptor(&self) -> &'static LanguageDescriptor {
        &DESCRIPTOR
    }

    fn lex(&self, source: &str, opts: &HostAnalysisOptions) -> Result<HostTokenization, HostError> {
        require_default_dialect(&DESCRIPTOR, opts.dialect.as_ref())?;
        if opts.limits.exceeds_input_bytes(source.len()) {
            return Err(HostError::InputTooLarge {
                max: opts.limits.max_input_bytes,
                actual: source.len(),
            });
        }
        Ok(lex_to_host(lex(source)))
    }

    fn semantic_tokens(
        &self,
        source: &str,
        opts: &HostAnalysisOptions,
    ) -> Result<HostTokenization, HostError> {
        require_default_dialect(&DESCRIPTOR, opts.dialect.as_ref())?;
        if opts.limits.exceeds_input_bytes(source.len()) {
            return Err(HostError::InputTooLarge {
                max: opts.limits.max_input_bytes,
                actual: source.len(),
            });
        }
        Ok(analyze_full_host(source, &profile()))
    }

    fn diagnose(
        &self,
        source: &str,
        opts: &HostAnalysisOptions,
    ) -> Result<Vec<HostDiagnostic>, HostError> {
        require_default_dialect(&DESCRIPTOR, opts.dialect.as_ref())?;
        Ok(validate(source)
            .into_iter()
            .map(themoretheless_tokenizer_core::HostDiagnostic::from_diagnostic)
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    }
}
