use themoretheless_tokenizer_rush::{CancellationToken, Program, Value};
const CURVES: &str = include_str!("../examples/scripts/curves.r");

fn run(source: &str) -> Value<'_> {
    // Values tested below contain only owned numeric/mesh data.
    let library = Program::compile(CURVES).unwrap();
    Program::compile(source)
        .unwrap()
        .run_with_modules(
            100_000,
            &CancellationToken::default(),
            &[],
            &[],
            &[("curves", &library)],
        )
        .unwrap()
}

#[test]
fn bezier_endpoints_midpoint_and_tangents() {
    assert_eq!(
        run(
            "import curves\n(curves.cubic(0,2,4,6,0), curves.cubic(0,2,4,6,1), curves.cubic(0,2,4,6,0.5), curves.cubic_tangent(0,2,4,6,0.5))"
        ),
        Value::Tuple(vec![
            Value::Number(0.),
            Value::Number(6.),
            Value::Number(3.),
            Value::Number(6.)
        ])
    );
    assert_eq!(
        run("import curves\ncurves.quadratic(vec2(0,0),vec2(1,2),vec2(2,0),0.5)"),
        Value::Vector(vec![1., 1.])
    );
    assert_eq!(
        run("import curves\ncurves.quadratic_tangent(vec2(0,0),vec2(1,2),vec2(2,0),0.5)"),
        Value::Vector(vec![2., 0.])
    );
}

#[test]
fn ribbon_example_has_expected_topology_and_bounds() {
    let Value::Mesh(mesh) = run(include_str!("../examples/scripts/bezier-ribbon.r")) else {
        panic!()
    };
    assert_eq!(mesh.vertices().len(), 82);
    assert_eq!(mesh.triangles().len(), 80);
    assert!(mesh.vertices().iter().flatten().all(|v| v.is_finite()));
    for p in mesh.vertices() {
        assert!((-3.0..=3.0).contains(&p[0]));
    }
}
