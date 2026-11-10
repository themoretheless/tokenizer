//! Full Python engine (lex → parse → AST → semantic).

#![forbid(unsafe_code)]

use themoretheless_tokenizer_core::{FullProfile, LanguageId, define_full_engine};

define_full_engine! {
    id: LanguageId::PYTHON,
    name: "Python",
    aliases: &["py"],
    extensions: &[".py", ".pyi"],
    mime_types: &["text/x-python"],
    profile: FullProfile {
        keywords: &[
            "False", "None", "True", "and", "as", "assert", "async", "await", "break", "class",
            "continue", "def", "del", "elif", "else", "except", "finally", "for", "from", "global",
            "if", "import", "in", "is", "lambda", "nonlocal", "not", "or", "pass", "raise",
            "return", "try", "while", "with", "yield", "match", "case",
        ],
        types: &[],
        line_comment: None,
        block_comment: None,
        hash_line_comment: true,
        dollar_ident: false,
        backtick_strings: false,
        triple_strings: true,
        soft_indent_blocks: true,
    },
}
