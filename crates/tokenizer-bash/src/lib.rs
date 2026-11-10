//! Full Bash engine (lex → parse → AST → semantic).

#![forbid(unsafe_code)]

use themoretheless_tokenizer_core::{FullProfile, LanguageId, define_full_engine};

define_full_engine! {
    id: LanguageId::BASH,
    name: "Bash",
    aliases: &["shell", "sh", "zsh"],
    extensions: &[".sh", ".bash", ".zsh"],
    mime_types: &["application/x-sh"],
    profile: FullProfile {
        keywords: &[
            "if", "then", "else", "elif", "fi", "for", "while", "do", "done", "case", "esac",
            "function", "in", "return", "exit", "export", "local", "readonly", "declare",
        ],
        types: &[],
        line_comment: None,
        block_comment: None,
        hash_line_comment: true,
        dollar_ident: false,
        backtick_strings: true,
        triple_strings: false,
        soft_indent_blocks: false,
    },
}
