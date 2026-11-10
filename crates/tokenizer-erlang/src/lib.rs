//! Full Erlang engine (lex → parse → AST → semantic).

#![forbid(unsafe_code)]

use themoretheless_tokenizer_core::{FullProfile, LanguageId, define_full_engine};

define_full_engine! {
    id: LanguageId::ERLANG,
    name: "Erlang",
    aliases: &["erl"],
    extensions: &[".erl", ".hrl"],
    mime_types: &["text/x-erlang"],
    profile: FullProfile {
        keywords: &[
            "after", "and", "andalso", "band", "begin", "bnot", "bor", "bsl", "bsr", "bxor",
            "case", "catch", "cond", "div", "end", "fun", "if", "let", "not", "of", "or", "orelse",
            "query", "receive", "rem", "try", "when", "xor", "true", "false",
        ],
        types: &[],
        line_comment: Some("%"),
        block_comment: None,
        hash_line_comment: false,
        dollar_ident: false,
        backtick_strings: false,
        triple_strings: false,
        soft_indent_blocks: false,
    },
}
