//! Full MongoDB engine (lex → parse → AST → semantic).

#![forbid(unsafe_code)]

use themoretheless_tokenizer_core::{FullProfile, LanguageId, define_full_engine};

define_full_engine! {
    id: LanguageId::MONGO,
    name: "MongoDB",
    aliases: &["mongodb"],
    extensions: &[".mongodb", ".mongo"],
    mime_types: &[],
    profile: FullProfile {
        keywords: &[
            "find",
            "findOne",
            "insert",
            "update",
            "delete",
            "aggregate",
            "match",
            "project",
            "group",
            "sort",
            "limit",
            "skip",
            "lookup",
            "unwind",
        ],
        types: &[],
        line_comment: Some("//"),
        block_comment: Some(("/*", "*/")),
        hash_line_comment: false,
        dollar_ident: false,
        backtick_strings: false,
        triple_strings: false,
        soft_indent_blocks: false,
    },
}
