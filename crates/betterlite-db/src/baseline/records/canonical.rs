use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;
use serde_json::Number;

mod datetime;
mod escape;
mod float;
mod projection;

use super::BaselineAccountingRecord;

// A JSON tree whose objects are sorted by key and whose floats render as CPython's json.dumps renders them.
enum Canonical {
    Null,
    Bool(bool),
    Integer(i64),
    Float(f64),
    Text(String),
    List(Vec<Self>),
    Object(BTreeMap<String, Self>),
}

impl Canonical {
    fn insert(&mut self, key: &str, value: Self) {
        if let Self::Object(fields) = self {
            fields.insert(key.to_owned(), value);
        }
    }
}

impl From<&Json> for Canonical {
    fn from(value: &Json) -> Self {
        match value {
            Json::Null => Self::Null,
            Json::Bool(flag) => Self::Bool(*flag),
            Json::Number(number) => number_from_json(number),
            Json::Text(text) => Self::Text(text.clone()),
            Json::List(items) => Self::List(items.iter().map(Self::from).collect()),
            Json::Object(fields) => Self::Object(
                fields
                    .iter()
                    .map(|(key, child)| (key.clone(), Self::from(child)))
                    .collect(),
            ),
        }
    }
}

impl From<Decimal> for Canonical {
    fn from(value: Decimal) -> Self {
        Self::Float(value.to_f64().unwrap_or_default())
    }
}

impl From<Option<i64>> for Canonical {
    fn from(value: Option<i64>) -> Self {
        value.map_or(Self::Null, Self::Integer)
    }
}

fn number_from_json(number: &Number) -> Canonical {
    let lexeme = number.to_string();
    if let Some(whole) = number.as_i64()
        && !lexeme.contains(['.', 'e', 'E'])
    {
        return Canonical::Integer(whole);
    }
    Canonical::Float(number.as_f64().unwrap_or_default())
}

fn write_value(out: &mut String, value: &Canonical) {
    match value {
        Canonical::Null => out.push_str("null"),
        Canonical::Bool(true) => out.push_str("true"),
        Canonical::Bool(false) => out.push_str("false"),
        Canonical::Integer(whole) => out.push_str(&whole.to_string()),
        Canonical::Float(number) => out.push_str(&float::format(*number)),
        Canonical::Text(text) => escape::write(out, text),
        Canonical::List(items) => write_list(out, items),
        Canonical::Object(fields) => write_object(out, fields),
    }
}

fn write_list(out: &mut String, items: &[Canonical]) {
    out.push('[');
    for (index, item) in items.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        write_value(out, item);
    }
    out.push(']');
}

fn write_object(out: &mut String, fields: &BTreeMap<String, Canonical>) {
    out.push('{');
    for (index, (key, value)) in fields.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        escape::write(out, key);
        out.push(':');
        write_value(out, value);
    }
    out.push('}');
}

// _serialized: json.dumps(model_dump(mode="json"), sort_keys=True, separators=(",", ":")).
#[must_use]
pub fn serialize_record(record: &BaselineAccountingRecord) -> String {
    let mut out = String::new();
    write_value(&mut out, &projection::record(record));
    out
}
