use mlua::{AnyUserData, Lua, UserData, UserDataMethods};
use std::{
    cell::Cell,
    rc::{Rc, Weak},
};

struct Point(Weak<Cell<f64>>);
impl UserData for Point {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("get", |_, point, ()| {
            point
                .0
                .upgrade()
                .map(|p| p.get())
                .ok_or_else(|| mlua::Error::RuntimeError("point was deleted".into()))
        });
        methods.add_method("move", |_, point, delta: f64| {
            let owner = point
                .0
                .upgrade()
                .ok_or_else(|| mlua::Error::RuntimeError("point was deleted".into()))?;
            let next = owner.get() + delta;
            if !next.is_finite() {
                return Err(mlua::Error::RuntimeError("position must be finite".into()));
            }
            owner.set(next);
            Ok(())
        });
    }
}
fn main() {
    let lua = Lua::new();
    let owner = Rc::new(Cell::new(2.0));
    lua.globals()
        .set("point", Point(Rc::downgrade(&owner)))
        .unwrap();
    lua.globals()
        .set(
            "read_point",
            lua.create_function(|_, value: AnyUserData| {
                let point = value.borrow::<Point>()?;
                point
                    .0
                    .upgrade()
                    .map(|p| p.get())
                    .ok_or_else(|| mlua::Error::RuntimeError("point was deleted".into()))
            })
            .unwrap(),
        )
        .unwrap();
    let result: f64 = lua
        .load("alias = point\npoint:move(3)\nreturn read_point(alias)")
        .eval()
        .unwrap();
    assert_eq!(result, 5.0);
    assert_eq!(owner.get(), 5.0);
    assert!(lua.load("return read_point(42)").eval::<f64>().is_err());
    let overflow = lua.load("point:move(math.huge)").exec().unwrap_err();
    assert!(overflow.to_string().contains("position must be finite"));
    assert_eq!(owner.get(), 5.0);
    drop(owner);
    for source in [
        "return point:get()",
        "return alias:get()",
        "return read_point(alias)",
    ] {
        assert!(
            lua.load(source)
                .eval::<f64>()
                .unwrap_err()
                .to_string()
                .contains("point was deleted")
        );
    }
    assert_eq!(lua.load("return 1+2").eval::<i64>().unwrap(), 3);
    println!(
        "{}: alias mutation, wrong argument type, nonfinite rejection, deleted owner and VM reuse passed",
        env!("CARGO_PKG_NAME")
    );
}
