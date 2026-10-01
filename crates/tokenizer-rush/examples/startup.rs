use themoretheless_tokenizer_rush::{CancellationToken, Program, Value};
fn main() {
    let result = Program::compile("1+2")
        .unwrap()
        .run(100, &CancellationToken::default(), &[])
        .unwrap();
    assert_eq!(result, Value::Number(3.0));
    println!("3");
}
