use std::rc::Rc;
use themoretheless_tokenizer_rush::{
    CancellationToken, HostFunction, Program, Value, ValueType as T, analyze_host_calls,
};

fn never_execute<'s>(_: &[Value<'s>], _: &CancellationToken) -> Result<Value<'s>, String> {
    panic!("Static analysis must not execute callbacks")
}
fn function(name: &'static str, parameters: Vec<T>, result: T) -> Rc<HostFunction> {
    Rc::new(HostFunction {
        name,
        parameters,
        result,
        callback: never_execute,
    })
}
fn functions() -> Vec<Rc<HostFunction>> {
    vec![
        function("double", vec![T::Number], T::Number),
        function("text", vec![T::Number], T::String),
        function("numbers", vec![T::List(Box::new(T::Number))], T::Null),
        function("pair", vec![T::Tuple(vec![T::Number, T::String])], T::Null),
        function("maybe", vec![], T::Option(Box::new(T::Bool))),
        function(
            "optional_number",
            vec![T::Option(Box::new(T::Number))],
            T::Null,
        ),
        function("flags", vec![], T::List(Box::new(T::Bool))),
    ]
}

#[test]
fn registered_result_types_provide_expression_member_completions_without_execution() {
    use themoretheless_tokenizer_rush::analyze_editor_with_host;
    let registered = vec![
        function("point", vec![], T::Vector(3)),
        function("model", vec![], T::Mesh),
    ];
    for (source, expected) in [
        ("point().", vec!["x", "y", "z"]),
        ("model().", vec!["triangles", "vertices"]),
        ("model().vertices[0].", vec!["x", "y", "z"]),
        ("let alias=point; alias().", vec!["x", "y", "z"]),
        ("fn point()->number { return 1 }; point().", vec![]),
    ] {
        let analysis = analyze_editor_with_host(source, &[], &registered);
        assert!(!analysis.parsed.is_valid());
        assert_eq!(
            analysis.parsed.diagnostics.len(),
            1,
            "{source}: {:?}",
            analysis.parsed.diagnostics
        );
        assert_eq!(analysis.parsed.diagnostics[0].code, "expected-name");
        let completion = analysis
            .member_completions
            .iter()
            .find(|c| c.name_span.start == source.len())
            .unwrap();
        assert_eq!(completion.members, expected, "{source}");
    }
    let analysis = analyze_editor_with_host("point().z", &[], &registered);
    assert!(analysis.parsed.is_valid());
    assert_eq!(analysis.member_completions[0].members, ["x", "y", "z"]);
    let invalid = analyze_editor_with_host("point(1).", &[], &registered);
    assert!(
        invalid
            .parsed
            .diagnostics
            .iter()
            .any(|d| d.code != "expected-name"),
        "Call arity must still be checked"
    );
    let invalid = analyze_editor_with_host("point( .", &[], &registered);
    assert!(invalid.member_completions.is_empty());
}

#[test]
fn inferred_math_results_are_checked_against_host_arguments() {
    let hosts = [function("accept2", vec![T::Vector(2)], T::Null)];
    for source in [
        "accept2(normalize(vec3(1,2,3)))",
        "vec3(1,2,3) | normalize | accept2",
    ] {
        let parsed = analyze_host_calls(source, &hosts);
        assert!(
            parsed.diagnostics.iter().any(|d| d.code == "argument-type"),
            "{source}"
        );
    }
    for source in [
        "accept2(normalize(vec2(1,2)))",
        "vec2(1,2) | normalize | accept2",
    ] {
        assert!(analyze_host_calls(source, &hosts).is_valid(), "{source}");
    }
}

#[test]
fn vector_math_arguments_report_known_type_and_dimension_errors() {
    for (source, code, marked) in [
        ("normalize(true)", "argument-type", "true"),
        ("3 | normalize", "argument-type", "3"),
        ("length([1,2])", "argument-type", "[1,2]"),
        ("cross(vec2(1,2),vec3(1,2,3))", "argument-type", "vec2(1,2)"),
        (
            "dot(vec2(1,2),vec3(1,2,3))",
            "vector-dimensions",
            "vec3(1,2,3)",
        ),
        (
            "let d=dot; vec2(1,2) | d(vec3(1,2,3))",
            "vector-dimensions",
            "vec3(1,2,3)",
        ),
        ("lerp(1,vec2(1,2),0.5)", "argument-type", "vec2(1,2)"),
        (
            "lerp(vec2(1,2),vec3(1,2,3),0.5)",
            "vector-dimensions",
            "vec3(1,2,3)",
        ),
        ("lerp(1,2,true)", "argument-type", "true"),
    ] {
        let parsed = analyze_host_calls(source, &[]);
        let diagnostic = parsed
            .diagnostics
            .iter()
            .find(|d| d.code == code)
            .unwrap_or_else(|| panic!("{source}: {:?}", parsed.diagnostics));
        assert_eq!(
            &source[diagnostic.span.start..diagnostic.span.end],
            marked,
            "{source}"
        );
        assert!(
            themoretheless_tokenizer_rush::evaluate(source, 1000).is_err(),
            "{source}"
        );
    }
    for source in [
        "normalize(vec2(1,2))",
        "dot(vec4(1,2,3,4),vec4(4,3,2,1))",
        "cross(vec3(1,0,0),vec3(0,1,0))",
        "lerp(1,2,0.5)",
        "vec2(1,2) | lerp(vec2(3,4),0.5) | normalize",
        "fn f(a,b) { return dot(a,b) }",
        "fn normalize(value:bool)->bool { return value }; normalize(true)",
    ] {
        let parsed = analyze_host_calls(source, &[]);
        assert!(parsed.is_valid(), "{source}: {:?}", parsed.diagnostics);
        assert!(
            themoretheless_tokenizer_rush::evaluate(source, 1000).is_ok(),
            "{source}"
        );
    }
}

#[test]
fn transformation_contracts_match_runtime_types_and_accept_angle_values() {
    for source in [
        "translation(vec2(1,2))",
        "scaling(3)",
        "rotation_x(true)",
        "axis_angle(vec2(1,2),degrees(90))",
        "axis_angle(vec3(0,1,0),false)",
        "rotation_matrix(identity())",
        "slerp(identity(),identity(),0.5)",
        "transform_point(vec3(1,2,3),vec3(1,2,3))",
        "transform_direction(identity(),vec2(1,2))",
        "degrees(vec3(1,2,3))",
        "1 | transform(identity())",
        "let t=translation; vec2(1,2) | t",
    ] {
        let parsed = analyze_host_calls(source, &[]);
        assert!(
            parsed.diagnostics.iter().any(|d| d.code == "argument-type"),
            "{source}: {:?}",
            parsed.diagnostics
        );
        assert!(
            themoretheless_tokenizer_rush::evaluate(source, 1000).is_err(),
            "{source}"
        );
    }
    for source in [
        "const m:mat4=translation(vec3(1,2,3)); transform_point(m,vec3(1,2,3))",
        "const q:quat=axis_angle(vec3(0,1,0),degrees(90)); rotation_matrix(slerp(q,q,0.5))",
        "rotation_x(radians(1))",
        "rotation_y(0.5)",
        "let r=rotation_z; degrees(90) | r",
        "grid_mesh([0,1],[0,1],(x,y)=>vec3(x,y,0)) | transform(identity())",
        "polygon([vec2(0,0),vec2(1,0),vec2(0,1)]) | rotate(degrees(90)) | translate(vec2(1,2))",
        "fn scaling(x:bool)->bool { return x }; scaling(true)",
        "fn transform_unknown(a,b) { return transform_point(a,b) }",
    ] {
        let parsed = analyze_host_calls(source, &[]);
        assert!(parsed.is_valid(), "{source}: {:?}", parsed.diagnostics);
        assert!(
            themoretheless_tokenizer_rush::evaluate(source, 10000).is_ok(),
            "{source}"
        );
    }
    let hosts = [function("accept_matrix", vec![T::Matrix4], T::Null)];
    assert!(analyze_host_calls("accept_matrix(identity())", &hosts).is_valid());
    assert!(
        analyze_host_calls("accept_matrix(axis_angle(vec3(0,1,0),0))", &hosts)
            .diagnostics
            .iter()
            .any(|d| d.code == "argument-type")
    );
}

#[test]
fn composition_preserves_matrix_quaternion_and_angle_contracts() {
    let registrations = [
        function("matrix_only", vec![T::Matrix4], T::Null),
        function("quaternion_only", vec![T::Quaternion], T::Null),
        function("angle_only", vec![T::Angle], T::Null),
    ];
    for (expression, accepted, rejected) in [
        (
            "translation(vec3(1,2,3))*rotation_z(degrees(90))",
            "matrix_only",
            "quaternion_only",
        ),
        (
            "axis_angle(vec3(0,1,0),0)*axis_angle(vec3(1,0,0),0)",
            "quaternion_only",
            "matrix_only",
        ),
        ("degrees(90)+radians(1)", "angle_only", "matrix_only"),
        ("degrees(90)-degrees(30)", "angle_only", "matrix_only"),
        ("degrees(90)*2", "angle_only", "matrix_only"),
        ("2*degrees(90)", "angle_only", "matrix_only"),
        ("degrees(90)/2", "angle_only", "matrix_only"),
    ] {
        let valid = format!("let composed={expression}; {accepted}(composed)");
        let analysis = analyze_host_calls(&valid, &registrations);
        assert!(analysis.is_valid(), "{valid}: {:?}", analysis.diagnostics);
        let invalid = format!("{expression} | {rejected}");
        assert!(
            analyze_host_calls(&invalid, &registrations)
                .diagnostics
                .iter()
                .any(|d| d.code == "argument-type"),
            "{invalid}"
        );
        assert!(
            themoretheless_tokenizer_rush::evaluate(expression, 1000).is_ok(),
            "{expression}"
        );
    }
    let source = "const m:mat4=translation(vec3(1,2,3))*identity(); const q:quat=axis_angle(vec3(0,1,0),0)*axis_angle(vec3(1,0,0),0); const a:angle=degrees(90)/2; transform_point(m,vec3(0,0,0))";
    assert!(analyze_host_calls(source, &[]).is_valid());
    assert_eq!(
        themoretheless_tokenizer_rush::evaluate(source, 1000).unwrap(),
        Value::Vector(vec![1.0, 2.0, 3.0])
    );
}

#[test]
fn invalid_matrix_quaternion_and_angle_arithmetic_is_diagnosed() {
    for source in [
        "identity()+identity()",
        "identity()*2",
        "2/identity()",
        "axis_angle(vec3(0,1,0),0)+axis_angle(vec3(1,0,0),0)",
        "identity()*axis_angle(vec3(0,1,0),0)",
        "degrees(90)*degrees(30)",
        "2/degrees(30)",
        "degrees(90)+1",
        "degrees(90)%2",
        "degrees(90)**2",
    ] {
        let parsed = analyze_host_calls(source, &[]);
        let diagnostic = parsed
            .diagnostics
            .iter()
            .find(|d| d.code == "math-operands")
            .unwrap_or_else(|| panic!("{source}: {:?}", parsed.diagnostics));
        assert_eq!(&source[diagnostic.span.start..diagnostic.span.end], source);
        assert!(
            themoretheless_tokenizer_rush::evaluate(source, 1000).is_err(),
            "{source}"
        );
    }
    for source in [
        "identity()==identity()",
        "identity()!=axis_angle(vec3(0,1,0),0)",
        "degrees(90)==degrees(90)",
        "fn f(x) { return identity()*x }",
        "fn identity()->number { return 2 }; identity()+identity()",
    ] {
        let parsed = analyze_host_calls(source, &[]);
        assert!(parsed.is_valid(), "{source}: {:?}", parsed.diagnostics);
        assert!(themoretheless_tokenizer_rush::evaluate(source, 1000).is_ok());
    }
}
#[test]
fn unary_sign_contract_matches_runtime_and_keeps_unknown_parameters_open() {
    for operand in [
        "true",
        "'text'",
        "null",
        "[1]",
        "{x:1}",
        "identity()",
        "degrees(90)",
        "axis_angle(vec3(0,1,0),0)",
    ] {
        for sign in ["+", "-"] {
            let source = format!("{sign}{operand}");
            let parsed = analyze_host_calls(&source, &[]);
            let diagnostic = parsed
                .diagnostics
                .iter()
                .find(|d| d.code == "unary-operand")
                .unwrap_or_else(|| panic!("{source}: {:?}", parsed.diagnostics));
            assert_eq!(&source[diagnostic.span.start..diagnostic.span.end], operand);
            assert!(themoretheless_tokenizer_rush::evaluate(&source, 1000).is_err());
        }
    }
    for source in [
        "-2",
        "+2",
        "-vec2(1,2)",
        "+vec3(1,2,3)",
        "-vec4(1,2,3,4)",
        "fn f(x) { return -x }; f(vec2(1,2))",
        "fn degrees(x)->number { return x }; -degrees(1)",
    ] {
        let parsed = analyze_host_calls(source, &[]);
        assert!(parsed.is_valid(), "{source}: {:?}", parsed.diagnostics);
        assert!(themoretheless_tokenizer_rush::evaluate(source, 1000).is_ok());
    }
}

#[test]
fn ordered_comparison_checks_each_known_operand_without_restricting_equality() {
    for operand in [
        "vec2(1,2)",
        "identity()",
        "degrees(90)",
        "axis_angle(vec3(0,1,0),0)",
        "true",
        "'text'",
        "[1]",
        "{x:1}",
    ] {
        for operator in ["<", ">", "<=", ">="] {
            for source in [
                format!("{operand} {operator} 1"),
                format!("1 {operator} {operand}"),
            ] {
                let parsed = analyze_host_calls(&source, &[]);
                let errors: Vec<_> = parsed
                    .diagnostics
                    .iter()
                    .filter(|d| d.code == "comparison-operand")
                    .collect();
                assert_eq!(errors.len(), 1, "{source}: {:?}", parsed.diagnostics);
                let span = errors[0].span;
                assert_eq!(&source[span.start..span.end], operand);
                assert!(themoretheless_tokenizer_rush::evaluate(&source, 1000).is_err());
            }
        }
    }
    for source in [
        "1 < 2",
        "2 >= 1",
        "1 <= 1",
        "2 > 1",
        "vec2(1,2)==vec2(1,2)",
        "degrees(90)!=degrees(30)",
        "fn f(x) { return x < 1 }; f(0)",
        "fn degrees(x)->number { return x }; degrees(1)<2",
    ] {
        let parsed = analyze_host_calls(source, &[]);
        assert!(parsed.is_valid(), "{source}: {:?}", parsed.diagnostics);
        assert!(themoretheless_tokenizer_rush::evaluate(source, 1000).is_ok());
    }
    let source = "fn f(x) { return x < degrees(90) }";
    let parsed = analyze_host_calls(source, &[]);
    assert_eq!(
        parsed
            .diagnostics
            .iter()
            .filter(|d| d.code == "comparison-operand")
            .count(),
        1
    );
}

#[test]
fn obvious_types_nested_literals_and_host_results_are_checked_without_execution() {
    for source in [
        "double(true)",
        "let (f,_) = (double,0); f(true)",
        "let {run:f} = {run:double}; true | f",
        "let (double,f) = (text,double); f('wrong')",
        "double('x')",
        "double(null)",
        "double({x:1})",
        "double(x => x)",
        "double(1 == 1)",
        "double(text(1))",
        "numbers([1,true])",
        "numbers([unknown,true])",
        "pair((1,true))",
        "pair((1,'x',3))",
        "double(if true { 'a' } else { 'b' })",
    ] {
        let parsed = analyze_host_calls(source, &functions());
        assert!(
            parsed.diagnostics.iter().any(|d| d.code == "argument-type"),
            "{source}: {:?}",
            parsed.diagnostics
        );
    }
    for source in [
        "double(1+2)",
        "double(-2)",
        "numbers([])",
        "numbers([1,2])",
        "pair((1,'x'))",
    ] {
        assert!(
            analyze_host_calls(source, &functions()).is_valid(),
            "{source}"
        );
    }
}

#[test]
fn aliases_and_pipeline_results_respect_lexical_shadowing() {
    for source in [
        "const f = double\nconst g = f\ng(false)",
        "true | double",
        "1 | text | double",
        "1 | text() | double()",
    ] {
        assert!(
            !analyze_host_calls(source, &functions()).is_valid(),
            "{source}"
        );
    }
    for source in [
        "let double = x => x\ndouble(true)",
        "fn f(double) { return double(true) }",
        "mut f = double\nf = x => x\nf(true)",
        "1 | double | text",
    ] {
        assert!(
            analyze_host_calls(source, &functions()).is_valid(),
            "{source}"
        );
    }
    let source = "1 | text | double";
    let parsed = analyze_host_calls(source, &functions());
    let diagnostic = parsed
        .diagnostics
        .iter()
        .find(|d| d.code == "argument-type")
        .unwrap();
    assert_eq!(&source[diagnostic.span.start..diagnostic.span.end], "text");
}

#[test]
fn unknown_values_and_overlapping_contracts_are_deferred_to_runtime() {
    // A list can be empty, and Option contracts all accept None.
    for source in [
        "numbers(flags())",
        "optional_number(maybe())",
        "double(unknown)",
        "double(if condition { 1 } else { 'x' })",
        "numbers([unknown])",
        "mut xs=[1]\nxs=[true]\nnumbers(xs)",
    ] {
        assert!(
            analyze_host_calls(source, &functions()).is_valid(),
            "{source}"
        );
    }
    let source = "mut xs=[1]\nxs=[true]\nnumbers(xs)";
    let error = Program::compile(source)
        .unwrap()
        .run_with_host(1000, &CancellationToken::default(), &[], &functions())
        .unwrap_err();
    assert!(error.message.contains("signature"));
}

#[test]
fn host_editor_uses_runtime_registrations_for_checks_and_completion() {
    use themoretheless_tokenizer_rush::analyze_editor_with_host;
    let registered = functions();
    let source = "let f = double\nf(true)\nmissing\ninput";
    let editor = analyze_editor_with_host(source, &["input"], &registered);
    assert!(
        editor
            .parsed
            .diagnostics
            .iter()
            .any(|d| d.code == "argument-type")
    );
    let unknown: Vec<_> = editor
        .parsed
        .diagnostics
        .iter()
        .filter(|d| d.code == "unknown-name")
        .collect();
    assert_eq!(unknown.len(), 1);
    assert_eq!(
        &source[unknown[0].span.start..unknown[0].span.end],
        "missing"
    );
    assert!(editor.bindings.iter().any(|b| b.name == "f"));
    assert!(editor.references.iter().any(|r| r.definition.is_some()));
    assert_eq!(editor.functions.len(), registered.len());
    for (actual, expected) in editor.functions.iter().zip(&registered) {
        assert!(Rc::ptr_eq(actual, expected));
        let arguments = vec!["null"; expected.parameters.len() + 1].join(",");
        let source = format!("{}({arguments})", expected.name);
        let checked = analyze_editor_with_host(&source, &[], &registered);
        assert!(
            checked
                .parsed
                .diagnostics
                .iter()
                .any(|d| d.code == "argument-count")
        );
        assert!(
            Program::compile(&source)
                .unwrap()
                .run_with_host(1000, &Default::default(), &[], &registered)
                .unwrap_err()
                .message
                .contains("argument")
        );
    }
    assert!(
        analyze_editor_with_host("let double = x => x; double(true)", &[], &registered)
            .parsed
            .is_valid()
    );
}

#[test]
fn editor_rejects_the_same_registration_conflicts_as_runtime() {
    use themoretheless_tokenizer_rush::analyze_editor_with_host;
    for (registered, inputs) in [
        (vec![function("sin", vec![], T::Null)], vec![]),
        (
            vec![
                function("f", vec![], T::Null),
                function("f", vec![], T::Null),
            ],
            vec![],
        ),
        (vec![function("f", vec![], T::Null)], vec!["f"]),
        (vec![], vec!["x", "x"]),
        (vec![], vec!["sin"]),
    ] {
        let editor = analyze_editor_with_host("0", &inputs, &registered);
        assert!(
            editor
                .parsed
                .diagnostics
                .iter()
                .any(|d| d.code == "duplicate-host-name")
        );
        let values: Vec<_> = inputs.iter().map(|name| (*name, 0.)).collect();
        assert!(
            Program::compile("0")
                .unwrap()
                .run_with_host(1000, &Default::default(), &values, &registered)
                .is_err()
        );
    }
}

#[test]
fn vector_constructor_shapes_respect_local_shadowing() {
    let functions = [function("accept", vec![T::Vector(3)], T::Null)];
    assert!(analyze_host_calls("accept(vec3(1,2,3))", &functions).is_valid());
    assert!(!analyze_host_calls("accept(vec2(1,2))", &functions).is_valid());
    assert!(!analyze_host_calls("accept(vec4(1,2,3,4))", &functions).is_valid());
    // A local callable named vec2 has no statically known result type.
    assert!(
        analyze_host_calls(
            "let vec2 = (x,y) => vec3(x,y,0); accept(vec2(1,2))",
            &functions
        )
        .is_valid()
    );
}

#[test]
fn vector_constructor_results_flow_through_pipelines() {
    let functions = [function("accept", vec![T::Vector(3)], T::Null)];
    assert!(analyze_host_calls("1 | vec3(2,3) | accept", &functions).is_valid());
    assert!(!analyze_host_calls("1 | vec2(2) | accept", &functions).is_valid());
    assert!(!analyze_host_calls("1 | vec4(2,3,4) | accept", &functions).is_valid());
    assert!(
        analyze_host_calls(
            "let vec2 = (x,y) => vec3(x,y,0); 1 | vec2(2) | accept",
            &functions
        )
        .is_valid()
    );
}

#[test]
fn nested_pipeline_results_and_vector_components_have_known_types() {
    let functions = [
        function("number", vec![T::Number], T::Null),
        function("vector", vec![T::Vector(3)], T::Null),
        function("string", vec![T::String], T::Null),
    ];
    for source in ["number(vec3(1,2,3).z)", "vector(1 | vec3(2,3))"] {
        assert!(
            analyze_host_calls(source, &functions).is_valid(),
            "{source}"
        );
    }
    for source in [
        "string(vec3(1,2,3).z)",
        "vector(1 | vec2(2))",
        "number(1 | vec3(2,3))",
    ] {
        assert!(
            !analyze_host_calls(source, &functions).is_valid(),
            "{source}"
        );
    }
}

#[test]
fn known_vector_members_report_the_field_span() {
    use themoretheless_tokenizer_rush::analyze_calls;
    for (source, field) in [("vec2(1,2).z", "z"), ("vec3(1,2,3).width", "width")] {
        let parsed = analyze_calls(source);
        let diagnostic = parsed
            .diagnostics
            .iter()
            .find(|d| d.code == "vector-component")
            .unwrap();
        assert_eq!(&source[diagnostic.span.start..diagnostic.span.end], field);
    }
    assert!(analyze_calls("vec4(1,2,3,4).w").is_valid());
    assert!(analyze_calls("let vec2 = (x,y) => {z:0}; vec2(1,2).z").is_valid());
    let functions = [function("point", vec![], T::Vector(2))];
    assert!(!analyze_host_calls("point().z", &functions).is_valid());
    assert!(analyze_host_calls("point().x", &functions).is_valid());
}

#[test]
fn immutable_bindings_preserve_known_types_without_leaking_across_scopes() {
    let functions = [function("accept", vec![T::Vector(3)], T::Null)];
    for source in [
        "let v = vec2(1,2); accept(v)",
        "let v = vec2(1,2); let w = v; w.z",
    ] {
        assert!(
            !analyze_host_calls(source, &functions).is_valid(),
            "{source}"
        );
    }
    for source in [
        "let v = vec2(1,2); if true { let v = vec3(1,2,3); accept(v) }; v.x",
        "let v = vec3(1,2,3); if true { let v = v; accept(v) }",
        "mut v = vec2(1,2); v = vec3(1,2,3); accept(v)",
    ] {
        assert!(
            analyze_host_calls(source, &functions).is_valid(),
            "{source}"
        );
    }
}

#[test]
fn editor_exposes_known_vector_fields_for_immutable_bindings() {
    let (_, _, bindings) = themoretheless_tokenizer_rush::analyze_editor(
        "let v = vec2(1,2); let w = v; mut unknown = vec3(1,2,3)",
    );
    assert_eq!(
        bindings.iter().find(|b| b.name == "v").unwrap().members,
        ["x", "y"]
    );
    assert_eq!(
        bindings.iter().find(|b| b.name == "w").unwrap().members,
        ["x", "y"]
    );
    assert!(
        bindings
            .iter()
            .find(|b| b.name == "unknown")
            .unwrap()
            .members
            .is_empty()
    );
}

#[test]
fn editor_preserves_bindings_at_an_unfinished_final_member() {
    let source = "let v = vec3(1,2,3); v.";
    let (parsed, _, bindings) = themoretheless_tokenizer_rush::analyze_editor(source);
    assert!(!parsed.is_valid());
    let binding = bindings.iter().find(|b| b.name == "v").unwrap();
    assert_eq!(binding.members, ["x", "y", "z"]);
    assert!(binding.visible.end > source.len());
    assert!(Program::compile(source).is_err());
    let (_, _, bindings) =
        themoretheless_tokenizer_rush::analyze_editor("let v = vec3(1,2,3); @ v.");
    assert!(bindings.is_empty());
}

#[test]
fn vector_arithmetic_preserves_result_dimensions() {
    let functions = [function("accept", vec![T::Vector(3)], T::Null)];
    for expression in [
        "vec2(1,2) + vec2(3,4)",
        "vec2(1,2) - vec2(3,4)",
        "vec2(1,2) * 2",
        "2 * vec2(1,2)",
        "vec2(1,2) / 2",
    ] {
        let source = format!("accept({expression})");
        assert!(
            !analyze_host_calls(&source, &functions).is_valid(),
            "{source}"
        );
    }
    assert!(analyze_host_calls("accept(vec3(1,2,3) * 2)", &functions).is_valid());
    let (_, _, bindings) = themoretheless_tokenizer_rush::analyze_editor("let v = 2 * vec3(1,2,3)");
    assert_eq!(bindings[0].members, ["x", "y", "z"]);
}

#[test]
fn known_invalid_vector_operators_are_rejected_before_execution() {
    use themoretheless_tokenizer_rush::analyze_calls;
    for source in [
        "vec2(1,2) + vec3(1,2,3)",
        "vec2(1,2) * vec2(1,2)",
        "2 / vec2(1,2)",
        "vec2(1,2) + 1",
        "vec2(1,2) % 2",
        "vec2(1,2) ** 2",
    ] {
        let parsed = analyze_calls(source);
        assert!(
            parsed
                .diagnostics
                .iter()
                .any(|d| d.code == "vector-operands"),
            "{source}"
        );
        assert!(
            Program::compile(source)
                .unwrap()
                .run(1000, &Default::default(), &[])
                .is_err()
        );
    }
    for source in [
        "vec2(1,2) + vec2(3,4)",
        "vec2(1,2) / 2",
        "2 * vec2(1,2)",
        "vec2(1,2) == vec3(1,2,3)",
        "fn f(x) { return x + vec2(1,2) }",
    ] {
        assert!(analyze_calls(source).is_valid(), "{source}");
    }
}

#[test]
fn destructuring_preserves_types_before_introducing_shadowing_names() {
    let functions = [function("accept", vec![T::Vector(3)], T::Null)];
    for source in [
        "let (v,n) = (vec2(1,2),0); accept(v)",
        "let {point:v} = {point:vec2(1,2)}; v.z",
        "let ((v,n),m) = ((vec2(1,2),0),1); accept(v)",
        "let original = vec2(1,2); let (original,v) = (vec3(1,2,3),original); accept(v)",
    ] {
        assert!(
            !analyze_host_calls(source, &functions).is_valid(),
            "{source}"
        );
    }
    let (_, _, bindings) =
        themoretheless_tokenizer_rush::analyze_editor("let {point:v} = {point:vec3(1,2,3)}; v.");
    assert_eq!(
        bindings.iter().find(|b| b.name == "v").unwrap().members,
        ["x", "y", "z"]
    );
}

#[test]
fn unary_vectors_retain_types_for_host_checks_and_completion() {
    let functions = [function("accept", vec![T::Vector(3)], T::Null)];
    assert!(!analyze_host_calls("accept(-vec2(1,2))", &functions).is_valid());
    assert!(analyze_host_calls("accept(+vec3(1,2,3))", &functions).is_valid());
    let (_, _, bindings) =
        themoretheless_tokenizer_rush::analyze_editor("let v = -vec3(1,2,3); v.");
    assert_eq!(bindings[0].members, ["x", "y", "z"]);
}

#[test]
fn annotated_parameters_use_runtime_types_for_analysis_and_editor_fields() {
    use themoretheless_tokenizer_rush::{analyze_calls, analyze_editor};
    assert!(!analyze_calls("fn f(v: vec2) { return v.z }").is_valid());
    assert!(analyze_calls("fn f(v: vec3) { return v.z }").is_valid());
    let functions = [function("accept", vec![T::Vector(3)], T::Null)];
    assert!(!analyze_host_calls("fn f(v: vec2) { return accept(v) }", &functions).is_valid());
    let (_, _, bindings) = analyze_editor("fn f(v: vec3) { return v.x }");
    assert_eq!(
        bindings.iter().find(|b| b.name == "v").unwrap().members,
        ["x", "y", "z"]
    );
    assert!(
        analyze_calls("fn f(v: vec2) { if true { let v = vec3(1,2,3); v.z }; return v.x }")
            .is_valid()
    );
}

#[test]
fn annotated_tuple_parameters_propagate_nested_component_types() {
    use themoretheless_tokenizer_rush::{analyze_calls, analyze_editor};
    let bad = "fn f((v,n): tuple[vec2,number]) { return v.z }";
    assert!(!analyze_calls(bad).is_valid());
    let good = "fn f(((v,n),_): tuple[tuple[vec3,number],bool]) { return v.z + n }";
    assert!(analyze_calls(good).is_valid());
    let (_, _, bindings) = analyze_editor(good);
    assert_eq!(
        bindings.iter().find(|b| b.name == "v").unwrap().members,
        ["x", "y", "z"]
    );
    assert!(
        bindings
            .iter()
            .find(|b| b.name == "n")
            .unwrap()
            .members
            .is_empty()
    );
    assert!(!bindings.iter().any(|b| b.name == "_"));
}

#[test]
fn declaration_annotations_check_initializers_without_guessing_unknown_results() {
    use themoretheless_tokenizer_rush::{analyze_calls, analyze_editor};
    for source in [
        "let v: vec3 = vec2(1,2)",
        "mut n: number = true",
        "let xs: list[number] = [1,false]",
    ] {
        assert!(
            analyze_calls(source)
                .diagnostics
                .iter()
                .any(|d| d.code == "annotation-type"),
            "{source}"
        );
        assert!(
            Program::compile(source)
                .unwrap()
                .run(1000, &Default::default(), &[])
                .is_err()
        );
    }
    let source = "fn make() { return vec3(1,2,3) }; let v: vec3 = make(); v.z";
    assert!(analyze_calls(source).is_valid());
    let (_, _, bindings) = analyze_editor(source);
    assert_eq!(
        bindings.iter().find(|b| b.name == "v").unwrap().members,
        ["x", "y", "z"]
    );
}

#[test]
fn unsupported_annotations_are_reported_even_in_unused_functions() {
    use themoretheless_tokenizer_rush::analyze_calls;
    for source in [
        "let x: unknown = 1",
        "let xs: list[number,bool] = []",
        "fn f(v: vec3[number]) { return v }",
        "fn f(v: list[unknown]) { return v }",
        "fn f() -> unknown { return 1 }",
    ] {
        let parsed = analyze_calls(source);
        assert!(
            parsed
                .diagnostics
                .iter()
                .any(|d| d.code == "unsupported-annotation"),
            "{source}: {:?}",
            parsed.diagnostics
        );
    }
    assert!(analyze_calls("fn f(v: list[vec3]) -> number { return 0 }").is_valid());
}

#[test]
fn explicit_returns_are_checked_against_the_innermost_function_annotation() {
    use themoretheless_tokenizer_rush::analyze_calls;
    for source in [
        "fn f() -> number { return true }",
        "fn f() -> vec3 { return vec2(1,2) }",
        "fn f() -> number { return }",
    ] {
        assert!(
            analyze_calls(source)
                .diagnostics
                .iter()
                .any(|d| d.code == "return-type"),
            "{source}"
        );
    }
    for source in [
        "fn f() -> number { fn g() -> bool { return true }; return 1 }",
        "fn f() -> number { fn g() { return true }; return 1 }",
        "fn f(x) -> number { return x }",
    ] {
        assert!(analyze_calls(source).is_valid(), "{source}");
    }
}

#[test]
fn null_annotations_parse_format_analyze_and_execute() {
    use themoretheless_tokenizer_rush::{analyze_calls, format_source};
    for source in [
        "fn f() -> null { return }; f()",
        "fn f(v: null) -> null { return v }; f(null)",
        "let xs: list[null] = [null]; xs[0]",
        "let value: null = null; value",
    ] {
        assert!(analyze_calls(source).is_valid(), "{source}");
        let formatted = format_source(source).unwrap();
        assert!(analyze_calls(&formatted).is_valid());
        assert_eq!(
            Program::compile(&formatted)
                .unwrap()
                .run(1000, &Default::default(), &[])
                .unwrap(),
            Value::Null
        );
    }
    assert!(!analyze_calls("fn f() -> null { return 1 }").is_valid());
    assert!(Program::compile("let null = 1").is_err());
}

#[test]
fn annotated_function_results_flow_into_calls_aliases_and_editor_types() {
    use themoretheless_tokenizer_rush::{analyze_calls, analyze_editor};
    assert!(!analyze_calls("fn point() -> vec2 { return vec2(1,2) }; point().z").is_valid());
    assert!(
        !analyze_calls("fn point() -> vec2 { return vec2(1,2) }; let alias = point; alias().z")
            .is_valid()
    );
    let source = "fn point() -> vec3 { return vec3(1,2,3) }; let v = point(); v.";
    let (_, _, bindings) = analyze_editor(source);
    assert_eq!(
        bindings.iter().find(|b| b.name == "v").unwrap().members,
        ["x", "y", "z"]
    );
    assert!(analyze_calls("fn point() -> vec2 { return vec2(1,2) }; if true { let point = () => {z:0}; point().z }").is_valid());
}

#[test]
fn user_function_arguments_are_checked_from_annotations_in_calls_and_pipes() {
    use themoretheless_tokenizer_rush::analyze_calls;
    for source in [
        "fn f(v: vec3) { return v }; f(vec2(1,2))",
        "fn f(v: vec3) { return v }; vec2(1,2) | f",
        "fn f((v,n): tuple[vec3,number]) { return v }; f((vec2(1,2),0))",
        "fn f(v: number) { return f(true) }",
    ] {
        assert!(
            analyze_calls(source)
                .diagnostics
                .iter()
                .any(|d| d.code == "argument-type"),
            "{source}"
        );
    }
    for source in [
        "fn f(v) { return v }; f(true)",
        "fn f(v: vec3) { return v }; vec3(1,2,3) | f",
        "fn f(v: number) { return v }; if true { let f = x => x; f(true) }",
    ] {
        assert!(analyze_calls(source).is_valid(), "{source}");
    }
}

#[test]
fn immutable_user_function_aliases_keep_parameter_contracts() {
    use themoretheless_tokenizer_rush::analyze_calls;
    for source in [
        "fn f(v: vec3) { return v }; let g = f; g(vec2(1,2))",
        "fn f(v: number) { return v }; let g = f; let h = g; true | h",
        "fn f(v: number) { return v }; if true { let f = f; f(true) }",
    ] {
        assert!(
            analyze_calls(source)
                .diagnostics
                .iter()
                .any(|d| d.code == "argument-type"),
            "{source}"
        );
    }
    assert!(
        analyze_calls("fn f(v: number) { return v }; mut g = f; g = x => x; g(true)").is_valid()
    );
    assert!(
        analyze_calls(
            "fn f(v: number) { return v }; let g = f; if true { let g = x => x; g(true) }"
        )
        .is_valid()
    );
}

#[test]
fn destructured_user_functions_keep_parameter_and_result_annotations() {
    use themoretheless_tokenizer_rush::analyze_calls;
    for source in [
        "fn f(x: number) -> vec2 { return vec2(x,0) }; let (g,_) = (f,0); g(true)",
        "fn f(x: number) -> vec2 { return vec2(x,0) }; let {run:g} = {run:f}; g(1).z",
        "fn f(x: number) -> vec2 { return vec2(x,0) }; let (f,g) = (x => x,f); g(true)",
    ] {
        assert!(!analyze_calls(source).is_valid(), "{source}");
    }
    assert!(
        analyze_calls(
            "fn f(x: number) -> vec2 { return vec2(x,0) }; let {run:g} = {run:f}; g(1).x"
        )
        .is_valid()
    );
}

#[test]
fn immutable_record_fields_are_available_to_editor_completion() {
    use themoretheless_tokenizer_rush::analyze_editor;
    let (_, _, bindings) =
        analyze_editor("let config = {width:10,height:20}; let alias = config; alias.");
    for name in ["config", "alias"] {
        assert_eq!(
            bindings.iter().find(|b| b.name == name).unwrap().members,
            ["height", "width"]
        );
    }
    let (_, _, bindings) = analyze_editor("mut config = {width:10}; config.");
    assert!(bindings[0].members.is_empty());
    let (_, _, bindings) =
        analyze_editor("let config = {width:10}; if true { let config = 1; config }");
    assert!(
        bindings
            .iter()
            .rfind(|b| b.name == "config")
            .unwrap()
            .members
            .is_empty()
    );
}

#[test]
fn known_record_field_typos_report_precise_spans() {
    use themoretheless_tokenizer_rush::analyze_calls;
    for source in [
        "let config = {width:10}; config.widht",
        "let config = {width:10}; let alias = config; alias.widht",
        "({width:10}).widht",
    ] {
        let parsed = analyze_calls(source);
        let diagnostic = parsed
            .diagnostics
            .iter()
            .find(|d| d.code == "unknown-record-field")
            .unwrap();
        assert_eq!(&source[diagnostic.span.start..diagnostic.span.end], "widht");
        assert!(
            Program::compile(source)
                .unwrap()
                .run(1000, &Default::default(), &[])
                .is_err()
        );
    }
    for source in [
        "let config = {width:10}; config.width",
        "mut config = {width:10}; config = {height:20}; config.height",
        "let config = {width:10}; if true { let config = {height:20}; config.height }",
        "fn f(config) { return config.height }",
    ] {
        assert!(analyze_calls(source).is_valid(), "{source}");
    }
}

#[test]
fn known_records_validate_required_destructuring_fields() {
    use themoretheless_tokenizer_rush::analyze_calls;
    for source in [
        "let {height:h} = {width:10}",
        "let config = {width:10}; let {height:h} = config",
    ] {
        let parsed = analyze_calls(source);
        let diagnostic = parsed
            .diagnostics
            .iter()
            .find(|d| d.code == "unknown-record-field")
            .unwrap();
        assert_eq!(
            &source[diagnostic.span.start..diagnostic.span.end],
            "height"
        );
        assert!(
            Program::compile(source)
                .unwrap()
                .run(1000, &Default::default(), &[])
                .is_err()
        );
    }
    for source in [
        "let {width:w} = {width:10,height:20}",
        "fn f(config) { let {height:h} = config; return h }",
    ] {
        assert!(analyze_calls(source).is_valid(), "{source}");
    }
}

#[test]
fn tuple_destructuring_checks_known_shapes_and_nested_lengths() {
    use themoretheless_tokenizer_rush::analyze_calls;
    for source in [
        "let (a,b) = (1,2,3)",
        "let ((a,b),c) = ((1,2,3),0)",
        "let (a,b) = [1,2]",
        "let (a,b) = 1",
    ] {
        assert!(
            analyze_calls(source)
                .diagnostics
                .iter()
                .any(|d| d.code == "tuple-pattern"),
            "{source}"
        );
        assert!(
            Program::compile(source)
                .unwrap()
                .run(1000, &Default::default(), &[])
                .is_err()
        );
    }
    assert!(analyze_calls("let ((a,b),c) = ((1,2),0)").is_valid());
    assert!(analyze_calls("fn f(value) { let (a,b) = value; return a }").is_valid());
    let functions = [function(
        "pair",
        vec![],
        T::Tuple(vec![T::Number, T::Number]),
    )];
    assert!(!analyze_host_calls("let (a,b,c) = pair()", &functions).is_valid());
}

#[test]
fn tuple_parameter_patterns_must_fit_their_annotations() {
    use themoretheless_tokenizer_rush::analyze_calls;
    for source in [
        "fn f((a,b): number) { return a }",
        "fn f((a,b): tuple[number]) { return a }",
        "fn f(((a,b),c): tuple[number,number]) { return a }",
    ] {
        assert!(
            analyze_calls(source)
                .diagnostics
                .iter()
                .any(|d| d.code == "tuple-pattern"),
            "{source}"
        );
    }
    for source in [
        "fn f((a,b): tuple[number,bool]) { return a }",
        "fn f(((a,b),c): tuple[tuple[number,bool],vec3]) { return c.x }",
        "fn f((a,b)) { return a }",
    ] {
        assert!(analyze_calls(source).is_valid(), "{source}");
    }
}

#[test]
fn known_non_boolean_conditions_are_diagnosed() {
    use themoretheless_tokenizer_rush::analyze_calls;
    for source in [
        "if 1 { 2 }",
        "while vec2(1,2) { break }",
        "let x = if 1 { 2 } else { 3 }",
        "match 1 { x if (42) => x, _ => 0 }",
        "let yes = 1; if yes { 2 }",
    ] {
        assert!(
            analyze_calls(source)
                .diagnostics
                .iter()
                .any(|d| d.code == "condition-type"),
            "{source}"
        );
    }
    for source in [
        "if true { 1 }",
        "while false { break }",
        "fn f(flag) { if flag { return 1 }; return 0 }",
        "match 1 { x if (true) => x, _ => 0 }",
    ] {
        assert!(analyze_calls(source).is_valid(), "{source}");
    }
}

#[test]
fn logical_operators_require_known_boolean_operands() {
    use themoretheless_tokenizer_rush::analyze_calls;
    for source in [
        "not 1",
        "!vec2(1,2)",
        "true and 2",
        "1 or false",
        "true && 2",
        "1 || false",
    ] {
        assert!(
            analyze_calls(source)
                .diagnostics
                .iter()
                .any(|d| d.code == "condition-type"),
            "{source}"
        );
    }
    for source in [
        "not false",
        "!true",
        "true and false",
        "false || true",
        "fn f(flag) { return not flag }",
    ] {
        assert!(analyze_calls(source).is_valid(), "{source}");
    }
}

#[test]
fn annotated_functions_check_implicit_null_paths() {
    use themoretheless_tokenizer_rush::analyze_calls;
    for source in [
        "fn f() -> number { 1 }",
        "fn f(x) -> number { if x { return 1 } }",
        "fn f() -> number { fn g() { return 1 } }",
        "fn f() -> number { if false { return 1 } }",
        "fn f() -> number { while false { return 1 } }",
    ] {
        let analysis = analyze_calls(source);
        assert!(
            analysis
                .diagnostics
                .iter()
                .any(|d| d.code == "missing-return"),
            "{source}: {:?}",
            analysis.diagnostics
        );
    }
    for source in [
        "fn f() -> null { 1 }",
        "fn f() { 1 }",
        "fn f(x) -> number { if x { return 1 } else { return 2 } }",
        "fn f(x) -> number { if x { return 1 }; return 2 }",
        "fn f() -> number { if true { return 1 } }",
        "fn f() -> number { while true { return 1 } }",
    ] {
        assert!(analyze_calls(source).is_valid(), "{source}");
    }
}

#[test]
fn return_paths_respect_loop_exits_and_zero_iterations() {
    use themoretheless_tokenizer_rush::analyze_calls;
    for source in [
        "fn f(flag) -> number { while flag { return 1 } }",
        "fn f() -> number { for x in [] { return 1 } }",
        "fn f() -> number { while true { break } }",
        "fn f(flag) -> number { while true { if flag { break } else { return 1 } } }",
        "fn f() -> number { while true { while true { break }; break } }",
    ] {
        let analysis = analyze_calls(source);
        assert!(
            analysis
                .diagnostics
                .iter()
                .any(|d| d.code == "missing-return"),
            "{source}: {:?}",
            analysis.diagnostics
        );
    }
    for source in [
        "fn f() -> number { while true { continue; break } }",
        "fn f() -> number { while true { return 1; break } }",
        "fn f() -> number { while true { if false { break } } }",
        "fn f() -> number { while true { while true { break } } }",
        "fn f() -> number { while true { for x in [] { break } } }",
        "fn f(flag) -> number { while flag { continue }; return 1 }",
        "fn f() -> number { while true { break }; return 1 }",
        "fn f() -> number { for x in [] { return 1 }; return 2 }",
    ] {
        let analysis = analyze_calls(source);
        assert!(analysis.is_valid(), "{source}: {:?}", analysis.diagnostics);
    }
}

#[test]
fn indexing_checks_known_contracts_and_preserves_element_types() {
    use themoretheless_tokenizer_rush::analyze_calls;
    for (source, code) in [
        ("vec2(1,2)[2]", "index-bounds"),
        ("[1,2][-1]", "index-bounds"),
        ("(1,2)[0.5]", "index-bounds"),
        ("[][0]", "index-bounds"),
        ("vec3(1,2,3)[true]", "index-type"),
        ("{x:1}[0]", "index-type"),
        ("true[0]", "index-object"),
        ("fn f(v: list[number]) { return v[false] }", "index-type"),
        ("let x: bool = vec2(1,2)[0]", "annotation-type"),
        ("let x: number = (1,true)[1]", "annotation-type"),
        ("fn f(v: list[vec2]) { return v[0].z }", "vector-component"),
    ] {
        let analysis = analyze_calls(source);
        assert!(
            analysis.diagnostics.iter().any(|d| d.code == code),
            "{source}: {:?}",
            analysis.diagnostics
        );
    }
    for source in [
        "vec2(1,2)[1]",
        "[1,2][0]",
        "{x:1}[\"x\"]",
        "fn f(v,i) { return v[i] }",
        "fn f(v: list[vec3], i: number) -> number { return v[i].z }",
        "let x: bool = (1,true)[1]",
    ] {
        let analysis = analyze_calls(source);
        assert!(analysis.is_valid(), "{source}: {:?}", analysis.diagnostics);
    }
}

#[test]
fn higher_order_calls_check_callback_arity_without_confusing_shadowed_names() {
    use themoretheless_tokenizer_rush::{analyze_calls, evaluate};
    for source in [
        "map([], (a,b) => a)",
        "[] | filter(() => true)",
        "fold([], 0, x => x)",
        "group_by([], (a,b) => 'a')",
        "[] | fold_by(x => 'a', 0, x => x)",
        "let reducer = x => x; fold([], 0, reducer)",
        "map([], vec2)",
        "range_iter(0,0) | map((x,y) => x)",
    ] {
        let analysis = analyze_calls(source);
        assert!(
            analysis
                .diagnostics
                .iter()
                .any(|d| d.code == "callback-argument-count"),
            "{source}: {:?}",
            analysis.diagnostics
        );
        assert!(
            evaluate(source, 10000)
                .unwrap_err()
                .message
                .contains("Callback argument count"),
            "{source}"
        );
    }
    for source in [
        "map([], x => x)",
        "[] | fold_by(x => 'a', 0, (a,x) => a+x)",
        "fn map(items, callback) { return callback(1,2) }; map([], (a,b) => a+b)",
        "fn f(callback) { return map([], callback) }",
    ] {
        assert!(analyze_calls(source).is_valid(), "{source}");
    }
}

#[test]
fn builtin_aliases_preserve_callback_contracts_through_destructuring() {
    use themoretheless_tokenizer_rush::analyze_calls;
    for source in [
        "let transform = map; let alias = transform; alias([], (a,b) => a)",
        "let transform = fold; [] | transform(0,x => x)",
        "let (op,_) = (group_by,0); op([], (a,b) => 'a')",
        "let {op:f} = {op:fold_by}; f([], x => 'a', 0, x => x)",
        "let map = map; map([], (a,b) => a)",
        "let (map,op) = (x => x,map); op([], (a,b) => a)",
        "let op = if true {map} else {map}; op([], (a,b) => a)",
    ] {
        let analysis = analyze_calls(source);
        assert!(
            analysis
                .diagnostics
                .iter()
                .any(|d| d.code == "callback-argument-count"),
            "{source}: {:?}",
            analysis.diagnostics
        );
    }
    for source in [
        "let op = map; if true { let op = (items,f) => f(1,2); op([], (a,b) => a+b) }",
        "mut op = map; op = (items,f) => f(1,2); op([], (a,b) => a+b)",
        "fn f(map) { return map([], (a,b) => a+b) }",
        "let transform = map; transform([], x => x)",
    ] {
        let analysis = analyze_calls(source);
        assert!(analysis.is_valid(), "{source}: {:?}", analysis.diagnostics);
    }
}

#[test]
fn immutable_nested_records_keep_member_shapes() {
    use themoretheless_tokenizer_rush::analyze_calls;
    for (source, code) in [
        (
            "let settings = {camera:{position:vec3(1,2,3)}}; settings.camera.postion",
            "unknown-record-field",
        ),
        (
            "let settings = {camera:{position:vec3(1,2,3)}}; settings.camera.position.w",
            "vector-component",
        ),
        (
            "let settings = {camera:{position:vec3(1,2,3)}}; let camera = settings.camera; camera.position.w",
            "vector-component",
        ),
        (
            "let x: bool = {inner:{value:1}}.inner.value",
            "annotation-type",
        ),
        (
            "let a = {inner:{x:1}}; let {y:y} = a.inner",
            "unknown-record-field",
        ),
    ] {
        let analysis = analyze_calls(source);
        assert!(
            analysis.diagnostics.iter().any(|d| d.code == code),
            "{source}: {:?}",
            analysis.diagnostics
        );
    }
    for source in [
        "let settings = {camera:{position:vec3(1,2,3)}}; settings.camera.position.z",
        "let a = {inner:{x:1}}; if true { let a = {inner:{y:2}}; a.inner.y }",
        "mut a = {inner:{x:1}}; a = {inner:{y:2}}; a.inner.y",
        "fn f(a) { return a.inner.anything }",
        "let a = {inner:{x:1},inner:{y:2}}; a.inner.y",
    ] {
        let analysis = analyze_calls(source);
        assert!(analysis.is_valid(), "{source}: {:?}", analysis.diagnostics);
    }
}

#[test]
fn known_data_values_cannot_be_called_or_passed_as_callbacks() {
    use themoretheless_tokenizer_rush::analyze_calls;
    for (source, code) in [
        ("1()", "not-callable"),
        ("let value = 1; value()", "not-callable"),
        ("({x:1})()", "not-callable"),
        ("let config = {f:1}; config.f()", "not-callable"),
        ("fn f() -> number { return 1 }; f()()", "not-callable"),
        ("let f = 2; 1 | f", "not-callable"),
        ("map([], 1)", "callback-type"),
        ("[] | fold(0, false)", "callback-type"),
        (
            "let op = fold_by; op([], x => 'all', 0, {f:1})",
            "callback-type",
        ),
    ] {
        let analysis = analyze_calls(source);
        assert!(
            analysis.diagnostics.iter().any(|d| d.code == code),
            "{source}: {:?}",
            analysis.diagnostics
        );
    }
    for source in [
        "fn f(callback) { return callback(1) }",
        "fn f(callback) { return map([], callback) }",
        "let record = {f:x => x}; record.f(1)",
        "let op = if true {x => x} else {x => x+1}; op(1)",
        "mut op = 1; op = x => x; op(1)",
    ] {
        let analysis = analyze_calls(source);
        assert!(analysis.is_valid(), "{source}: {:?}", analysis.diagnostics);
    }
}

#[test]
fn member_access_rejects_known_unsupported_types_but_not_unknown_objects() {
    for source in [
        "null.missing",
        "true.x",
        "[1].x",
        "(1,2).x",
        "\"text\".length",
        "let number = 1; number.x",
        "let config = {item:false}; config.item.x",
        "fn bad(value: number) { return value.x }",
    ] {
        let parsed = analyze_host_calls(source, &[]);
        assert!(
            parsed.diagnostics.iter().any(|d| d.code == "member-object"),
            "{source}: {:?}",
            parsed.diagnostics
        );
    }
    for source in [
        "fn read(value) { return value.x }",
        "vec2(1,2).x",
        "let record = {x:1}; record.x",
    ] {
        let parsed = analyze_host_calls(source, &[]);
        assert!(parsed.is_valid(), "{source}: {:?}", parsed.diagnostics);
    }
}

#[test]
fn mesh_fields_have_checked_names_and_element_types() {
    for source in [
        "fn vertices(m: mesh) -> list[vec3] { return m.vertices }",
        "fn triangles(m: mesh) -> list[list[number]] { return m.triangles }",
    ] {
        let parsed = analyze_host_calls(source, &[]);
        assert!(parsed.is_valid(), "{source}: {:?}", parsed.diagnostics);
    }
    let parsed = analyze_host_calls("fn bad(m: mesh) { return m.vertces }", &[]);
    assert!(parsed.diagnostics.iter().any(|d| d.code == "mesh-field"));
    let parsed = analyze_host_calls("fn bad(m: mesh) -> number { return m.vertices }", &[]);
    assert!(!parsed.is_valid());
}

#[test]
fn immutable_collection_aliases_preserve_shapes_for_indexing_and_destructuring() {
    for (source, code) in [
        (
            "let values = [1,2]; let alias = values; alias[2]",
            "index-bounds",
        ),
        (
            "let values = (1, false); let alias = values; alias[2]",
            "index-bounds",
        ),
        (
            "let values = ({position:vec3(1,2,3)}, 0); let (record, _) = values; record.position.w",
            "vector-component",
        ),
        (
            "let values = [{position:vec3(1,2,3)}]; values[0].missing",
            "unknown-record-field",
        ),
    ] {
        let parsed = analyze_host_calls(source, &[]);
        assert!(
            parsed.diagnostics.iter().any(|d| d.code == code),
            "{source}: {:?}",
            parsed.diagnostics
        );
    }
    assert!(analyze_host_calls("let mut values = [1]; values = [1,2]; values[1]", &[]).is_valid());
}

#[test]
fn destructuring_typed_tuple_parameters_preserves_element_types() {
    for source in [
        "fn read(pair: tuple[vec3,number]) -> number { let (point, _) = pair; return point.z }",
        "fn read(pair: tuple[tuple[vec3,number],bool]) -> number { let ((point, _), _) = pair; return point.z }",
    ] {
        assert!(analyze_host_calls(source, &[]).is_valid(), "{source}");
    }
    for (source, code) in [
        (
            "fn read(pair: tuple[vec2,number]) { let (point, _) = pair; return point.z }",
            "vector-component",
        ),
        (
            "fn read(pair: tuple[number,bool]) { let (point, _) = pair; return point.x }",
            "member-object",
        ),
    ] {
        let parsed = analyze_host_calls(source, &[]);
        assert!(
            parsed.diagnostics.iter().any(|d| d.code == code),
            "{source}: {:?}",
            parsed.diagnostics
        );
    }
}

#[test]
fn nested_record_patterns_validate_known_shapes_recursively() {
    for (source, code) in [
        (
            "let {outer:{missing:value}} = {outer:{known:1}}",
            "unknown-record-field",
        ),
        (
            "let ({outer:{missing:value}}, _) = ({outer:{known:1}}, 0)",
            "unknown-record-field",
        ),
        ("let {outer:{field:value}} = {outer:1}", "record-pattern"),
        ("let {outer:(x,y)} = {outer:(1,2,3)}", "tuple-pattern"),
    ] {
        let parsed = analyze_host_calls(source, &[]);
        assert!(
            parsed.diagnostics.iter().any(|d| d.code == code),
            "{source}: {:?}",
            parsed.diagnostics
        );
    }
    for source in [
        "let {outer:{known:value}} = {outer:{known:1}}; value",
        "fn read(input) { let {outer:{field:value}} = input; return value }",
    ] {
        assert!(analyze_host_calls(source, &[]).is_valid(), "{source}");
    }
}

#[test]
fn for_bindings_infer_known_uniform_list_elements() {
    for (source, code) in [
        (
            "for point in [vec2(1,2),vec2(3,4)] { point.z }",
            "vector-component",
        ),
        (
            "fn check(points: list[vec2]) { for point in points { point.z } }",
            "vector-component",
        ),
        (
            "for item in [{position:vec3(1,2,3)}] { item.missing }",
            "unknown-record-field",
        ),
        (
            "let points = [vec2(1,2)]; for points in points { points.z }",
            "vector-component",
        ),
        (
            "for row in [[vec2(1,2)]] { for point in row { point.z } }",
            "vector-component",
        ),
        (
            "for pair in [({position:vec2(1,2)},0)] { let (item,_) = pair; item.position.z }",
            "vector-component",
        ),
    ] {
        let parsed = analyze_host_calls(source, &[]);
        assert!(
            parsed.diagnostics.iter().any(|d| d.code == code),
            "{source}: {:?}",
            parsed.diagnostics
        );
    }
    for source in [
        "for value in [] { value.x }",
        "for value in [1,vec2(1,2)] { value.x }",
        "fn check(values) { for value in values { value.x } }",
        "let point = {outer:1}; for point in [vec3(1,2,3)] { point.z }; point.outer",
    ] {
        assert!(analyze_host_calls(source, &[]).is_valid(), "{source}");
    }
}
