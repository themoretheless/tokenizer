//! Full Perl engine (lex → parse → AST → semantic).

#![forbid(unsafe_code)]

use themoretheless_tokenizer_core::{FullProfile, LanguageId, define_full_engine};

define_full_engine! {
    id: LanguageId::PERL,
    name: "Perl",
    aliases: &["pl"],
    extensions: &[".pl", ".pm", ".t"],
    mime_types: &["text/x-perl"],
    profile: FullProfile {
        keywords: &[
            "if", "elsif", "else", "unless", "while", "until", "for", "foreach", "given", "when",
            "default", "continue", "last", "next", "redo", "goto", "sub", "package", "use", "no",
            "require", "my", "our", "local", "state", "return", "do", "eval", "bless", "ref",
            "defined", "undef", "die", "warn", "print", "printf", "say", "open", "close", "read",
            "write", "seek", "tell", "binmode", "sysopen", "sysread", "syswrite", "pipe", "fork",
            "wait", "exec", "system", "chomp", "chop", "join", "split", "map", "grep", "sort",
            "reverse", "keys", "values", "each", "push", "pop", "shift", "unshift", "splice",
            "exists", "delete",
        ],
        types: &[],
        line_comment: Some("#"),
        block_comment: None,
        hash_line_comment: false,
        dollar_ident: true,
        backtick_strings: false,
        triple_strings: false,
        soft_indent_blocks: false,
    },
}
