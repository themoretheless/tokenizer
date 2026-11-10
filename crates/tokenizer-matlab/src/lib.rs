//! Full MATLAB engine (lex → parse → AST → semantic).

#![forbid(unsafe_code)]

use themoretheless_tokenizer_core::{FullProfile, LanguageId, define_full_engine};

define_full_engine! {
    id: LanguageId::MATLAB,
    name: "MATLAB",
    aliases: &["octave"],
    extensions: &[".m"],
    mime_types: &["text/x-matlab"],
    profile: FullProfile {
        keywords: &[
            "if",
            "else",
            "elseif",
            "end",
            "for",
            "while",
            "break",
            "continue",
            "switch",
            "case",
            "otherwise",
            "function",
            "return",
            "global",
            "persistent",
            "try",
            "catch",
            "classdef",
            "properties",
            "methods",
            "events",
            "enumeration",
            "parfor",
            "spmd",
            "true",
            "false",
        ],
        types: &[],
        line_comment: Some("%"),
        block_comment: Some(("%{", "%}")),
        hash_line_comment: false,
        dollar_ident: false,
        backtick_strings: false,
        triple_strings: false,
        soft_indent_blocks: false,
    },
}
