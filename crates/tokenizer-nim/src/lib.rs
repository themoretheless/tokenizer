//! Full Nim engine (lex → parse → AST → semantic).

use themoretheless_tokenizer_core::{
    Diagnostic, FullProfile, HostAnalysisOptions, HostDiagnostic, HostError, HostLanguage,
    HostTokenization, LanguageDescriptor, LanguageId, Lexed, Parse, SemanticTokenization,
    analyze_full_host, full_descriptor, lex_full, lex_to_host, parse_full, require_default_dialect,
    semantic_full,
};

fn profile() -> FullProfile {
    FullProfile {
        keywords: &[
            "addr",
            "and",
            "as",
            "asm",
            "bind",
            "block",
            "break",
            "case",
            "cast",
            "concept",
            "const",
            "continue",
            "converter",
            "defer",
            "discard",
            "distinct",
            "div",
            "do",
            "elif",
            "else",
            "end",
            "enum",
            "except",
            "export",
            "finally",
            "for",
            "from",
            "func",
            "if",
            "import",
            "in",
            "include",
            "interface",
            "is",
            "isnot",
            "iterator",
            "let",
            "macro",
            "method",
            "mixin",
            "mod",
            "nil",
            "not",
            "notin",
            "object",
            "of",
            "or",
            "out",
            "proc",
            "ptr",
            "raise",
            "ref",
            "return",
            "shl",
            "shr",
            "static",
            "template",
            "try",
            "tuple",
            "type",
            "using",
            "var",
            "when",
            "while",
            "xor",
            "yield",
            "true",
            "false",
        ],
        types: &[
            "int", "int8", "int16", "int32", "int64", "uint", "uint8", "uint16", "uint32",
            "uint64", "float", "float32", "float64", "bool", "char", "string", "cstring",
            "pointer",
        ],
        line_comment: Some("#"),
        block_comment: Some(("#[", "]#")),
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
    LanguageId::NIM,
    "Nim",
    &[],
    &[".nim", ".nims"],
    &["text/x-nim"],
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
