use koto::prelude::*;
fn main() {
    let mut koto = Koto::default();
    let result = koto.compile_and_run("1+2").unwrap();
    assert!(matches!(result, KValue::Number(n) if f64::from(n) == 3.0));
    println!("3");
}
