fn main() {
    let lua = mlua::Lua::new();
    let result: f64 = lua.load("return 1+2").eval().unwrap();
    assert_eq!(result, 3.0);
    println!("3");
}
