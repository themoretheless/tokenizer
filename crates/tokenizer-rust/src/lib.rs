//! Full Rust engine (lex → parse → AST → semantic).

#![forbid(unsafe_code)]

use themoretheless_tokenizer_core::{FullProfile, LanguageId, define_full_engine};

define_full_engine! {
    id: LanguageId::RUST,
    name: "Rust",
    aliases: &["rs"],
    extensions: &[".rs"],
    mime_types: &["text/x-rust"],
    profile: FullProfile {
        keywords: &[
            "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum",
            "extern", "false", "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod",
            "move", "mut", "pub", "ref", "return", "self", "Self", "static", "struct", "super",
            "trait", "true", "type", "unsafe", "use", "where", "while", "abstract", "become",
            "box", "do", "final", "macro", "override", "priv", "typeof", "unsized", "virtual",
            "yield", "try",
        ],
        types: &[
            "i8", "i16", "i32", "i64", "u8", "u16", "u32", "u64", "isize", "usize", "f32", "f64",
            "bool", "char", "str", "String", "Vec", "Option", "Result",
        ],
        line_comment: Some("//"),
        block_comment: Some(("/*", "*/")),
        hash_line_comment: false,
        dollar_ident: false,
        backtick_strings: false,
        triple_strings: false,
        soft_indent_blocks: false,
    },
}
