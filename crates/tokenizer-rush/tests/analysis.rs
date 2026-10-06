use themoretheless_tokenizer_core::InputLimits;
use themoretheless_tokenizer_rush::{analyze, analyze_with, parse};

fn rejects(source: &str, code: &str) {
    assert!(parse(source).is_valid(), "syntax: {source}");
    let result = analyze(source);
    assert!(!result.is_valid(), "accepted: {source}");
    assert!(
        result.diagnostics.iter().any(|d| d.code == code),
        "{:?}",
        result.diagnostics
    );
    for diagnostic in result.diagnostics {
        assert!(
            source
                .get(diagnostic.span.start..diagnostic.span.end)
                .is_some()
        );
    }
}

#[test]
fn immutable_bindings_and_compound_assignments() {
    for op in ["=", "+=", "-=", "*=", "/=", "%="] {
        rejects(
            &format!("const value = 1\nvalue {op} 2"),
            "immutable-binding",
        );
    }
    rejects("fn f() { return 1 }\nf = 2", "immutable-binding");
    rejects("const x = 1\nfn f() { x = 2 }", "immutable-binding");
    assert!(analyze("let mut x = 1\nx += 2").is_valid());
}

#[test]
fn duplicate_names_and_shadowing() {
    for source in [
        "let x = 1\nconst x = 2",
        "fn x() {}\nlet x = 2",
        "fn f(x) { let x = 2 }",
        "for x in items { let x = 2 }",
    ] {
        rejects(source, "duplicate-binding");
    }
    assert!(
        analyze("const x = 1\nif ready { let mut x = 2; x += 1 }\nif other { let x = 3 }")
            .is_valid()
    );
    rejects(
        "const x = 1\nif ready { let x = 2 }\nx = 3",
        "immutable-binding",
    );
}

#[test]
fn traverses_expression_children_and_match_scopes() {
    for expression in [
        "f(x = 2)",
        "[x = 2]",
        "{\"key\": (x = 2)}",
        "items[x = 2]",
        "source | f(x = 2)",
        "match state { 0 => (x = 2), _ => 0 }",
    ] {
        rejects(&format!("const x = 1\n{expression}"), "immutable-binding");
    }
    rejects(
        "const x = 1\nlet y = match value { x => (x = 2) }",
        "immutable-binding",
    );
    rejects(
        "const x = 1\nlet y = match value { x => x }\nx = 2",
        "immutable-binding",
    );
}

#[test]
fn host_names_and_shallow_const_are_supported() {
    assert!(analyze("external(arg)\nlet result = source | where size > 1\nconst record = {}\nrecord.value = 2\nrecord[0] = 3").is_valid());
}

#[test]
fn budgets_and_syntax_errors_preserve_invalid_status() {
    for max_diagnostics in [0, 1, 2] {
        let limits = InputLimits {
            max_diagnostics,
            ..InputLimits::conservative()
        };
        let result = analyze_with("const x = 1\nx = 2\nx = 3\nx = 4", limits);
        assert!(!result.is_valid());
        assert_eq!(result.diagnostics.len(), max_diagnostics);
    }
    let source = "let x = ;\nlet x = 2";
    assert_eq!(analyze(source), parse(source));
}

#[test]
fn let_is_immutable_and_mut_is_explicit() {
    rejects("let x = 1\nx = 2", "immutable-binding");
    assert!(analyze("mut x = 1\nx = 2").is_valid());
    assert!(analyze("let mut x = 1\nx += 2").is_valid());
    assert_eq!(
        themoretheless_tokenizer_rush::evaluate("let x = 2\nx * 3", 100).unwrap(),
        themoretheless_tokenizer_rush::Value::Number(6.)
    );
}

#[test]
fn builtin_calls_count_pipeline_inputs_and_respect_shadowing() {
    use themoretheless_tokenizer_rush::analyze_calls;
    for source in [
        "vec2(1)",
        "[1] | map()",
        "range(0,1,2,3)",
        "if false { sin(1,2) }",
    ] {
        let parsed = analyze_calls(source);
        assert!(
            parsed
                .diagnostics
                .iter()
                .any(|d| d.code == "argument-count"),
            "{source}: {:?}",
            parsed.diagnostics
        );
    }
    for source in [
        "[1] | map(x => x)",
        "range(0,1)",
        "range(0,1,0.1)",
        "let sin = (x,y) => x+y\nsin(1,2)",
        "1 | sin",
    ] {
        assert!(analyze_calls(source).is_valid(), "{source}");
    }
}

#[test]
fn local_function_arity_follows_immutable_aliases() {
    use themoretheless_tokenizer_rush::analyze_calls;
    for source in [
        "fn f(x) { return f() }",
        "const f = (x,y) => x+y\nf(1)",
        "const f = sin\nf(1,2)",
        "fn f(x,y) { return x+y }\n1 | f",
    ] {
        assert!(
            analyze_calls(source)
                .diagnostics
                .iter()
                .any(|d| d.code == "argument-count"),
            "{source}"
        );
    }
    for source in [
        "fn f(x,y) { return x+y }\n1 | f(2)",
        "const f = x => x\nconst g = f\ng(1)",
        "fn f(sin) { return sin(1,2) }",
        "mut f = x => x\nf = (x,y) => x+y\nf(1,2)",
    ] {
        assert!(analyze_calls(source).is_valid(), "{source}");
    }
}

#[test]
fn conditional_functions_preserve_possible_arities() {
    use themoretheless_tokenizer_rush::analyze_calls;
    for source in [
        "(if flag { sin } else { cos })(1, 2)",
        "let f = if flag { () => 0 } else { (x,y) => x+y }; f(1)",
        "let f = if flag { sin } else { cos }; let g = f; 1 | g(2)",
        "let f = if a { sin } else { if b { cos } else { () => 0 } }; f(1,2)",
        "let sin = (x,y) => x+y; let f = if flag { sin } else { cos }; f()",
    ] {
        let result = analyze_calls(source);
        assert!(
            parse(source).is_valid(),
            "{source}: {:?}",
            result.diagnostics
        );
        assert!(
            result
                .diagnostics
                .iter()
                .any(|d| d.code == "argument-count"),
            "{source}: {:?}",
            result.diagnostics
        );
    }
    for source in [
        "let f = if flag { sin } else { cos }; 1 | f",
        "let f = if flag { () => 0 } else { (x,y) => x+y }; f()",
        "let f = if flag { () => 0 } else { (x,y) => x+y }; f(1,2)",
        "fn choose(unknown) { let f = if flag { sin } else { unknown }; return f(1,2) }",
        "mut f = if flag { sin } else { cos }; f = (x,y) => x+y; f(1,2)",
    ] {
        let result = analyze_calls(source);
        assert!(result.is_valid(), "{source}: {:?}", result.diagnostics);
    }
}

#[test]
fn destructuring_preserves_function_signatures_from_literal_values() {
    use themoretheless_tokenizer_rush::analyze_calls;
    for source in [
        "let (f,g) = (sin,cos); f(1,2)",
        "let {run:f} = {run:(x,y)=>x+y}; 1 | f",
        "let ({run:f},_) = ({run:sin},0); f()",
        "let (sin,f) = ((x,y)=>x+y,sin); f(1,2)",
    ] {
        let result = analyze_calls(source);
        assert!(
            parse(source).is_valid(),
            "{source}: {:?}",
            result.diagnostics
        );
        assert!(
            result
                .diagnostics
                .iter()
                .any(|d| d.code == "argument-count"),
            "{source}: {:?}",
            result.diagnostics
        );
    }
    for source in [
        "let (sin,f) = ((x,y)=>x+y,sin); sin(1,2); f(1)",
        "let {run:f} = {run:(x,y)=>x+y}; 1 | f(2)",
        "fn invoke((f,g)) { return f(1,2,3) }",
        "let (f,g) = external(); f(1,2,3)",
    ] {
        let result = analyze_calls(source);
        assert!(result.is_valid(), "{source}: {:?}", result.diagnostics);
    }
}
