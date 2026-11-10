//! Full Ruby engine (lex → parse → AST → semantic).

#![forbid(unsafe_code)]

use themoretheless_tokenizer_core::{FullProfile, LanguageId, define_full_engine};

define_full_engine! {
    id: LanguageId::RUBY,
    name: "Ruby",
    aliases: &["rb"],
    extensions: &[".rb"],
    mime_types: &["text/x-ruby"],
    profile: FullProfile {
        keywords: &[
            "BEGIN", "END", "alias", "and", "begin", "break", "case", "class", "def", "defined",
            "do", "else", "elsif", "end", "ensure", "false", "for", "if", "in", "module", "next",
            "nil", "not", "or", "redo", "rescue", "retry", "return", "self", "super", "then",
            "true", "undef", "unless", "until", "when", "while", "yield",
        ],
        types: &[],
        line_comment: None,
        block_comment: None,
        hash_line_comment: true,
        dollar_ident: false,
        backtick_strings: false,
        triple_strings: false,
        soft_indent_blocks: false,
    },
}
