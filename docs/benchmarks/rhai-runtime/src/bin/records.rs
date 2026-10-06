use rhai::{Array, Dynamic, Engine, EvalAltResult, ImmutableString, Map};
use serde_json::{Value, json};
use std::{
    cell::RefCell,
    fs::File,
    io::{BufRead, BufReader, Read},
};

fn record(row: &Value) -> Result<Array, String> {
    let group = row
        .get("group")
        .and_then(Value::as_str)
        .ok_or("group must be a string")?;
    let amount = row
        .get("amount")
        .and_then(Value::as_f64)
        .filter(|n| n.is_finite())
        .ok_or("amount must be finite")?;
    let active = row
        .get("active")
        .and_then(Value::as_bool)
        .ok_or("active must be boolean")?;
    Ok(vec![
        Dynamic::from(group.to_owned()),
        Dynamic::from_float(amount),
        Dynamic::from_bool(active),
    ])
}
enum Input {
    Array(std::vec::IntoIter<Array>),
    Lines(Option<BufReader<File>>),
}
impl Input {
    fn next(&mut self) -> Result<Option<Array>, String> {
        match self {
            Self::Array(rows) => Ok(rows.next()),
            Self::Lines(reader) => {
                let Some(stream) = reader.as_mut() else {
                    return Ok(None);
                };
                let result = (|| {
                    let mut line = String::new();
                    let bytes = stream
                        .take(65537)
                        .read_line(&mut line)
                        .map_err(|e| e.to_string())?;
                    if bytes == 0 {
                        return Ok(None);
                    }
                    if bytes > 65536 {
                        return Err("line exceeds byte limit".into());
                    }
                    let row: Value = serde_json::from_str(&line).map_err(|e| e.to_string())?;
                    record(&row).map(Some)
                })();
                if !matches!(result, Ok(Some(_))) {
                    *reader = None;
                }
                result
            }
        }
    }
}
fn run() -> Result<Value, Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    let first = args.next().ok_or("Expected input path")?;
    let jsonl = first == "--jsonl";
    let path = if jsonl {
        args.next().ok_or("Expected JSONL path")?
    } else {
        first
    };
    if args.next().is_some() {
        return Err("Expected exactly one input path".into());
    }
    let input = if jsonl {
        Input::Lines(Some(BufReader::new(File::open(path)?)))
    } else {
        let value: Value = serde_json::from_slice(&std::fs::read(path)?)?;
        let rows = value.as_array().ok_or("Input must be an array")?;
        if rows.len() > 1000 {
            return Err("At most 1000 rows are allowed".into());
        }
        Input::Array(
            rows.iter()
                .map(record)
                .collect::<Result<Vec<_>, _>>()?
                .into_iter(),
        )
    };
    let input = RefCell::new(input);
    let mut engine = Engine::new();
    engine.set_max_operations(10_000_000);
    engine.register_fn(
        "next_record",
        move || -> Result<Dynamic, Box<EvalAltResult>> {
            input
                .borrow_mut()
                .next()
                .map(|row| row.map(Dynamic::from_array).unwrap_or(Dynamic::UNIT))
                .map_err(Into::into)
        },
    );
    engine.register_fn("finite", f64::is_finite);
    let result: Array = engine.eval(include_str!("../../records.rhai"))?;
    let mut output = Vec::new();
    for row in result {
        let row = row.try_cast::<Map>().ok_or("Expected result map")?;
        let group = row
            .get("group")
            .and_then(|v| v.clone().try_cast::<ImmutableString>())
            .ok_or("Expected group string")?;
        let total = row
            .get("total")
            .and_then(|v| v.clone().try_cast::<f64>())
            .filter(|n| n.is_finite())
            .ok_or("Expected finite total")?;
        output.push(json!({"group":group.as_str(), "total":total}));
    }
    Ok(Value::Array(output))
}
fn main() {
    match run() {
        Ok(result) => println!("{result}"),
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1);
        }
    }
}
