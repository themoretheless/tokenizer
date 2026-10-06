//! cargo run -p themoretheless-tokenizer-rush --example host_scene > triangle.obj
//! The application owns scene objects; scripts receive non-owning references.
use std::{cell::RefCell, rc::Rc};
use themoretheless_tokenizer_rush::{
    CancellationToken, HostFunction, HostObject, Program, Value, ValueType as T,
    analyze_editor_with_host,
};
type Point = RefCell<[f64; 3]>;
thread_local! {
    // A minimal single-threaded embedding. A real application owns its scene lifecycle.
    static SCENE: RefCell<Vec<Rc<Point>>> = const { RefCell::new(Vec::new()) };
}
fn point(value: &Value<'_>) -> Result<Rc<Point>, String> {
    let Value::HostObject(object) = value else {
        return Err("Expected a scene point".into());
    };
    object
        .upgrade::<Point>()
        .ok_or_else(|| "Scene point is no longer available".into())
}
fn create<'s>(args: &[Value<'s>], _: &CancellationToken) -> Result<Value<'s>, String> {
    let [Value::Vector(coordinates)] = args else {
        unreachable!()
    };
    let owner = Rc::new(RefCell::new([
        coordinates[0],
        coordinates[1],
        coordinates[2],
    ]));
    let reference = HostObject::new("ScenePoint", &owner);
    SCENE.with(|scene| scene.borrow_mut().push(owner));
    Ok(Value::HostObject(reference))
}
fn position<'s>(args: &[Value<'s>], _: &CancellationToken) -> Result<Value<'s>, String> {
    Ok(Value::Vector(point(&args[0])?.borrow().to_vec()))
}
fn translate<'s>(args: &[Value<'s>], _: &CancellationToken) -> Result<Value<'s>, String> {
    let owner = point(&args[0])?;
    let Value::Vector(offset) = &args[1] else {
        unreachable!()
    };
    let previous = *owner.borrow();
    let next = std::array::from_fn(|i| previous[i] + offset[i]);
    if !next.iter().all(|x: &f64| x.is_finite()) {
        return Err("Point translation overflow".into());
    }
    *owner.borrow_mut() = next;
    Ok(args[0].clone())
}
fn remove<'s>(args: &[Value<'s>], _: &CancellationToken) -> Result<Value<'s>, String> {
    let owner = point(&args[0])?;
    SCENE.with(|scene| scene.borrow_mut().retain(|p| !Rc::ptr_eq(p, &owner)));
    Ok(Value::Null)
}
fn functions() -> Vec<Rc<HostFunction>> {
    let point = T::HostObject("ScenePoint");
    vec![
        Rc::new(HostFunction {
            name: "scene_point",
            parameters: vec![T::Vector(3)],
            result: point.clone(),
            callback: create,
        }),
        Rc::new(HostFunction {
            name: "position",
            parameters: vec![point.clone()],
            result: T::Vector(3),
            callback: position,
        }),
        Rc::new(HostFunction {
            name: "translate_point",
            parameters: vec![point.clone(), T::Vector(3)],
            result: point.clone(),
            callback: translate,
        }),
        Rc::new(HostFunction {
            name: "remove_point",
            parameters: vec![point],
            result: T::Null,
            callback: remove,
        }),
    ]
}
fn main() {
    let functions = functions();
    let source = "let points = [vec3(0,0,0), vec3(1,0,0), vec3(0,1,0)] | map(scene_point)\nlet moved = points | map(p => translate_point(p, vec3(0,0,2)))\nassert(position(points[0]).z == 2)\nmesh(moved | map(position), [[0,1,2]])";
    let checked = analyze_editor_with_host(source, &[], &functions);
    assert!(
        checked.parsed.is_valid(),
        "{:?}",
        checked.parsed.diagnostics
    );
    let result = Program::compile(source)
        .unwrap()
        .run_with_host(10_000, &CancellationToken::default(), &[], &functions)
        .unwrap();
    let Value::Mesh(mesh) = result else {
        panic!("Expected mesh")
    };
    assert_eq!(mesh.vertices(), &[[0., 0., 2.], [1., 0., 2.], [0., 1., 2.]]);
    assert_eq!(mesh.triangles(), &[[0, 1, 2]]);
    print!("{}", mesh.to_obj());
    // Application deletion also expires references captured by script closures.
    let deleted =
        "let p = scene_point(vec3(0,0,0)); let read = () => position(p); remove_point(p); read()";
    let error = Program::compile(deleted)
        .unwrap()
        .run_with_host(1000, &Default::default(), &[], &functions)
        .unwrap_err();
    assert_eq!(&deleted[error.span.start..error.span.end], "position(p)");
    SCENE.with(|scene| scene.borrow_mut().clear());
}

#[test]
fn scene_example_executes_and_checks_expired_references() {
    main();
}

#[test]
fn failed_translation_preserves_application_object() {
    let functions = functions();
    let script = "let p = scene_point(vec3(1e308,0,0)); translate_point(p, vec3(1e308,1,1))";
    let error = Program::compile(script)
        .unwrap()
        .run_with_host(1000, &Default::default(), &[], &functions)
        .unwrap_err();
    assert_eq!(error.message, "Point translation overflow");
    SCENE.with(|scene| {
        assert_eq!(*scene.borrow()[0].borrow(), [1e308, 0., 0.]);
        scene.borrow_mut().clear();
    });
}
