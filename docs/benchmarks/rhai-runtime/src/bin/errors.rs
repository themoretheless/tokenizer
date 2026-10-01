use rhai::{Engine, EvalAltResult, INT};
fn main() {
    let mut engine = Engine::new();
    engine.register_fn("host_fail", || -> Result<(), Box<EvalAltResult>> {
        Err("host rejected operation".into())
    });
    let syntax = engine.compile("let value =\n").unwrap_err();
    assert!(syntax.position().line().is_some());
    assert_eq!(engine.eval::<INT>("1+2").unwrap(), 3);
    let mut program = engine.compile("let value = ();\nvalue.missing").unwrap();
    program.set_source("runtime-case.rhai");
    let runtime = engine.eval_ast::<rhai::Dynamic>(&program).unwrap_err();
    assert_eq!(runtime.position().line(), Some(2));
    assert_eq!(engine.eval::<INT>("1+2").unwrap(), 3);
    let host = engine.eval::<()>("host_fail()").unwrap_err();
    assert!(host.to_string().contains("host rejected operation"));
    assert_eq!(host.position().line(), Some(1));
    assert_eq!(engine.eval::<INT>("1+2").unwrap(), 3);
    let caught = engine
        .eval::<String>(
            "try { host_fail(); } catch (error) { return error; } return \"not caught\";",
        )
        .unwrap();
    assert!(caught.contains("host rejected operation"));
    println!("Rhai 1.26.1: compile/runtime/host errors, positions and engine reuse verified");
    println!("SYNTAX\n{syntax}\nRUNTIME\n{runtime}\nHOST\n{host}");
}
