//! Full PowerShell engine (lex → parse → AST → semantic).

#![forbid(unsafe_code)]

use themoretheless_tokenizer_core::{FullProfile, LanguageId, define_full_engine};

define_full_engine! {
    id: LanguageId::POWERSHELL,
    name: "PowerShell",
    aliases: &["ps1"],
    extensions: &[".ps1", ".psm1"],
    mime_types: &[],
    profile: FullProfile {
        keywords: &[
            "if", "else", "elseif", "foreach", "for", "while", "function", "param", "begin",
            "process", "end", "switch", "return", "throw", "try", "catch", "finally", "filter",
        ],
        types: &[],
        line_comment: Some("#"),
        block_comment: Some(("<#", "#>")),
        hash_line_comment: false,
        dollar_ident: false,
        backtick_strings: false,
        triple_strings: false,
        soft_indent_blocks: false,
    },
}
