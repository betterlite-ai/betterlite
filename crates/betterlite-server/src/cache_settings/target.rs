use std::io;

use serde::Serialize as _;
use serde_json::Value;
use serde_json::ser::{Formatter, Serializer};

use super::REDACTED_VALUE;

#[must_use]
pub fn has_connection_target(value: &Value) -> bool {
    match value {
        Value::String(text) => !text.trim().is_empty() && text != REDACTED_VALUE,
        Value::Null => false,
        Value::Array(items) => !items.is_empty(),
        Value::Object(fields) => !fields.is_empty(),
        Value::Bool(_) | Value::Number(_) => true,
    }
}

#[must_use]
pub fn target_repr(value: &Value) -> String {
    match value {
        Value::Array(_) | Value::Object(_) => canonical_json(value),
        Value::Null => "None".to_owned(),
        Value::Bool(true) => "True".to_owned(),
        Value::Bool(false) => "False".to_owned(),
        Value::Number(number) => number.to_string(),
        Value::String(text) => text.clone(),
    }
}

// json.dumps(sort_keys=True) adds ", " and ": " separators that serde_json's compact formatter omits, so only those are overridden here.
fn canonical_json(value: &Value) -> String {
    let mut buffer = Vec::new();
    let mut serializer = Serializer::with_formatter(&mut buffer, PythonStyleFormatter);
    value.serialize(&mut serializer).unwrap_or_default();
    String::from_utf8(buffer).unwrap_or_default()
}

struct PythonStyleFormatter;

impl Formatter for PythonStyleFormatter {
    fn begin_array_value<W>(&mut self, writer: &mut W, first: bool) -> io::Result<()>
    where
        W: ?Sized + io::Write,
    {
        if first {
            Ok(())
        } else {
            writer.write_all(b", ")
        }
    }

    fn begin_object_key<W>(&mut self, writer: &mut W, first: bool) -> io::Result<()>
    where
        W: ?Sized + io::Write,
    {
        if first {
            Ok(())
        } else {
            writer.write_all(b", ")
        }
    }

    fn begin_object_value<W>(&mut self, writer: &mut W) -> io::Result<()>
    where
        W: ?Sized + io::Write,
    {
        writer.write_all(b": ")
    }
}
