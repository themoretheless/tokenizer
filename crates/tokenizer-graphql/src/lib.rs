//! Full GraphQL engine (lex → parse → AST → semantic).

#![forbid(unsafe_code)]

use themoretheless_tokenizer_core::{FullProfile, LanguageId, define_full_engine};

define_full_engine! {
    id: LanguageId::GRAPHQL,
    name: "GraphQL",
    aliases: &["gql"],
    extensions: &[".graphql", ".gql"],
    mime_types: &["application/graphql"],
    profile: FullProfile {
        keywords: &[
            "query",
            "mutation",
            "subscription",
            "schema",
            "type",
            "interface",
            "union",
            "enum",
            "input",
            "extend",
            "implements",
            "fragment",
            "on",
            "scalar",
            "true",
            "false",
            "null",
            "directive",
            "repeatable",
        ],
        types: &["Int", "Float", "String", "Boolean", "ID"],
        line_comment: None,
        block_comment: None,
        hash_line_comment: true,
        dollar_ident: false,
        backtick_strings: false,
        triple_strings: false,
        soft_indent_blocks: false,
    },
}
