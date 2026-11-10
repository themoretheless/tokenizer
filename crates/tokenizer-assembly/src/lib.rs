//! Full Assembly engine (lex → parse → AST → semantic).

#![forbid(unsafe_code)]

use themoretheless_tokenizer_core::{FullProfile, LanguageId, define_full_engine};

define_full_engine! {
    id: LanguageId::ASSEMBLY,
    name: "Assembly",
    aliases: &["asm", "nasm"],
    extensions: &[".asm", ".s", ".S"],
    mime_types: &["text/x-asm"],
    profile: FullProfile {
        keywords: &[
            "section", "global", "extern", "db", "dw", "dd", "dq", "resb", "resw", "resd", "resq",
            "equ", "times", "bits", "use16", "use32", "use64", "org", "mov", "add", "sub", "mul",
            "div", "push", "pop", "call", "ret", "jmp", "je", "jne", "jl", "jg", "jle", "jge",
            "ja", "jb", "jae", "jbe", "jz", "jnz", "loop", "cmp", "test", "and", "or", "xor",
            "not", "shl", "shr", "lea", "nop", "int", "syscall",
        ],
        types: &[],
        line_comment: Some(";"),
        block_comment: None,
        hash_line_comment: false,
        dollar_ident: false,
        backtick_strings: false,
        triple_strings: false,
        soft_indent_blocks: false,
    },
}
