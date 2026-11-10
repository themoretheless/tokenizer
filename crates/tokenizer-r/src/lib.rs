//! Full R engine (lex → parse → AST → semantic).

#![forbid(unsafe_code)]

use themoretheless_tokenizer_core::{FullProfile, LanguageId, define_full_engine};

define_full_engine! {
    id: LanguageId::R,
    name: "R",
    aliases: &[],
    extensions: &[".r", ".R"],
    mime_types: &["text/x-r"],
    profile: FullProfile {
        keywords: &[
            "if", "else", "repeat", "while", "function", "for", "in", "next", "break", "TRUE",
            "FALSE", "NULL", "NA", "Inf", "NaN",
        ],
        types: &[],
        line_comment: Some("#"),
        block_comment: None,
        hash_line_comment: false,
        dollar_ident: false,
        backtick_strings: false,
        triple_strings: false,
        soft_indent_blocks: false,
    },
}
