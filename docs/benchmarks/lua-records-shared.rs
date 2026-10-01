//! Like the Rush adapter, the host parses and validates JSON before passing rows to Lua.
use mlua::{Lua, Table};
use serde_json::{Value, json};
use std::{
    fs::File,
    io::{BufRead, BufReader, Read},
};

fn record(lua: &Lua, row: &Value) -> mlua::Result<Table> {
    let invalid = mlua::Error::runtime;
    let group = row
        .get("group")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid("group must be a string"))?;
    let amount = row
        .get("amount")
        .and_then(Value::as_f64)
        .filter(|n| n.is_finite())
        .ok_or_else(|| invalid("amount must be finite"))?;
    let active = row
        .get("active")
        .and_then(Value::as_bool)
        .ok_or_else(|| invalid("active must be boolean"))?;
    let record = lua.create_table()?;
    record.set(1, group)?;
    record.set(2, amount)?;
    record.set(3, active)?;
    Ok(record)
}

fn next_line(reader: &mut impl BufRead) -> mlua::Result<Option<Value>> {
    let mut text = String::new();
    let bytes = reader
        .take(65537)
        .read_line(&mut text)
        .map_err(mlua::Error::external)?;
    if bytes == 0 {
        return Ok(None);
    }
    if bytes > 65536 {
        return Err(mlua::Error::runtime("line exceeds byte limit"));
    }
    serde_json::from_str(&text)
        .map(Some)
        .map_err(mlua::Error::external)
}

fn run() -> Result<Value, Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    let first = args.next().ok_or("Expected JSON input path")?;
    let jsonl = first == "--jsonl";
    let path = if jsonl {
        args.next().ok_or("Expected JSONL input path")?
    } else {
        first
    };
    if args.next().is_some() {
        return Err("Expected exactly one input path".into());
    }
    let lua = Lua::new();
    let records = if jsonl {
        let mut reader = Some(BufReader::new(File::open(path)?));
        lua.create_function_mut(move |lua, ()| {
            let Some(stream) = reader.as_mut() else {
                return Ok(None);
            };
            let result = next_line(stream)
                .and_then(|row| row.as_ref().map(|row| record(lua, row)).transpose());
            if !matches!(result, Ok(Some(_))) {
                reader = None;
            }
            result
        })?
    } else {
        let input: Value = serde_json::from_slice(&std::fs::read(path)?)?;
        let rows = input.as_array().ok_or("Input must be an array")?;
        if rows.len() > 1000 {
            return Err("At most 1000 rows are allowed".into());
        }
        let mut rows = rows
            .iter()
            .map(|row| record(&lua, row))
            .collect::<mlua::Result<Vec<_>>>()?
            .into_iter();
        lua.create_function_mut(move |_, ()| Ok(rows.next()))?
    };
    lua.globals().set("records", records)?;
    let result: Table = lua.load(include_str!("lua-records.lua")).eval()?;
    let mut output = Vec::new();
    for row in result.sequence_values::<Table>() {
        let row = row?;
        let group: String = row.get("group")?;
        let total: f64 = row.get("total")?;
        if !total.is_finite() {
            return Err("Non-finite JSON number".into());
        }
        output.push(json!({"group": group, "total": total}));
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn reader_stops_at_each_line_and_at_the_byte_limit() {
        let mut source = Cursor::new(b"{\"a\":1}\n{\"a\":2}");
        assert_eq!(next_line(&mut source).unwrap(), Some(json!({"a":1})));
        assert_eq!(source.position(), 8);
        assert_eq!(next_line(&mut source).unwrap(), Some(json!({"a":2})));
        assert_eq!(next_line(&mut source).unwrap(), None);

        let mut source = Cursor::new(vec![b' '; 100_000]);
        assert!(
            next_line(&mut source)
                .unwrap_err()
                .to_string()
                .contains("byte limit")
        );
        assert_eq!(source.position(), 65537);
    }
}
