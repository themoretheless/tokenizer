//! Full Lua engine (lex → parse → AST → semantic).

#![forbid(unsafe_code)]

use themoretheless_tokenizer_core::{FullProfile, LanguageId, define_full_engine};

define_full_engine! {
    id: LanguageId::LUA,
    name: "Lua",
    aliases: &[],
    extensions: &[".lua"],
    mime_types: &["text/x-lua"],
    profile: FullProfile {
        keywords: &[
            "and", "break", "do", "else", "elseif", "end", "false", "for", "function", "goto",
            "if", "in", "local", "nil", "not", "or", "repeat", "return", "then", "true", "until",
            "while",
        ],
        types: &[],
        line_comment: Some("--"),
        block_comment: Some(("--[[", "]]")),
        hash_line_comment: false,
        dollar_ident: false,
        backtick_strings: false,
        triple_strings: false,
        soft_indent_blocks: false,
    },
}
