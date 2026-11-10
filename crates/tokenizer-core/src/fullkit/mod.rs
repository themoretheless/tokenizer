//! Full multi-language engine kit: lossless lex → recovering parse → AST → semantic.
//!
//! Modularized by responsibility:
//! - [`profile`]: declarative language syntax & semantic keyword configuration
//! - [`lookup`]: compile-time open-addressed $O(1)$ keyword/type hash table
//! - [`lexer`]: lossless byte-level scanner + delimiter balance pass
//! - [`ast`]: zero-copy borrowing syntax tree nodes
//! - [`parser`]: silent-recovery recursive-descent + Pratt expression parser
//! - [`semantic`]: $O(N + M)$ cursor-based AST identifier enrichment
//! - [`engine`]: table-driven [`FullEngine`] and [`crate::define_full_engine!`]

pub mod ast;
pub mod engine;
pub mod lexer;
pub mod lookup;
pub mod parser;
pub mod profile;
pub mod semantic;

pub use ast::{Block, Expr, Item, LitKind, Module, Parse, Stmt};
pub use engine::{FULL_ENGINE_CAPS, FullEngine};
pub use lexer::{LexToken, Lexed, SyntaxKind, lex_full, lex_full_with_lookup, lex_to_host};
pub use lookup::KeywordLookup;
pub use parser::{parse_full, parse_full_with_lookup};
pub use profile::FullProfile;
pub use semantic::{SemanticToken, SemanticTokenization, analyze_full_host, semantic_full};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Span;
    use lookup::classify_linear;

    fn py() -> FullProfile {
        FullProfile {
            keywords: &["def", "return", "if", "else", "class", "for", "while"],
            types: &[],
            line_comment: None,
            block_comment: None,
            hash_line_comment: true,
            dollar_ident: false,
            backtick_strings: false,
            triple_strings: true,
            soft_indent_blocks: true,
            ..FullProfile::DEFAULT
        }
    }

    #[test]
    fn lex_parse_function_js() {
        let profile = FullProfile {
            keywords: &["function", "return", "if", "else"],
            ..FullProfile::default()
        };
        let source = "function add(a, b) { return a + b; }";
        let p = parse_full(source, &profile);
        assert!(p.lexed.is_lossless(source));
        assert!(matches!(
            p.module.items.first(),
            Some(Item::Function { .. })
        ));
        let sem = semantic_full(&p);
        assert!(sem.tokens.iter().any(|t| t.kind == "function"));
    }

    #[test]
    fn python_def() {
        let source = "def f(x):\n    return x\n";
        let p = parse_full(source, &py());
        assert!(p.lexed.is_lossless(source));
        assert!(matches!(
            p.module.items.first(),
            Some(Item::Function { .. })
        ));
    }

    #[test]
    fn profile_driven_semantic_keywords_enrich_declarations() {
        let nim_profile = FullProfile {
            keywords: &["proc", "iterator", "object", "let"],
            fn_keywords: &["proc", "iterator"],
            type_keywords: &["object"],
            hash_line_comment: true,
            soft_indent_blocks: true,
            ..FullProfile::DEFAULT
        };
        let source = "proc greet(x): let msg = x";
        let p = parse_full(source, &nim_profile);
        let sem = semantic_full(&p);
        assert!(
            sem.tokens
                .iter()
                .any(|t| t.kind == "function" && t.span.slice(source) == Some("greet"))
        );
        assert!(
            sem.tokens
                .iter()
                .any(|t| t.kind == "variable" && t.span.slice(source) == Some("msg"))
        );
    }

    #[test]
    fn keyword_lookup_matches_linear_case_insensitively() {
        let profile = FullProfile {
            keywords: &["SELECT", "FROM", "WHERE", "function"],
            types: &["INTEGER", "VARCHAR", "int"],
            ..FullProfile::DEFAULT
        };
        let table = KeywordLookup::from_profile(&profile);
        for sample in [
            "select",
            "SELECT",
            "From",
            "FUNCTION",
            "integer",
            "VarChar",
            "INT",
            "unknown",
            "x",
            "very_long_identifier_name_over_sixty_four_bytes_to_test_mask_cap_behavior_12345",
        ] {
            assert_eq!(
                table.classify(&profile, sample),
                classify_linear(&profile, sample),
                "mismatch for {sample}"
            );
        }
    }

    fn codes(source: &str) -> Vec<(&'static str, usize)> {
        let lexed = lex_full(source, &FullProfile::default());
        lexed
            .diagnostics
            .iter()
            .map(|d| (d.code, d.span.start))
            .collect()
    }

    #[test]
    fn one_mismatched_closer_does_not_cascade() {
        assert_eq!(codes("let a = [f(x)) { }"), vec![("stray-delimiter", 13)]);
    }

    #[test]
    fn a_template_literal_is_one_string_span() {
        let js = FullProfile {
            dollar_ident: true,
            ..FullProfile::default()
        };
        let source = "const t = `x${y}\nz`;";
        let lexed = lex_full(source, &js);
        let strings: Vec<Span> = lexed
            .tokens
            .iter()
            .filter(|t| t.kind == SyntaxKind::StringLit)
            .map(|t| t.span)
            .collect();
        assert_eq!(strings, vec![Span::new(10, 19)]);
        assert_eq!(lexed.diagnostics, Vec::new());

        let open = lex_full("let t = `x;", &js);
        assert!(!open.tokens.iter().any(|t| t.kind == SyntaxKind::StringLit));
        assert_eq!(open.diagnostics, Vec::new());

        let other = lex_full("let t = `x`;", &FullProfile::default());
        assert!(!other.tokens.iter().any(|t| t.kind == SyntaxKind::StringLit));
    }
}
