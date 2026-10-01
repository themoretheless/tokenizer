fn main() {
    let engine = rhai::Engine::new();
    let result: rhai::INT = engine.eval("1+2").unwrap();
    assert_eq!(result, 3);
    println!("3");
}
