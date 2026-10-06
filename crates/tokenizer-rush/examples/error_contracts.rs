use std::rc::Rc;
use themoretheless_tokenizer_rush::{
    CancellationToken, HostFunction, Program, RuntimeError, Value, ValueType,
};
fn fail<'s>(_: &[Value<'s>], _: &CancellationToken) -> Result<Value<'s>, String> {
    Err("host rejected operation".into())
}
fn show(name: &str, source: &str, error: &RuntimeError) {
    assert!(error.span.start <= error.span.end && error.span.end <= source.len());
    assert!(source.is_char_boundary(error.span.start) && source.is_char_boundary(error.span.end));
    let line = source[..error.span.start]
        .bytes()
        .filter(|b| *b == b'\n')
        .count()
        + 1;
    println!(
        "{name}:{line} bytes {}..{}: {}",
        error.span.start, error.span.end, error.message
    );
}
fn main() {
    let token = CancellationToken::default();
    let syntax_source = "let value =\n";
    let syntax = Program::compile(syntax_source)
        .err()
        .expect("Expected syntax error");
    show("syntax-case.r", syntax_source, &syntax);
    let runtime_source = "let value = null\nvalue.missing";
    let program = Program::compile(runtime_source).unwrap();
    let runtime = program.run(100, &token, &[]).unwrap_err();
    assert!(runtime.span.start > runtime_source.find('\n').unwrap());
    show("runtime-case.r", runtime_source, &runtime);
    let functions = [Rc::new(HostFunction {
        name: "host_fail",
        parameters: vec![],
        result: ValueType::Null,
        callback: fail,
    })];
    let host_source = "if fail > 0 { host_fail() } else { 3 }";
    let program = Program::compile(host_source).unwrap();
    let host = program
        .run_with_host(100, &token, &[("fail", 1.0)], &functions)
        .unwrap_err();
    assert!(host.message.contains("host rejected operation"));
    assert!(host_source[host.span.start..host.span.end].contains("host_fail"));
    show("host-case.r", host_source, &host);
    assert_eq!(
        program
            .run_with_host(100, &token, &[("fail", 0.0)], &functions)
            .unwrap(),
        Value::Number(3.0)
    );
    assert_eq!(
        Program::compile("1+2")
            .unwrap()
            .run(100, &token, &[])
            .unwrap(),
        Value::Number(3.0)
    );
    println!("Rush: error spans and reuse of the same prepared program verified");
}
