//! Full Visual Basic engine (lex → parse → AST → semantic).

use themoretheless_tokenizer_core::{
    Diagnostic, FullProfile, HostAnalysisOptions, HostDiagnostic, HostError, HostLanguage,
    HostTokenization, LanguageDescriptor, LanguageId, Lexed, Parse, SemanticTokenization,
    analyze_full_host, full_descriptor, lex_full, lex_to_host, parse_full, require_default_dialect,
    semantic_full,
};

fn profile() -> FullProfile {
    FullProfile {
        keywords: &["AddHandler",
        "AddressOf",
        "Alias",
        "And",
        "AndAlso",
        "As",
        "Boolean",
        "ByRef",
        "Byte",
        "ByVal",
        "Call",
        "Case",
        "Catch",
        "CBool",
        "CByte",
        "CChar",
        "CDate",
        "CDbl",
        "CDec",
        "Char",
        "CInt",
        "Class",
        "CLng",
        "CObj",
        "Const",
        "Continue",
        "CSByte",
        "CShort",
        "CSng",
        "CStr",
        "CType",
        "CUInt",
        "CULng",
        "CUShort",
        "Date",
        "Decimal",
        "Declare",
        "Default",
        "Delegate",
        "Dim",
        "DirectCast",
        "Do",
        "Double",
        "Each",
        "Else",
        "ElseIf",
        "End",
        "Enum",
        "Erase",
        "Error",
        "Event",
        "Exit",
        "False",
        "Finally",
        "For",
        "Friend",
        "Function",
        "Get",
        "GetType",
        "GetXMLNamespace",
        "Global",
        "GoSub",
        "GoTo",
        "Handles",
        "If",
        "Implements",
        "Imports",
        "In",
        "Inherits",
        "Integer",
        "Interface",
        "Is",
        "IsNot",
        "Let",
        "Lib",
        "Like",
        "Long",
        "Loop",
        "Me",
        "Mod",
        "Module",
        "MustInherit",
        "MustOverride",
        "MyBase",
        "MyClass",
        "Namespace",
        "Narrowing",
        "New",
        "Next",
        "Not",
        "Nothing",
        "NotInheritable",
        "NotOverridable",
        "Object",
        "Of",
        "On",
        "Operator",
        "Option",
        "Optional",
        "Or",
        "OrElse",
        "Overloads",
        "Overridable",
        "Overrides",
        "ParamArray",
        "Partial",
        "Private",
        "Property",
        "Protected",
        "Public",
        "RaiseEvent",
        "ReadOnly",
        "ReDim",
        "REM",
        "RemoveHandler",
        "Resume",
        "Return",
        "SByte",
        "Select",
        "Set",
        "Shadows",
        "Shared",
        "Short",
        "Single",
        "Static",
        "Step",
        "Stop",
        "String",
        "Structure",
        "Sub",
        "SyncLock",
        "Then",
        "Throw",
        "To",
        "True",
        "Try",
        "TryCast",
        "TypeOf",
        "UInteger",
        "ULong",
        "UShort",
        "Using",
        "Variant",
        "When",
        "While",
        "Widening",
        "With",
        "WithEvents",
        "WriteOnly",
        "Xor",
        "Await",
        "Async",
        "Iterator",
        "Yield"],
        types: &[],
        line_comment: Some("'"),
        block_comment: None,
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
    LanguageId::VISUALBASIC,
    "Visual Basic",
    &["vb", "vbnet"],
    &[".vb", ".bas", ".vbs"],
    &["text/x-vb"],
    env!("CARGO_PKG_VERSION"),
);

impl HostLanguage for Host {
    fn descriptor(&self) -> &'static LanguageDescriptor {
        &DESCRIPTOR
    }

    fn lex(
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
