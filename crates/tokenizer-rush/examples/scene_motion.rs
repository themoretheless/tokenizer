//! cargo run -p themoretheless-tokenizer-rush --example scene_motion
//! An application supplies its own scene state without a global registry.
use std::{cell::RefCell, rc::Rc};
use themoretheless_tokenizer_rush::{
    CancellationToken, ExecutionLimits, HostRegistration, Program, Value, ValueType,
    analyze_editor_with_host,
};

#[derive(Default)]
struct Scene {
    position: [f64; 3],
    updates: usize,
}

fn main() {
    let scene = Rc::new(RefCell::new(Scene::default()));
    let captured = scene.clone();
    let publish = HostRegistration::new(
        "publish_position",
        vec![ValueType::Vector(3)],
        ValueType::Vector(3),
        move |arguments, _| {
            let Value::Vector(position) = &arguments[0] else {
                unreachable!()
            };
            let mut scene = captured.borrow_mut();
            scene.position.copy_from_slice(position);
            scene.updates += 1;
            Ok(arguments[0].clone())
        },
    );
    let source = include_str!("scripts/scene-motion.r");
    let analysis = analyze_editor_with_host(source, &[], std::slice::from_ref(&publish.function));
    assert!(
        analysis.parsed.is_valid(),
        "{:?}",
        analysis.parsed.diagnostics
    );
    let program = Program::compile(source).unwrap();
    let token = CancellationToken::default();
    let limits = ExecutionLimits::new(10_000);
    let mut instance = program
        .instantiate(limits, &token, &[], &[], &[publish], &[])
        .unwrap();
    for _ in 0..120 {
        instance
            .call("update", &[Value::Number(1.0 / 60.0)], limits)
            .unwrap();
    }
    instance
        .call(
            "set_velocity",
            &[Value::Vector(vec![0.0, 3.0, 0.0])],
            limits,
        )
        .unwrap();
    let result = instance
        .call("update", &[Value::Number(0.5)], limits)
        .unwrap();
    let scene = scene.borrow();
    for (actual, expected) in scene.position.iter().zip([4.0, 1.5, -2.0]) {
        assert!((actual - expected).abs() < 1e-12);
    }
    assert_eq!(scene.updates, 121);
    assert_eq!(result, Value::Vector(scene.position.to_vec()));
    println!("{} updates; position = {:?}", scene.updates, scene.position);
}

#[test]
fn scene_motion_example() {
    main();
}
