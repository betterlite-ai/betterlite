use std::collections::BTreeMap;

use toml_edit::{Array, ArrayOfTables, InlineTable, Item, Table, Value};

pub(crate) fn item(item: &Item) -> String {
    match item {
        Item::None => "null".to_owned(),
        Item::Value(value) => value_text(value),
        Item::Table(table) => table_text(table),
        Item::ArrayOfTables(tables) => array_of_tables_text(tables),
    }
}

fn value_text(value: &Value) -> String {
    match value {
        Value::String(text) => json_string(text.value()),
        Value::Integer(number) => number.value().to_string(),
        Value::Float(number) => float_text(*number.value()),
        Value::Boolean(flag) => flag.value().to_string(),
        Value::Datetime(moment) => json_string(&moment.value().to_string()),
        Value::Array(array) => array_text(array),
        Value::InlineTable(table) => inline_table_text(table),
    }
}

// json.dumps writes a TOML float through float.__repr__: NaN and Infinity lower their own spelling
fn float_text(value: f64) -> String {
    if value.is_nan() {
        return "NaN".to_owned();
    }
    if value.is_infinite() {
        return if value.is_sign_positive() {
            "Infinity".to_owned()
        } else {
            "-Infinity".to_owned()
        };
    }
    serde_json::to_string(&value).unwrap_or_else(|_error| value.to_string())
}

fn array_text(array: &Array) -> String {
    let mut out = String::from("[");
    for (position, value) in array.iter().enumerate() {
        if position != 0 {
            out.push_str(", ");
        }
        out.push_str(&value_text(value));
    }
    out.push(']');
    out
}

fn array_of_tables_text(tables: &ArrayOfTables) -> String {
    let mut out = String::from("[");
    for (position, table) in tables.iter().enumerate() {
        if position != 0 {
            out.push_str(", ");
        }
        out.push_str(&table_text(table));
    }
    out.push(']');
    out
}

fn table_text(table: &Table) -> String {
    let pairs: BTreeMap<&str, String> = table
        .iter()
        .map(|(key, value)| (key, item(value)))
        .collect();
    object_text(&pairs)
}

fn inline_table_text(table: &InlineTable) -> String {
    let pairs: BTreeMap<&str, String> = table
        .iter()
        .map(|(key, value)| (key, value_text(value)))
        .collect();
    object_text(&pairs)
}

// json.dumps sorts object keys by code point and puts ", " and ": " between them
fn object_text(pairs: &BTreeMap<&str, String>) -> String {
    let mut out = String::from("{");
    for (position, (key, value)) in pairs.iter().enumerate() {
        if position != 0 {
            out.push_str(", ");
        }
        out.push_str(&json_string(key));
        out.push_str(": ");
        out.push_str(value);
    }
    out.push('}');
    out
}

// json.dumps escapes everything outside the 0x20-0x7e range as \uXXXX by default (ensure_ascii=True)
fn json_string(value: &str) -> String {
    let mut out = String::from("\"");
    for character in value.chars() {
        match character {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            printable_ascii if (' '..='~').contains(&printable_ascii) => out.push(printable_ascii),
            other => push_unicode_escape(&mut out, other),
        }
    }
    out.push('"');
    out
}

fn push_unicode_escape(out: &mut String, character: char) {
    let mut units = [0_u16; 2];
    for unit in character.encode_utf16(&mut units) {
        out.push_str("\\u");
        for byte in unit.to_be_bytes() {
            out.extend(char::from_digit(u32::from(byte) >> 4_u32, 16));
            out.extend(char::from_digit(u32::from(byte) & 0x0f_u32, 16));
        }
    }
}
