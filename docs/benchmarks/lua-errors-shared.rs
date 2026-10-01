use mlua::Lua;
fn main() {
    let lua = Lua::new();
    let syntax = lua
        .load("local value =\n")
        .set_name("syntax-case.lua")
        .into_function()
        .unwrap_err();
    assert!(matches!(syntax, mlua::Error::SyntaxError { .. }));
    assert!(syntax.to_string().contains("syntax-case.lua\"]:2:"));
    assert_eq!(lua.load("return 1+2").eval::<i64>().unwrap(), 3);
    let program = lua
        .load("local value = nil\nreturn value.missing")
        .set_name("runtime-case.lua")
        .into_function()
        .unwrap();
    let runtime = program.call::<()>(()).unwrap_err();
    assert!(runtime.to_string().contains("runtime-case.lua\"]:2:"));
    assert_eq!(lua.load("return 1+2").eval::<i64>().unwrap(), 3);
    lua.globals()
        .set(
            "host_fail",
            lua.create_function(|_, ()| -> mlua::Result<()> {
                Err(mlua::Error::RuntimeError("host rejected operation".into()))
            })
            .unwrap(),
        )
        .unwrap();
    let host = lua
        .load("host_fail()")
        .set_name("host-case.lua")
        .exec()
        .unwrap_err();
    assert!(host.to_string().contains("host rejected operation"));
    assert!(host.to_string().contains("host-case.lua\"]:1:"));
    assert_eq!(lua.load("return 1+2").eval::<i64>().unwrap(), 3);
    // Catching an ordinary host error is distinct from cancellation/resource limits.
    let caught: bool = lua.load("local ok, err = pcall(host_fail)\nreturn not ok and string.find(tostring(err), 'host rejected operation', 1, true) ~= nil").eval().unwrap();
    assert!(caught);
    println!(
        "{}: syntax/runtime/host errors and VM reuse verified",
        env!("CARGO_PKG_NAME")
    );
    println!("SYNTAX\n{syntax}\nRUNTIME\n{runtime}\nHOST\n{host}");
}
