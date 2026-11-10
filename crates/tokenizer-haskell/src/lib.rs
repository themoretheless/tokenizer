//! Full Haskell engine (lex → parse → AST → semantic).

#![forbid(unsafe_code)]

use themoretheless_tokenizer_core::{FullProfile, LanguageId, define_full_engine};

define_full_engine! {
    id: LanguageId::HASKELL,
    name: "Haskell",
    aliases: &["hs"],
    extensions: &[".hs", ".lhs"],
    mime_types: &["text/x-haskell"],
    profile: FullProfile {
        keywords: &[
            "case",
            "class",
            "data",
            "default",
            "deriving",
            "do",
            "else",
            "if",
            "import",
            "in",
            "infix",
            "infixl",
            "infixr",
            "instance",
            "let",
            "module",
            "newtype",
            "of",
            "then",
            "type",
            "where",
            "foreign",
            "mdo",
            "proc",
            "rec",
            "qualified",
            "as",
            "hiding",
        ],
        types: &[],
        line_comment: Some("--"),
        block_comment: Some(("{-", "-}")),
        hash_line_comment: false,
        dollar_ident: false,
        backtick_strings: false,
        triple_strings: false,
        soft_indent_blocks: false,
    },
}
