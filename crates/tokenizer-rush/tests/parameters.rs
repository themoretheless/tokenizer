use themoretheless_tokenizer_rush::{
    Value, analyze, analyze_calls, analyze_names, analyze_references, evaluate, parse,
};

#[test]
fn named_functions_and_lambdas_destructure_nested_arguments() {
    for source in [
        "fn sum((a,b)) { return a+b }\nsum((2,3))",
        "let sum = ((a,b)) => a+b\nsum((2,3))",
        "let sum = ({point: (x,y), ignored: _}) => x+y\nsum({point: (2,3), ignored: false, extra: 9})",
        "fn sum({point: (x,y)}) { return x+y }\nsum({point: (2,3)})",
        "fn sum((x,y): tuple[number, number]) { return x+y }\nsum((2,3))",
    ] {
        assert!(analyze_names(source, &[]).is_valid(), "{source}");
        assert_eq!(
            evaluate(source, 200).unwrap(),
            Value::Number(5.),
            "{source}"
        );
    }
}

#[test]
fn destructured_bindings_are_lexical_and_work_in_pipelines() {
    let source = "let offset = 10\nlet f = ((x,y)) => z => x+y+z+offset\nf((1,2))(3)";
    assert_eq!(evaluate(source, 200).unwrap(), Value::Number(16.));
    assert_eq!(
        evaluate(
            "[(1,2), (3,4)] | iter | map(((x,y)) => x+y) | collect(2)",
            200
        )
        .unwrap(),
        Value::List(vec![Value::Number(3.), Value::Number(7.)])
    );
    assert!(!analyze("let f = ((x,y)) => x = 2").is_valid());
    assert!(!analyze_names("let f = ((x,y)) => x+y\nx", &[]).is_valid());
}

#[test]
fn duplicate_and_refutable_parameter_patterns_are_rejected() {
    for source in [
        "fn f((x,x)) {}",
        "fn f((x,y), x) {}",
        "let f = ({a:x, b:x}) => x",
        "let f = ((x,y), x) => x",
        "fn f(42) {}",
        "let f = (Some(x)) => x",
        "fn f({a:x, a:y}) {}",
    ] {
        assert!(!parse(source).is_valid(), "{source}");
    }
    assert!(parse("fn f((_,_), _) {}\nlet g = (_,_) => 1").is_valid());
}

#[test]
fn shape_errors_and_arity_remain_distinct() {
    for source in [
        "fn f((x,y)) { return x }\nf((1,2,3))",
        "fn f({x:a}) { return a }\nf({y:1})",
        "let f = ((x,y)) => x\nf([1,2])",
    ] {
        assert!(evaluate(source, 200).is_err(), "{source}");
    }
    assert!(analyze_calls("let f = ((x,y)) => x+y\nf((1,2))").is_valid());
    assert!(!analyze_calls("let f = ((x,y)) => x+y\nf(1,2)").is_valid());
}

#[test]
fn definitions_point_to_bound_names_not_record_keys() {
    let source = "fn f({field: (x,y)}) { return x+y }";
    let (parsed, references) = analyze_references(source);
    assert!(parsed.is_valid());
    assert_eq!(references.len(), 2);
    for reference in references {
        let name = &source[reference.usage.start..reference.usage.end];
        assert!(matches!(name, "x" | "y"));
        assert_eq!(
            reference.definition.unwrap().start,
            source.find(name).unwrap()
        );
    }
}

#[test]
fn argument_shape_errors_point_to_the_defining_module() {
    use themoretheless_tokenizer_rush::{CancellationToken, Program};
    let source = "fn first((x,y)) { return x }\n{first: first}";
    let library = Program::compile(source).unwrap();
    let main = Program::compile("import library\nlibrary.first((1,2,3))").unwrap();
    let error = main
        .run_with_modules(
            200,
            &CancellationToken::default(),
            &[],
            &[],
            &[("library", &library)],
        )
        .unwrap_err();
    assert_eq!(error.module.as_deref(), Some("library"));
    assert_eq!(&source[error.span.start..error.span.end], "(x,y)");
}
