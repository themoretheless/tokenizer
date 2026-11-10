//! Table-driven `scripting` wave-language engines.

/// Full JavaScript engine (lex → parse → AST → semantic).
#[cfg(feature = "javascript")]
pub mod javascript {
    use themoretheless_tokenizer_core::{FullProfile, LanguageId, define_full_engine};

    define_full_engine! {
        id: LanguageId::JAVASCRIPT,
        name: "JavaScript",
        aliases: &["js", "jsx"],
        extensions: &[".js", ".mjs", ".cjs", ".jsx"],
        mime_types: &["text/javascript"],
        profile: FullProfile {
            keywords: &[
                "break",
                "case",
                "catch",
                "class",
                "const",
                "continue",
                "debugger",
                "default",
                "delete",
                "do",
                "else",
                "export",
                "extends",
                "finally",
                "for",
                "function",
                "if",
                "import",
                "in",
                "instanceof",
                "let",
                "new",
                "return",
                "static",
                "super",
                "switch",
                "this",
                "throw",
                "try",
                "typeof",
                "var",
                "void",
                "while",
                "with",
                "yield",
                "async",
                "await",
                "of",
            ],
            types: &[],
            dollar_ident: true,
            ..FullProfile::DEFAULT
        },
    }
}

/// Full TypeScript engine (lex → parse → AST → semantic).
#[cfg(feature = "typescript")]
pub mod typescript {
    use themoretheless_tokenizer_core::{FullProfile, LanguageId, define_full_engine};

    define_full_engine! {
        id: LanguageId::TYPESCRIPT,
        name: "TypeScript",
        aliases: &["ts", "tsx"],
        extensions: &[".ts", ".tsx", ".mts", ".cts"],
        mime_types: &["text/typescript"],
        profile: FullProfile {
            keywords: &[
                "break",
                "case",
                "catch",
                "class",
                "const",
                "continue",
                "debugger",
                "default",
                "delete",
                "do",
                "else",
                "export",
                "extends",
                "finally",
                "for",
                "function",
                "if",
                "import",
                "in",
                "instanceof",
                "let",
                "new",
                "return",
                "static",
                "super",
                "switch",
                "this",
                "throw",
                "try",
                "typeof",
                "var",
                "void",
                "while",
                "with",
                "yield",
                "async",
                "await",
                "of",
                "type",
                "interface",
                "enum",
                "namespace",
                "declare",
                "abstract",
                "implements",
                "private",
                "protected",
                "public",
                "readonly",
                "as",
                "satisfies",
            ],
            types: &[
                "string", "number", "boolean", "any", "void", "never", "unknown", "object", "symbol",
                "bigint",
            ],
            dollar_ident: true,
            ..FullProfile::DEFAULT
        },
    }
}

/// Full Python engine (lex → parse → AST → semantic).
#[cfg(feature = "python")]
pub mod python {
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
            triple_strings: true,
            soft_indent_blocks: true,
            ..FullProfile::DEFAULT
        },
    }
}

/// Full Ruby engine (lex → parse → AST → semantic).
#[cfg(feature = "ruby")]
pub mod ruby {
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
            ..FullProfile::DEFAULT
        },
    }
}

/// Full PHP engine (lex → parse → AST → semantic).
#[cfg(feature = "php")]
pub mod php {
    use themoretheless_tokenizer_core::{FullProfile, LanguageId, define_full_engine};

    define_full_engine! {
        id: LanguageId::PHP,
        name: "PHP",
        aliases: &[],
        extensions: &[".php"],
        mime_types: &["application/x-httpd-php"],
        profile: FullProfile {
            keywords: &[
                "abstract",
                "and",
                "array",
                "as",
                "break",
                "callable",
                "case",
                "catch",
                "class",
                "clone",
                "const",
                "continue",
                "declare",
                "default",
                "do",
                "echo",
                "else",
                "elseif",
                "empty",
                "enddeclare",
                "endfor",
                "endforeach",
                "endif",
                "endswitch",
                "endwhile",
                "extends",
                "final",
                "finally",
                "for",
                "foreach",
                "function",
                "global",
                "goto",
                "if",
                "implements",
                "include",
                "include_once",
                "instanceof",
                "insteadof",
                "interface",
                "isset",
                "list",
                "match",
                "namespace",
                "new",
                "or",
                "print",
                "private",
                "protected",
                "public",
                "readonly",
                "require",
                "require_once",
                "return",
                "static",
                "switch",
                "throw",
                "trait",
                "try",
                "unset",
                "use",
                "var",
                "while",
                "xor",
                "yield",
                "true",
                "false",
                "null",
            ],
            types: &[
                "int", "float", "string", "bool", "array", "object", "void", "mixed",
            ],
            dollar_ident: true,
            ..FullProfile::DEFAULT
        },
    }
}

/// Full Lua engine (lex → parse → AST → semantic).
#[cfg(feature = "lua")]
pub mod lua {
    use themoretheless_tokenizer_core::{FullProfile, LanguageId, define_full_engine};

    define_full_engine! {
        id: LanguageId::LUA,
        name: "Lua",
        aliases: &[],
        extensions: &[".lua"],
        mime_types: &["text/x-lua"],
        profile: FullProfile {
            keywords: &[
                "and", "break", "do", "else", "elseif", "end", "false", "for", "function", "goto",
                "if", "in", "local", "nil", "not", "or", "repeat", "return", "then", "true", "until",
                "while",
            ],
            types: &[],
            line_comment: Some("--"),
            block_comment: Some(("--[[", "]]")),
            ..FullProfile::DEFAULT
        },
    }
}

/// Full Perl engine (lex → parse → AST → semantic).
#[cfg(feature = "perl")]
pub mod perl {
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
            dollar_ident: true,
            ..FullProfile::DEFAULT
        },
    }
}

/// Full Bash engine (lex → parse → AST → semantic).
#[cfg(feature = "bash")]
pub mod bash {
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
            backtick_strings: true,
            ..FullProfile::DEFAULT
        },
    }
}

/// Full PowerShell engine (lex → parse → AST → semantic).
#[cfg(feature = "powershell")]
pub mod powershell {
    use themoretheless_tokenizer_core::{FullProfile, LanguageId, define_full_engine};

    define_full_engine! {
        id: LanguageId::POWERSHELL,
        name: "PowerShell",
        aliases: &["ps1"],
        extensions: &[".ps1", ".psm1"],
        mime_types: &[],
        profile: FullProfile {
            keywords: &[
                "if", "else", "elseif", "foreach", "for", "while", "function", "param", "begin",
                "process", "end", "switch", "return", "throw", "try", "catch", "finally", "filter",
            ],
            types: &[],
            line_comment: Some("#"),
            block_comment: Some(("<#", "#>")),
            ..FullProfile::DEFAULT
        },
    }
}

/// Full R engine (lex → parse → AST → semantic).
#[cfg(feature = "r")]
pub mod r {
    use themoretheless_tokenizer_core::{FullProfile, LanguageId, define_full_engine};

    define_full_engine! {
        id: LanguageId::R,
        name: "R",
        aliases: &[],
        extensions: &[".r", ".R"],
        mime_types: &["text/x-r"],
        profile: FullProfile {
            keywords: &[
                "if", "else", "repeat", "while", "function", "for", "in", "next", "break", "TRUE",
                "FALSE", "NULL", "NA", "Inf", "NaN",
            ],
            types: &[],
            line_comment: Some("#"),
            block_comment: None,
            ..FullProfile::DEFAULT
        },
    }
}

/// Full MATLAB engine (lex → parse → AST → semantic).
#[cfg(feature = "matlab")]
pub mod matlab {
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
            ..FullProfile::DEFAULT
        },
    }
}

/// Full Julia engine (lex → parse → AST → semantic).
#[cfg(feature = "julia")]
pub mod julia {
    use themoretheless_tokenizer_core::{FullProfile, LanguageId, define_full_engine};

    define_full_engine! {
        id: LanguageId::JULIA,
        name: "Julia",
        aliases: &["jl"],
        extensions: &[".jl"],
        mime_types: &["text/x-julia"],
        profile: FullProfile {
            keywords: &[
                "baremodule",
                "begin",
                "break",
                "catch",
                "const",
                "continue",
                "do",
                "else",
                "elseif",
                "end",
                "export",
                "false",
                "finally",
                "for",
                "function",
                "global",
                "if",
                "import",
                "in",
                "isa",
                "let",
                "local",
                "macro",
                "module",
                "quote",
                "return",
                "struct",
                "true",
                "try",
                "using",
                "while",
                "where",
                "abstract",
                "primitive",
                "type",
                "mutable",
            ],
            types: &[],
            line_comment: Some("#"),
            block_comment: Some(("#=", "=#")),
            ..FullProfile::DEFAULT
        },
    }
}
