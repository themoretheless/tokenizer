use themoretheless_tokenizer_rush::{
    CancellationToken, CoroutineState, ExecutionLimits, Program, Value, evaluate, evaluate_bytecode,
};

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
    assert_eq!(evaluate_bytecode(script, 1000).unwrap(), res);
}

#[test]
fn list_index_out_of_bounds_errors() {
    let script = "mut a = [1, 2]; a[5] = 10";
    let err = evaluate(script, 1000).unwrap_err();
    assert_eq!(err.message, "Index out of bounds");
    assert!(evaluate_bytecode(script, 1000).is_err());
}

#[test]
fn record_field_mutation_via_member_and_index() {
    let script = "mut r = { x: 1, y: 2 }; r.x = 100; r['y'] += 50; (r.x, r.y)";
    let res = evaluate(script, 1000).unwrap();
    assert_eq!(
        res,
        Value::Tuple(vec![Value::Number(100.0), Value::Number(52.0)])
    );
    assert_eq!(evaluate_bytecode(script, 1000).unwrap(), res);
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
    assert_eq!(evaluate_bytecode(script, 1000).unwrap(), res);
}

#[test]
fn struct_field_mutation() {
    let script = "struct Point { x: number, y: number }\nmut p = Point({ x: 1, y: 2 }); p.x = 10; p.y += 5; (p.x, p.y)";
    let res = evaluate(script, 1000).unwrap();
    assert_eq!(
        res,
        Value::Tuple(vec![Value::Number(10.0), Value::Number(7.0)])
    );
    assert_eq!(evaluate_bytecode(script, 1000).unwrap(), res);
}

#[test]
fn struct_field_type_violation_is_rejected() {
    let direct =
        "struct Point { x: number, y: number }\nmut p = Point({ x: 1, y: 2 }); p.x = 'bad'";
    let err = evaluate(direct, 1000).unwrap_err();
    assert_eq!(err.message, "Assignment violates variable type");

    let nested_in_list = "struct Point { x: number, y: number }\nmut pts = [Point({ x: 1, y: 2 })]; pts[0].x = 'bad'";
    let err = evaluate(nested_in_list, 1000).unwrap_err();
    assert_eq!(err.message, "Assignment violates variable type");

    let nested_list_in_struct =
        "struct Bag { items: list[number] }\nmut b = Bag({ items: [1, 2] }); b.items[0] = 'bad'";
    let err = evaluate(nested_list_in_struct, 1000).unwrap_err();
    assert_eq!(err.message, "Assignment violates variable type");
}

#[test]
fn vector_component_mutation_via_member_and_index() {
    let script = "mut v = vec3(1, 2, 3); v.x = 10; v.y += 20; v[2] *= 4; v";
    let res = evaluate(script, 1000).unwrap();
    assert_eq!(res, Value::vec3(10.0, 22.0, 12.0));
    assert_eq!(evaluate_bytecode(script, 1000).unwrap(), res);
}

#[test]
fn compound_path_assignment_evaluates_index_once() {
    let script = "\
mut calls = 0
fn idx() {
    calls += 1
    return 0
}
mut a = [10, 20]
a[idx()] += 5
(a[0], calls)";
    let res = evaluate(script, 1000).unwrap();
    assert_eq!(
        res,
        Value::Tuple(vec![Value::Number(15.0), Value::Number(1.0)])
    );
    assert_eq!(evaluate_bytecode(script, 1000).unwrap(), res);
}

#[test]
fn closure_upvalue_and_coroutine_field_index_mutation() {
    let closure_script = "\
mut state = { count: 0, items: [1, 2] }
fn bump(delta) {
    state.count += delta
    state.items[0] += delta
    return state.count + state.items[0]
}
(bump(3), bump(4), state.count, state.items[0])";
    let res = evaluate(closure_script, 1000).unwrap();
    assert_eq!(
        res,
        Value::Tuple(vec![
            Value::Number(7.0),
            Value::Number(15.0),
            Value::Number(7.0),
            Value::Number(8.0),
        ])
    );
    assert_eq!(evaluate_bytecode(closure_script, 1000).unwrap(), res);

    let coroutine_script = "\
fn worker() {
    mut state = { step: 0, pos: vec2(1, 2) }
    state.step += 1
    state.pos.x += 10
    yield (state.step, state.pos.x)
    state.step += 1
    state.pos.y += 20
    return (state.step, state.pos.y)
}
0";
    let program = Program::compile(coroutine_script).unwrap();
    let cancel = CancellationToken::default();
    let limits = ExecutionLimits::new(1000);
    let mut instance = program
        .instantiate(limits, &cancel, &[], &[], &[], &[])
        .unwrap();
    let co = instance.spawn_coroutine("worker", &[], limits).unwrap();
    assert_eq!(
        instance.resume_coroutine(co, limits).unwrap(),
        CoroutineState::Yielded(Value::Tuple(vec![Value::Number(1.0), Value::Number(11.0)]))
    );
    assert_eq!(
        instance.resume_coroutine(co, limits).unwrap(),
        CoroutineState::Complete(Value::Tuple(vec![Value::Number(2.0), Value::Number(22.0)]))
    );
}

#[test]
fn strict_contract_inference_propagates_through_member_mutation() {
    let script = "\
struct Point { x: number, y: number }
fn translate_x(p: Point, dx) -> Point {
    mut q = p
    q.x += dx
    return q
}
let p = translate_x(Point({ x: 3, y: 4 }), 7)
p.x";
    let program = Program::compile_strict(script).unwrap();
    let res = program
        .run(1000, &CancellationToken::default(), &[])
        .unwrap();
    assert_eq!(res, Value::Number(10.0));
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
