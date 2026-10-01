use koto::{derive::*, prelude::*, runtime};
use std::sync::{Arc, Mutex, Weak};
#[derive(Clone, KotoCopy, KotoType)]
struct Point(Weak<Mutex<f64>>);
#[koto_impl]
impl Point {
    #[koto_method]
    fn get(&self) -> runtime::Result<KValue> {
        let owner = self
            .0
            .upgrade()
            .ok_or_else(|| runtime::Error::from("point was deleted"))?;
        let value = *owner.lock().unwrap();
        Ok(value.into())
    }
    #[koto_method]
    fn shift(ctx: MethodContext<Self>) -> runtime::Result<KValue> {
        let delta = match ctx.args {
            [KValue::Number(n)] => f64::from(n),
            unexpected => return unexpected_args("|Number|", unexpected),
        };
        let owner = ctx
            .instance()?
            .0
            .upgrade()
            .ok_or_else(|| runtime::Error::from("point was deleted"))?;
        let mut value = owner.lock().unwrap();
        let next = *value + delta;
        if !next.is_finite() {
            return Err("position must be finite".into());
        }
        *value = next;
        Ok(KValue::Null)
    }
}
impl KotoObject for Point {}
fn main() {
    let mut koto = Koto::default();
    let owner = Arc::new(Mutex::new(2.0));
    koto.prelude()
        .insert("point", KObject::from(Point(Arc::downgrade(&owner))));
    koto.prelude().insert("infinite", f64::INFINITY);
    koto.prelude().add_fn("read_point", |ctx| match ctx.args() {
        [KValue::Object(object)] => object.cast::<Point>()?.get(),
        unexpected => unexpected_args("|Point|", unexpected),
    });
    let result = koto
        .compile_and_run("alias = point\npoint.shift 3.0\nread_point alias")
        .unwrap();
    assert!(matches!(result, KValue::Number(n) if f64::from(n) == 5.0));
    assert_eq!(*owner.lock().unwrap(), 5.0);
    assert!(koto.compile_and_run("read_point 42").is_err());
    assert!(
        koto.compile_and_run("point.shift infinite")
            .unwrap_err()
            .to_string()
            .contains("position must be finite")
    );
    assert_eq!(*owner.lock().unwrap(), 5.0);
    // Keep a second script-visible handle independently of transient local bindings.
    koto.prelude()
        .insert("alias", KObject::from(Point(Arc::downgrade(&owner))));
    drop(owner);
    for source in ["point.get()", "alias.get()", "read_point alias"] {
        assert!(
            koto.compile_and_run(source)
                .unwrap_err()
                .to_string()
                .contains("point was deleted")
        );
    }
    assert!(
        matches!(koto.compile_and_run("1+2").unwrap(), KValue::Number(n) if f64::from(n) == 3.0)
    );
    println!(
        "Koto 0.16.1: alias mutation, wrong argument type, nonfinite rejection, deleted owner and VM reuse passed"
    );
}
