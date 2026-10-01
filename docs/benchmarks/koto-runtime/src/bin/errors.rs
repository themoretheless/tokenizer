use koto::prelude::*;
fn reuse(koto: &mut Koto) {
    assert!(matches!(koto.compile_and_run("1+2").unwrap(), KValue::Number(n) if f64::from(n)==3.0));
}
fn main() {
    let mut koto = Koto::default();
    koto.prelude()
        .add_fn("host_fail", |_| Err("host rejected operation".into()));
    let syntax = koto
        .compile(CompileArgs::new("value =\n").script_path("syntax-case.koto"))
        .unwrap_err();
    assert!(syntax.to_string().contains("syntax-case.koto"));
    reuse(&mut koto);
    let program = koto
        .compile(CompileArgs::new("value = null\nvalue.missing").script_path("runtime-case.koto"))
        .unwrap();
    let runtime = koto.run(program).unwrap_err();
    assert!(runtime.to_string().contains("runtime-case.koto"));
    reuse(&mut koto);
    let host = koto
        .compile_and_run(CompileArgs::new("host_fail()").script_path("host-case.koto"))
        .unwrap_err();
    assert!(host.to_string().contains("host rejected operation"));
    assert!(host.to_string().contains("host-case.koto"));
    reuse(&mut koto);
    println!("Koto 0.16.1: compile/runtime/host errors, source names and VM reuse verified");
    println!("SYNTAX\n{syntax}\nRUNTIME\n{runtime}\nHOST\n{host}");
}
