use rhai::{Engine, EvalAltResult, FLOAT, Scope};
use std::{
    cell::Cell,
    rc::{Rc, Weak},
};

#[derive(Clone)]
struct Point(Weak<Cell<FLOAT>>);
fn read(point: &mut Point) -> Result<FLOAT, Box<EvalAltResult>> {
    point
        .0
        .upgrade()
        .map(|p| p.get())
        .ok_or_else(|| "point was deleted".into())
}
fn main() {
    let mut engine = Engine::new();
    engine.set_max_operations(10000);
    engine.register_type_with_name::<Point>("Point");
    engine.register_fn("read_point", read);
    engine.register_fn(
        "shift",
        |point: &mut Point, delta: FLOAT| -> Result<(), Box<EvalAltResult>> {
            let owner = point
                .0
                .upgrade()
                .ok_or_else(|| Box::<EvalAltResult>::from("point was deleted"))?;
            let value = owner.get() + delta;
            if !value.is_finite() {
                return Err("position must be finite".into());
            }
            owner.set(value);
            Ok(())
        },
    );
    let owner = Rc::new(Cell::new(2.0));
    let mut scope = Scope::new();
    scope.push("point", Point(Rc::downgrade(&owner)));
    let result = engine
        .eval_with_scope::<FLOAT>(
            &mut scope,
            "let alias = point; point.shift(3.0); read_point(alias)",
        )
        .unwrap();
    assert_eq!(result, 5.0);
    assert_eq!(owner.get(), 5.0);
    assert!(
        engine
            .eval_with_scope::<FLOAT>(&mut scope, "read_point(42)")
            .is_err()
    );
    scope.push("infinite", FLOAT::INFINITY);
    let error = engine
        .eval_with_scope::<()>(&mut scope, "point.shift(infinite)")
        .unwrap_err();
    assert!(error.to_string().contains("position must be finite"));
    assert_eq!(owner.get(), 5.0);
    drop(owner);
    for source in ["read_point(point)", "read_point(alias)", "point.shift(1.0)"] {
        let error = engine
            .eval_with_scope::<rhai::Dynamic>(&mut scope, source)
            .unwrap_err();
        assert!(error.to_string().contains("point was deleted"));
    }
    assert_eq!(
        engine
            .eval_with_scope::<rhai::INT>(&mut scope, "1+2")
            .unwrap(),
        3
    );
    println!(
        "Rhai 1.26.1: alias mutation, wrong argument type, nonfinite rejection, deleted owner and engine reuse passed"
    );
}
