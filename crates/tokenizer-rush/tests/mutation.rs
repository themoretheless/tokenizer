use themoretheless_tokenizer_rush::{Value, evaluate};

#[test]
fn list_element_mutation_and_compound_assignment() {
    let script = "mut a = [1, 2, 3]; a[0] = 10; a[1] += 20; a";
    let res = evaluate(script, 1000).unwrap();
    assert_eq!(
        res,
        Value::List(vec![
            Value::Number(10.0),
            Value::Number(22.0),
            Value::Number(3.0)
        ])
    );
}

#[test]
fn list_index_out_of_bounds_errors() {
    let script = "mut a = [1, 2]; a[5] = 10";
    let err = evaluate(script, 1000).unwrap_err();
    assert_eq!(err.message, "Index out of bounds");
}

#[test]
fn record_field_mutation_via_member_and_index() {
    let script = "mut r = { x: 1, y: 2 }; r.x = 100; r['y'] += 50; (r.x, r.y)";
    let res = evaluate(script, 1000).unwrap();
    assert_eq!(
        res,
        Value::Tuple(vec![Value::Number(100.0), Value::Number(52.0)])
    );
}

#[test]
fn nested_collection_mutation() {
    let script = "mut grid = [[1, 2], [3, 4]]; grid[0][1] = 99; grid[1][0] += 10; grid";
    let res = evaluate(script, 1000).unwrap();
    assert_eq!(
        res,
        Value::List(vec![
            Value::List(vec![Value::Number(1.0), Value::Number(99.0)]),
            Value::List(vec![Value::Number(13.0), Value::Number(4.0)])
        ])
    );
}

#[test]
fn struct_field_mutation() {
    let script = "struct Point { x: number, y: number }\nmut p = Point({ x: 1, y: 2 }); p.x = 10; p.y += 5; (p.x, p.y)";
    let res = evaluate(script, 1000).unwrap();
    assert_eq!(
        res,
        Value::Tuple(vec![Value::Number(10.0), Value::Number(7.0)])
    );
}

#[test]
fn vector_component_mutation() {
    let script = "mut v = vec2(1, 2); v.x = 10; v.y += 20; v";
    let res = evaluate(script, 1000).unwrap();
    assert_eq!(res, Value::Vector(vec![10.0, 22.0]));
}

#[test]
fn immutable_binding_mutation_is_rejected_at_runtime() {
    let script = "let a = [1, 2]; a[0] = 5";
    let err = evaluate(script, 1000).unwrap_err();
    assert_eq!(err.message, "Assignment requires a mutable variable");
}

#[test]
fn typed_list_violating_assignment_is_rejected() {
    let script = "mut a: list[number] = [1, 2]; a[0] = 'not_a_number'";
    let err = evaluate(script, 1000).unwrap_err();
    assert_eq!(err.message, "Assignment violates variable type");
}
