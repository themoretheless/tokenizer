use serde_json::{Value, json};
use std::{
    ffi::{c_char, c_int, c_void},
    fs::File,
    io::{BufRead, BufReader, Read},
};
type Row = (String, f64, bool);
fn record(row: &Value) -> Result<Row, String> {
    Ok((
        row.get("group")
            .and_then(Value::as_str)
            .ok_or("group must be a string")?
            .into(),
        row.get("amount")
            .and_then(Value::as_f64)
            .filter(|n| n.is_finite())
            .ok_or("amount must be finite")?,
        row.get("active")
            .and_then(Value::as_bool)
            .ok_or("active must be boolean")?,
    ))
}
enum Input {
    Array(std::vec::IntoIter<Row>),
    Lines(Option<BufReader<File>>),
}
impl Input {
    fn next(&mut self) -> Result<Option<Row>, String> {
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
                    let row = serde_json::from_str(&line).map_err(|e| e.to_string())?;
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
struct State {
    input: Input,
    current: Option<Row>,
    output: Vec<Value>,
    error: Option<String>,
}
unsafe extern "C" {
    fn rush_wren_records(
        context: *mut c_void,
        next: unsafe extern "C" fn(
            *mut c_void,
            *mut *const c_char,
            *mut usize,
            *mut f64,
            *mut bool,
        ) -> c_int,
        emit: unsafe extern "C" fn(*mut c_void, *const c_char, usize, f64) -> c_int,
    ) -> c_int;
}
unsafe extern "C" fn next(
    context: *mut c_void,
    group: *mut *const c_char,
    length: *mut usize,
    amount: *mut f64,
    active: *mut bool,
) -> c_int {
    // SAFETY: bridge.c invokes this synchronously with the State and valid output
    // pointers. The current row remains owned until the next callback; Wren copies it.
    let state = unsafe { &mut *context.cast::<State>() };
    match state.input.next() {
        Ok(row) => {
            state.current = row;
            let Some((key, value, enabled)) = &state.current else {
                return 0;
            };
            unsafe {
                *group = key.as_ptr().cast();
                *length = key.len();
                *amount = *value;
                *active = *enabled;
            }
            1
        }
        Err(error) => {
            state.error = Some(error);
            -1
        }
    }
}
unsafe extern "C" fn emit(
    context: *mut c_void,
    group: *const c_char,
    length: usize,
    total: f64,
) -> c_int {
    // SAFETY: bridge.c passes a live Wren byte string; copy it before returning.
    let state = unsafe { &mut *context.cast::<State>() };
    let bytes = unsafe { std::slice::from_raw_parts(group.cast::<u8>(), length) };
    match std::str::from_utf8(bytes) {
        Ok(group) => {
            state.output.push(json!({"group":group, "total":total}));
            1
        }
        Err(error) => {
            state.error = Some(error.to_string());
            0
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
        let input: Value = serde_json::from_slice(&std::fs::read(path)?)?;
        let rows = input.as_array().ok_or("Input must be an array")?;
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
    let mut state = State {
        input,
        current: None,
        output: Vec::new(),
        error: None,
    };
    // SAFETY: State remains live throughout the synchronous C call. Both callbacks
    // use the ABI declared in bridge.c; no pointers are retained after the call.
    let status = unsafe { rush_wren_records((&mut state as *mut State).cast(), next, emit) };
    if status != 0 {
        return Err(state.error.unwrap_or("Wren execution failed".into()).into());
    }
    Ok(Value::Array(state.output))
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
