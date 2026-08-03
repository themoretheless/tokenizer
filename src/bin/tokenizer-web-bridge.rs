//! Standard-input adapter used by the local Vue playground.

use std::{
    env,
    io::{self, Read},
};

use themoretheless_tokenizer::web_bridge::tokenization_json;

fn main() {
    let arguments: Vec<String> = env::args().skip(1).collect();
    let mode = argument_value(&arguments, "--mode").unwrap_or("strict");
    let layer = argument_value(&arguments, "--layer").unwrap_or("semantic");
    let mut source = String::new();
    if let Err(error) = io::stdin().read_to_string(&mut source) {
        eprintln!("failed to read source: {error}");
        std::process::exit(2);
    }
    println!("{}", tokenization_json(&source, mode, layer));
}

fn argument_value<'a>(arguments: &'a [String], name: &str) -> Option<&'a str> {
    arguments
        .windows(2)
        .find(|pair| pair[0] == name)
        .map(|pair| pair[1].as_str())
}
