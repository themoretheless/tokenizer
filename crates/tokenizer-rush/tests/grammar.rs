use themoretheless_tokenizer_core::{
    HostAnalysisOptions, HostError, HostLanguage, InputLimits, SyntaxKind,
};
use themoretheless_tokenizer_rush::{DESCRIPTOR, ENGINE, ExprKind, StmtKind, parse, parse_with};

fn good(source: &str) -> themoretheless_tokenizer_rush::Parse<'_> {
    let p = parse(source);
    assert!(
        p.is_valid(),
        "{source:?}: {:?}\n{:#?}",
        p.diagnostics,
        p.module
    );
    assert!(p.lexed.is_lossless(source));
    p
}
fn bad(source: &str, code: &str) {
    let p = parse(source);
    assert!(!p.is_valid(), "accepted {source:?}");
    assert!(
        p.diagnostics.iter().any(|d| d.code == code),
        "{source:?}: {:?}",
        p.diagnostics
    );
}

#[test]
fn dedent_ends_if_and_else_belongs_to_if() {
    let p = good("if ready:\n    print(1)\nelse:\n    print(2)\nprint(3)\n");
    assert_eq!(p.module.items.len(), 2);
    let StmtKind::If {
        then_block,
        else_block,
        ..
    } = &p.module.items[0].kind
    else {
        panic!()
    };
    assert_eq!(then_block.stmts.len(), 1);
    assert_eq!(else_block.as_ref().unwrap().stmts.len(), 1);
    assert!(matches!(p.module.items[1].kind, StmtKind::Expr(_)));
}

#[test]
fn nested_blocks_comments_blank_lines_and_crlf() {
    let p = good(
        "if outer:\r\n    if inner:\r\n        print(1)\r\n\r\n    // comment\r\n    print(2)\r\nprint(3)\r\n",
    );
    assert_eq!(p.module.items.len(), 2);
    let StmtKind::If { then_block, .. } = &p.module.items[0].kind else {
        panic!()
    };
    assert_eq!(then_block.stmts.len(), 2);
    bad("if x:\n    print(1)\n  print(2)\n", "unexpected-indent");
    bad("if x:\n\tprint(1)\n", "tab-indent");
}

#[test]
fn function_signatures_preserve_parameters_types_and_body() {
    for source in [
        "fn greet(who: str) -> str { return who; }",
        "fn greet who: str -> str\n    return who\n",
        "fn greet(who: str) -> str:\n    return who\n",
    ] {
        let p = good(source);
        assert_eq!(p.module.items.len(), 1);
        let StmtKind::Function {
            name,
            parameters,
            result,
            body,
        } = &p.module.items[0].kind
        else {
            panic!()
        };
        assert_eq!(name.text, "greet");
        assert_eq!(parameters.len(), 1);
        assert_eq!(parameters[0].name.text, "who");
        assert_eq!(parameters[0].ty.as_ref().unwrap().name.text, "str");
        assert_eq!(result.as_ref().unwrap().name.text, "str");
        assert!(matches!(body.stmts[0].kind, StmtKind::Return(Some(_))));
    }
    good("fn f(xs: list[int], y: map[str, int]) { return xs[0]; }");
    bad("fn f(x, x) {}", "duplicate-parameter");
}

#[test]
fn newline_after_return_is_a_boundary_and_parens_allow_continuation() {
    let p = good("fn f() { return\nprint(1) }");
    let StmtKind::Function { body, .. } = &p.module.items[0].kind else {
        panic!()
    };
    assert_eq!(body.stmts.len(), 2);
    assert!(matches!(body.stmts[0].kind, StmtKind::Return(None)));
    good("let x = (1 +\n 2)\nlet xs = [1,\n2,]\n");
    bad("let x = 1 +\n2", "expected-expression");
}

#[test]
fn errors_are_not_valid_and_recovery_keeps_later_declarations() {
    let source = "let broken = ;\nlet valid = 2;";
    let p = parse(source);
    assert!(!p.is_valid());
    assert_eq!(p.module.items.len(), 2);
    let StmtKind::Declaration { name, value, .. } = &p.module.items[1].kind else {
        panic!()
    };
    assert_eq!(name.text, "valid");
    assert!(matches!(value.kind, ExprKind::Number("2")));
    for source in [
        "fn f( {",
        "let = 2",
        "if x",
        "else {}",
        "match x {}",
        "let x = (1",
        "fn f() {",
        "let x = [1 2]",
    ] {
        assert!(!parse(source).is_valid(), "accepted {source}");
    }
}

#[test]
fn loops_retain_binding_and_iterable() {
    for keyword in ["for", "foreach"] {
        let source = format!("{keyword} item in items\n    yield item\nprint(1)");
        let p = good(&source);
        assert_eq!(p.module.items.len(), 2);
        let StmtKind::For {
            binding,
            iterable,
            body,
        } = &p.module.items[0].kind
        else {
            panic!()
        };
        assert_eq!(binding.text, "item");
        assert!(matches!(iterable.kind, ExprKind::Name(_)));
        assert!(matches!(body.stmts[0].kind, StmtKind::Yield(_)));
    }
    bad("break", "loop-control-outside-loop");
    bad("return 1", "return-outside-function");
    bad("while x { fn f() { break; } }", "loop-control-outside-loop");
    good("while ready { if done { break; } continue; }");
}

#[test]
fn pipes_have_ordered_stages_and_correct_comparison_precedence() {
    let p = good("ls | where size > 0 | select name");
    assert_eq!(p.module.items.len(), 1);
    let StmtKind::Expr(expr) = &p.module.items[0].kind else {
        panic!()
    };
    let ExprKind::Pipeline { input, stages } = &expr.kind else {
        panic!("{expr:?}")
    };
    assert!(matches!(input.kind, ExprKind::Name(_)));
    assert_eq!(stages.len(), 2);
    let ExprKind::Call { callee, arguments } = &stages[0].kind else {
        panic!()
    };
    assert!(matches!(&callee.kind,ExprKind::Name(n) if n.text=="where"));
    assert!(matches!(
        &arguments[0].kind,
        ExprKind::Binary { operator: ">", .. }
    ));
    good("let rows = ls(\"/docs\").where(size > 100).select(name)");
    bad("ls | 42", "invalid-pipeline-stage");
}

#[test]
fn match_keeps_scrutinee_and_patterns() {
    let p = good("let result = match x { 1 => 2, _ => 3 }");
    let StmtKind::Declaration { value, .. } = &p.module.items[0].kind else {
        panic!()
    };
    let ExprKind::Match { value, arms } = &value.kind else {
        panic!()
    };
    assert!(matches!(&value.kind,ExprKind::Name(n) if n.text=="x"));
    assert_eq!(arms.len(), 2);
    assert!(matches!(arms[0].pattern.kind, ExprKind::Number("1")));
    assert!(matches!(&arms[1].pattern.kind,ExprKind::Name(n) if n.text=="_"));
    good("let result = match x {\n  1 => 2\n  _ => 3\n}");
    bad("match x { _ => 1, 2 => 3 }", "unreachable-pattern");
    bad("match x { 1 + 2 => 3 }", "invalid-pattern");
}

#[test]
fn exact_keywords_comments_unicode_and_numbers() {
    assert_eq!(DESCRIPTOR.extensions, &[".r"]);
    let p = good("// comment\nfn f() { let привет = 1+2*3; return привет; } /* done */");
    assert!(
        p.lexed
            .tokens
            .iter()
            .any(|t| t.kind == SyntaxKind::LineComment)
    );
    assert!(
        p.lexed
            .tokens
            .iter()
            .any(|t| &p.source[t.span.range()] == "привет")
    );
    let p = good("let x = 1+2*3");
    let StmtKind::Declaration { value, .. } = &p.module.items[0].kind else {
        panic!()
    };
    let ExprKind::Binary {
        operator: "+",
        right,
        ..
    } = &value.kind
    else {
        panic!("{value:?}")
    };
    assert!(matches!(right.kind, ExprKind::Binary { operator: "*", .. }));
    good("let x = 2**3**2; let y = 1.25e-2");
    bad("let x = 1e+", "invalid-number");
    bad("let x = 2foo", "invalid-number");
    bad("# comment", "invalid-character");
    bad("let x = \"open\nlet y=2", "unclosed-string");
    let upper = themoretheless_tokenizer_rush::lex("RETURN");
    assert_eq!(upper.tokens[0].kind, SyntaxKind::Identifier);
    bad("async fn f() {}", "unsupported-syntax");
}

#[test]
fn host_layers_share_validity_and_all_entrypoints_honor_limits() {
    let opts = HostAnalysisOptions::default();
    for source in ["let x = ;", "match x {}", "if ready:\nprint(1)"] {
        assert!(!ENGINE.lex(source, &opts).unwrap().valid);
        assert!(!ENGINE.semantic_tokens(source, &opts).unwrap().valid);
        assert!(!ENGINE.diagnose(source, &opts).unwrap().is_empty());
    }
    let tiny =
        HostAnalysisOptions::default().with_limits(InputLimits::conservative().max_input_bytes(1));
    assert!(matches!(
        ENGINE.lex("abc", &tiny),
        Err(HostError::InputTooLarge { .. })
    ));
    assert!(matches!(
        ENGINE.semantic_tokens("abc", &tiny),
        Err(HostError::InputTooLarge { .. })
    ));
    assert!(matches!(
        ENGINE.diagnose("abc", &tiny),
        Err(HostError::InputTooLarge { .. })
    ));
    let silent =
        HostAnalysisOptions::default().with_limits(InputLimits::conservative().max_diagnostics(0));
    assert!(!ENGINE.semantic_tokens("let x = ;", &silent).unwrap().valid);
    assert!(ENGINE.diagnose("let x = ;", &silent).unwrap().is_empty());
    for limits in [
        InputLimits::conservative().max_depth(0),
        InputLimits::conservative().max_tokens(0),
        InputLimits::conservative().max_input_bytes(0),
    ] {
        assert!(!parse_with("x", limits).is_valid());
    }
    let source = format!("{}1{}", "(".repeat(400), ")".repeat(400));
    assert!(!parse(&source).is_valid());
}

#[test]
fn every_utf8_prefix_recovers_with_valid_spans() {
    let samples = [
        "fn f(x: str) -> str { return x; }",
        "if ready:\n    print(\"привет 🦀\")\nprint(2)",
        "let y = match x {1 => [2,3], _ => [4]}",
        "ls | where size > 0 | select name",
        "/* comment */ let x = {\"key\": 1}",
    ];
    for sample in samples {
        for end in (0..=sample.len()).filter(|&i| sample.is_char_boundary(i)) {
            let source = &sample[..end];
            let p = parse(source);
            assert!(p.lexed.is_lossless(source), "{source:?}");
            for d in &p.diagnostics {
                assert!(
                    d.span.start <= d.span.end && source.get(d.span.range()).is_some(),
                    "{source:?}: {d:?}"
                );
            }
        }
    }
}

#[test]
fn precedence_associativity_and_assignment_targets() {
    let p = good("let x = -2**3**2");
    let StmtKind::Declaration { value, .. } = &p.module.items[0].kind else {
        panic!()
    };
    let ExprKind::Unary {
        operator: "-",
        value,
    } = &value.kind
    else {
        panic!()
    };
    let ExprKind::Binary {
        operator: "**",
        right,
        ..
    } = &value.kind
    else {
        panic!()
    };
    assert!(matches!(
        right.kind,
        ExprKind::Binary { operator: "**", .. }
    ));
    let p = good("a = b = 1");
    let StmtKind::Expr(expr) = &p.module.items[0].kind else {
        panic!()
    };
    let ExprKind::Assign { value, .. } = &expr.kind else {
        panic!()
    };
    assert!(matches!(value.kind, ExprKind::Assign { .. }));
    bad("1 = 2", "invalid-assignment-target");
    good("xs[0] += 2; obj.field = 3");
}

#[test]
fn deterministic_malformed_inputs_terminate_and_keep_utf8_spans() {
    let pieces = [
        "fn", "let", "if", "else", "match", "for", "return", "yield", "(", ")", "[", "]", "{", "}",
        "=>", ":", ";", "|", "1", "x", "\n", "    ", " ", "🦀", "\"x", "/*", "//", "=",
    ];
    let mut state = 123456789_u32;
    for _ in 0..1500 {
        let mut source = String::new();
        for _ in 0..24 {
            state = state.wrapping_mul(1664525).wrapping_add(1013904223);
            source.push_str(pieces[(state as usize) % pieces.len()]);
        }
        let parsed = parse(&source);
        assert!(parsed.lexed.is_lossless(&source), "{source:?}");
        for diagnostic in parsed.diagnostics {
            assert!(
                source.get(diagnostic.span.range()).is_some(),
                "{source:?}: {diagnostic:?}"
            );
        }
    }
}

#[test]
fn reserved_expressions_and_negative_literal_patterns() {
    bad("let x = await f()", "unsupported-syntax");
    good("let y = match x { -1 => 0, _ => 1 }");
    good("let y = match x {\n1 => a\n-1 => b\n_ => c\n}");
}

#[test]
fn newlines_cannot_join_incomplete_statement_headers() {
    for source in [
        "let\nx=1",
        "let x\n=1",
        "fn\nf() {}",
        "fn f\n() {}",
        "for x\nin xs {}",
    ] {
        assert!(!parse(source).is_valid(), "accepted {source:?}");
    }
    good("fn f(\n x: int,\n y: int\n) { return x+y; }");
}
