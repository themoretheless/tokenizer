use koto::{prelude::*, runtime};
use serde_json::{Value, json};
use std::{
    fs::File,
    io::{BufRead, BufReader, Read},
    sync::Mutex,
};

fn record(row: &Value) -> Result<KValue, String> {
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
    Ok(KList::with_data(
        [group.into(), amount.into(), active.into()]
            .into_iter()
            .collect(),
    )
    .into())
}
enum Input {
    Array(std::vec::IntoIter<KValue>),
    Lines(Option<BufReader<File>>),
}
impl Input {
    fn next(&mut self) -> Result<Option<KValue>, String> {
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
    let input = Mutex::new(input);
    let mut koto = Koto::default();
    koto.prelude().add_fn("next_record", move |_| {
        input
            .lock()
            .map_err(|_| runtime::Error::from("input lock poisoned"))?
            .next()
            .map(|row| row.unwrap_or(KValue::Null))
            .map_err(runtime::Error::from)
    });
    koto.prelude().add_fn("finite", |ctx| match ctx.args() {
        [KValue::Number(n)] => Ok(f64::from(n).is_finite().into()),
        unexpected => unexpected_args("|Number|", unexpected),
    });
    let KValue::List(result) = koto.compile_and_run(include_str!("../../records.koto"))? else {
        return Err("Expected result list".into());
    };
    let mut output = Vec::new();
    for row in result.data().iter() {
        let KValue::Map(row) = row else {
            return Err("Expected result map".into());
        };
        let Some(KValue::Str(group)) = row.get("group") else {
            return Err("Expected group string".into());
        };
        let Some(KValue::Number(total)) = row.get("total") else {
            return Err("Expected total number".into());
        };
        let total = f64::from(total);
        if !total.is_finite() {
            return Err("Expected finite total".into());
        }
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
