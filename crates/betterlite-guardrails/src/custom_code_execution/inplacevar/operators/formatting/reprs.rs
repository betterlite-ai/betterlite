use super::super::super::super::value::Value;
use indexmap::IndexMap;
pub(super) fn display(value: &Value, as_repr: bool) -> Option<String> {
    match value {
        Value::Str(text) if as_repr => Some(quoted(text)),
        Value::Str(text) => Some(text.clone()),
        Value::Bool(flag) => Some(if *flag { "True" } else { "False" }.to_owned()),
        Value::None => Some("None".to_owned()),
        Value::Int(number) => Some(number.to_string()),
        Value::Float(_) => None,
        Value::Bytes(bytes) => Some(bytes_repr(bytes)),
        Value::List(items) => Some(format!("[{}]", joined(items)?)),
        Value::Tuple(items) => Some(tuple_repr(items)?),
        Value::Dict(map) => Some(dict_repr(map)?),
    }
}
fn joined(items: &[Value]) -> Option<String> {
    let mut parts = Vec::new();
    for item in items {
        parts.push(display(item, true)?);
    }
    Some(parts.join(", "))
}
fn tuple_repr(items: &[Value]) -> Option<String> {
    let body = joined(items)?;
    if items.len() == 1 {
        return Some(format!("({body},)"));
    }
    Some(format!("({body})"))
}
fn dict_repr(map: &IndexMap<String, Value>) -> Option<String> {
    let mut parts = Vec::new();
    for (key, value) in map {
        parts.push(format!("{}: {}", quoted(key), display(value, true)?));
    }
    Some(format!("{{{}}}", parts.join(", ")))
}
fn quoted(text: &str) -> String {
    let escaped = text
        .replace('\\', "\\\\")
        .replace('\'', "\\'")
        .replace('\n', "\\n")
        .replace('\t', "\\t");
    format!("'{escaped}'")
}
fn bytes_repr(bytes: &[u8]) -> String {
    let mut body = String::new();
    for byte in bytes {
        match *byte {
            b'\n' => body.push_str("\\n"),
            b'\t' => body.push_str("\\t"),
            b'\\' => body.push_str("\\\\"),
            b'\'' => body.push_str("\\'"),
            0x20..=0x7e => body.push(char::from(*byte)),
            other => {
                body.push_str("\\x");
                body.push(
                    char::from_digit(u32::from(other.checked_shr(4).unwrap_or(0)), 16)
                        .unwrap_or('0'),
                );
                body.push(char::from_digit(u32::from(other & 0x0f), 16).unwrap_or('0'));
            }
        }
    }
    format!("b'{body}'")
}
