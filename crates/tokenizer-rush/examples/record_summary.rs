//! cargo run -p themoretheless-tokenizer-rush --example record_summary -- input.json
//! File access and JSON conversion belong to this host, not to Rush.
use std::{
    cell::RefCell,
    fs::File,
    io::{BufRead, BufReader, Read},
    path::PathBuf,
    rc::Rc,
};
use themoretheless_tokenizer_rush::{
    CancellationToken, ExecutionLimits, HostFunction, HostSequence, HostSequenceIterator, Program,
    Value, ValueType,
};
thread_local! {
    static ROWS: RefCell<Vec<Value<'static>>> = const { RefCell::new(Vec::new()) };
    static SOURCE: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
}
fn records<'s>(_: &[Value<'s>], _: &CancellationToken) -> Result<Value<'s>, String> {
    if let Some(path) = SOURCE.with(|path| path.borrow().clone()) {
        return Ok(Value::host_sequence(Rc::new(JsonLines(path)), row_type()));
    }
    Ok(Value::List(ROWS.with(|rows| rows.borrow().clone())))
}
fn input(text: &str) -> Result<Vec<Value<'static>>, String> {
    let json: serde_json::Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    let rows = json.as_array().ok_or("Input must be an array")?;
    if rows.len() > 1000 {
        return Err("At most 1000 rows are allowed".into());
    }
    rows.iter()
        .enumerate()
        .map(|(i, row)| row_value(row).map_err(|e| format!("Row {i}: {e}")))
        .collect()
}
fn row_type() -> ValueType {
    ValueType::Tuple(vec![ValueType::String, ValueType::Number, ValueType::Bool])
}
fn row_value(row: &serde_json::Value) -> Result<Value<'static>, String> {
    let group = row
        .get("group")
        .and_then(|v| v.as_str())
        .ok_or("group must be a string")?;
    let amount = row
        .get("amount")
        .and_then(|v| v.as_f64())
        .filter(|n| n.is_finite())
        .ok_or("amount must be finite")?;
    let active = row
        .get("active")
        .and_then(|v| v.as_bool())
        .ok_or("active must be boolean")?;
    Ok(Value::Tuple(vec![
        Value::String(group.into()),
        Value::Number(amount),
        Value::Bool(active),
    ]))
}
const MAX_LINE_BYTES: usize = 65536;
#[derive(Debug)]
struct JsonLines(PathBuf);
struct JsonLineReader<R> {
    reader: R,
    line: usize,
}
impl HostSequence for JsonLines {
    fn open(&self, _: &CancellationToken) -> Result<Box<dyn HostSequenceIterator>, String> {
        let file = File::open(&self.0).map_err(|e| e.to_string())?;
        Ok(Box::new(JsonLineReader {
            reader: BufReader::new(file),
            line: 0,
        }))
    }
}
impl<R: BufRead> HostSequenceIterator for JsonLineReader<R> {
    fn next(&mut self, token: &CancellationToken) -> Result<Option<Value<'static>>, String> {
        if token.is_cancelled() {
            return Err("Execution cancelled".into());
        }
        let mut text = String::new();
        // Take bounds allocation even for an unterminated or oversized physical line.
        let bytes = self
            .reader
            .by_ref()
            .take((MAX_LINE_BYTES + 1) as u64)
            .read_line(&mut text)
            .map_err(|e| format!("Line {}: {e}", self.line + 1))?;
        if token.is_cancelled() {
            return Err("Execution cancelled".into());
        }
        if bytes == 0 {
            return Ok(None);
        }
        self.line += 1;
        if bytes > MAX_LINE_BYTES {
            return Err(format!(
                "Line {}: exceeds {MAX_LINE_BYTES} bytes",
                self.line
            ));
        }
        let row = serde_json::from_str(&text).map_err(|e| format!("Line {}: {e}", self.line))?;
        row_value(&row)
            .map(Some)
            .map_err(|e| format!("Line {}: {e}", self.line))
    }
}

fn output(value: Value<'_>) -> Result<serde_json::Value, String> {
    Ok(match value {
        Value::Null => serde_json::Value::Null,
        Value::Bool(v) => v.into(),
        Value::String(v) => v.into(),
        Value::Number(v) => serde_json::Number::from_f64(v)
            .ok_or("Non-finite JSON number")?
            .into(),
        Value::List(values) | Value::Tuple(values) => {
            serde_json::Value::Array(values.into_iter().map(output).collect::<Result<_, _>>()?)
        }
        Value::Record(fields) => serde_json::Value::Object(
            fields
                .into_iter()
                .map(|(key, value)| Ok((key, output(value)?)))
                .collect::<Result<_, String>>()?,
        ),
        _ => return Err("Result contains a value that JSON cannot represent".into()),
    })
}
fn summarize(text: &str) -> Result<serde_json::Value, String> {
    let rows = input(text)?;
    ROWS.with(|slot| *slot.borrow_mut() = rows);
    let result = execute(ValueType::List(Box::new(row_type())));
    ROWS.with(|slot| slot.borrow_mut().clear());
    result
}
fn execute(result_type: ValueType) -> Result<serde_json::Value, String> {
    let function = Rc::new(HostFunction {
        name: "records",
        parameters: vec![],
        result: result_type,
        callback: records,
    });
    let program =
        Program::compile(include_str!("scripts/record-summary.r")).map_err(|e| e.message)?;
    let result = program
        .run_with_limits(
            ExecutionLimits {
                max_collection_items: 1000,
                max_string_bytes: 65536,
                ..ExecutionLimits::new(10_000_000)
            },
            &CancellationToken::default(),
            &[],
            &[function],
            &[],
        )
        .map_err(|e| e.message)?;
    output(result)
}
fn summarize_lines(path: PathBuf) -> Result<serde_json::Value, String> {
    SOURCE.with(|slot| *slot.borrow_mut() = Some(path));
    let result = execute(ValueType::Sequence);
    SOURCE.with(|slot| *slot.borrow_mut() = None);
    result
}

fn main() -> Result<(), String> {
    let mut args = std::env::args_os().skip(1);
    let first = args.next().ok_or("Pass [--jsonl] input path")?;
    let result = if first == "--jsonl" {
        let path = args.next().ok_or("Pass a JSON Lines path")?;
        if args.next().is_some() {
            return Err("Unexpected argument".into());
        }
        summarize_lines(path.into())?
    } else {
        if args.next().is_some() {
            return Err("Unexpected argument".into());
        }
        let text = std::fs::read_to_string(first).map_err(|e| e.to_string())?;
        summarize(&text)?
    };
    println!(
        "{}",
        serde_json::to_string_pretty(&result).map_err(|e| e.to_string())?
    );
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn filters_groups_and_serializes_unicode_and_quotes() {
        let result = summarize(r#"[{"group":"A\"Б","amount":2.5,"active":true},{"group":"B","amount":99,"active":false},{"group":"A\"Б","amount":-1,"active":true},{"group":"C","amount":4,"active":true}]"#).unwrap();
        assert_eq!(
            result,
            serde_json::json!([{"group":"A\"Б","total":1.5},{"group":"C","total":4.0}])
        );
        let serialized = serde_json::to_string(&result).unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&serialized).unwrap(),
            result
        );
        assert_eq!(summarize("[]").unwrap(), serde_json::json!([]));
    }
    #[test]
    fn validates_input_and_cleans_host_data_after_runtime_error() {
        assert!(
            summarize(r#"[{"group":"a","amount":"bad","active":true}]"#)
                .unwrap_err()
                .contains("Row 0")
        );
        assert!(summarize(r#"[{"group":"a","amount":1e308,"active":true},{"group":"a","amount":1e308,"active":true}]"#).is_err());
        ROWS.with(|rows| assert!(rows.borrow().is_empty()));
    }
}

#[cfg(test)]
mod streaming_tests {
    use super::*;
    use std::io::{Cursor, Write};
    #[test]
    fn reads_crlf_and_final_line_without_newline() {
        let text = b"{\"group\":\"a\",\"amount\":1,\"active\":true}\r\n{\"group\":\"b\",\"amount\":2,\"active\":false}";
        let mut reader = JsonLineReader {
            reader: Cursor::new(text),
            line: 0,
        };
        let token = CancellationToken::default();
        assert!(reader.next(&token).unwrap().is_some());
        assert!(reader.next(&token).unwrap().is_some());
        assert!(reader.next(&token).unwrap().is_none());
        assert_eq!(reader.line, 2);
    }
    #[test]
    fn caps_physical_line_before_parsing_and_reports_line_number() {
        let token = CancellationToken::default();
        let row = r#"{"group":"a","amount":1,"active":true}"#;
        let exact = format!("{row}{}", " ".repeat(MAX_LINE_BYTES - row.len()));
        let mut reader = JsonLineReader {
            reader: Cursor::new(exact),
            line: 0,
        };
        assert!(reader.next(&token).unwrap().is_some());
        let mut reader = JsonLineReader {
            reader: Cursor::new(vec![b' '; MAX_LINE_BYTES * 2]),
            line: 0,
        };
        assert!(reader.next(&token).unwrap_err().contains("Line 1: exceeds"));
        assert_eq!(reader.reader.position(), (MAX_LINE_BYTES + 1) as u64);
        let mut reader = JsonLineReader {
            reader: Cursor::new(format!("{row}\ninvalid")),
            line: 0,
        };
        reader.next(&token).unwrap();
        assert!(reader.next(&token).unwrap_err().contains("Line 2"));
    }
    #[test]
    fn cancellation_before_read_leaves_input_untouched() {
        let token = CancellationToken::default();
        token.cancel();
        let mut reader = JsonLineReader {
            reader: Cursor::new(b"bad"),
            line: 0,
        };
        assert!(reader.next(&token).unwrap_err().contains("cancelled"));
        assert_eq!(reader.reader.position(), 0);
    }
    #[test]
    fn aggregates_ten_thousand_file_records_without_input_list_limit() {
        let path = std::env::temp_dir().join(format!("rush-jsonl-{}.jsonl", std::process::id()));
        {
            let mut file = std::io::BufWriter::new(File::create(&path).unwrap());
            for i in 0..10000 {
                writeln!(
                    file,
                    "{{\"group\":\"{}\",\"amount\":1,\"active\":true}}",
                    if i % 2 == 0 { "a" } else { "b" }
                )
                .unwrap();
            }
            file.flush().unwrap();
        }
        let result = summarize_lines(path.clone());
        std::fs::remove_file(path).unwrap();
        assert_eq!(
            result.unwrap(),
            serde_json::json!([{"group":"a","total":5000.0},{"group":"b","total":5000.0}])
        );
        SOURCE.with(|slot| assert!(slot.borrow().is_none()));
    }
}
