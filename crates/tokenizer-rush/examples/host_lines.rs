//! cargo run -p themoretheless-tokenizer-rush --example host_lines -- path/to/file
//! The embedding app supplies the path; Rush receives only the registered lines() source.
use std::{
    fs::File,
    io::{BufRead, BufReader, Lines},
    path::PathBuf,
    rc::Rc,
};
use themoretheless_tokenizer_rush::{
    CancellationToken, HostFunction, HostSequence, HostSequenceIterator, Program, Value, ValueType,
};

#[derive(Debug)]
struct FileSource(PathBuf);
struct FileIterator(Lines<BufReader<File>>);
impl HostSequence for FileSource {
    fn open(&self, _: &CancellationToken) -> Result<Box<dyn HostSequenceIterator>, String> {
        let file = File::open(&self.0).map_err(|error| error.to_string())?;
        Ok(Box::new(FileIterator(BufReader::new(file).lines())))
    }
}
impl HostSequenceIterator for FileIterator {
    fn next(&mut self, cancellation: &CancellationToken) -> Result<Option<Value<'static>>, String> {
        if cancellation.is_cancelled() {
            return Err("Execution cancelled".into());
        }
        // Blocking filesystem reads remain the host's responsibility. BufReader/File
        // close through normal Drop when the runtime drops this iterator.
        self.0
            .next()
            .transpose()
            .map(|line| line.map(Value::String))
            .map_err(|error| error.to_string())
    }
}
fn lines<'s>(_: &[Value<'s>], _: &CancellationToken) -> Result<Value<'s>, String> {
    let path = std::env::args_os().nth(1).ok_or("Pass a text file path")?;
    Ok(Value::host_sequence(
        Rc::new(FileSource(path.into())),
        ValueType::String,
    ))
}
fn main() {
    let program = Program::compile("lines() | filter(line => len(line) > 0) | collect(3)").unwrap();
    let function = Rc::new(HostFunction {
        name: "lines",
        parameters: vec![],
        result: ValueType::Sequence,
        callback: lines,
    });
    match program.run_with_host(100_000, &CancellationToken::default(), &[], &[function]) {
        Ok(value) => println!("{value:?}"),
        Err(error) => {
            eprintln!("{}", error.message);
            std::process::exit(1);
        }
    }
}
