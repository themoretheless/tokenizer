//! Full SQL engine (lex → parse → AST → semantic).

#![forbid(unsafe_code)]

use themoretheless_tokenizer_core::{FullProfile, LanguageId, define_full_engine};

define_full_engine! {
    id: LanguageId::SQL,
    name: "SQL",
    aliases: &[],
    extensions: &[".sql"],
    mime_types: &["application/sql"],
    profile: FullProfile {
        keywords: &[
            "select", "insert", "update", "delete", "from", "where", "join", "left", "right",
            "inner", "outer", "group", "order", "by", "as", "and", "or", "not", "null", "true",
            "false", "create", "table", "index", "view", "into", "values", "set", "limit",
            "offset", "having", "distinct", "union", "all", "case", "when", "then", "else", "end",
            "on", "using",
        ],
        types: &[],
        line_comment: Some("--"),
        block_comment: Some(("/*", "*/")),
        hash_line_comment: false,
        dollar_ident: false,
        backtick_strings: true,
        triple_strings: false,
        soft_indent_blocks: false,
    },
}
