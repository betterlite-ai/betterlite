#[must_use]
pub fn quote_promql_string_literal(value: &str) -> String {
    let mut quoted = String::from('"');
    for character in value.chars() {
        match character {
            '"' => quoted.push_str("\\\""),
            '\\' => quoted.push_str("\\\\"),
            '\n' => quoted.push_str("\\n"),
            '\r' => quoted.push_str("\\r"),
            '\t' => quoted.push_str("\\t"),
            '\u{8}' => quoted.push_str("\\b"),
            '\u{c}' => quoted.push_str("\\f"),
            other => match u8::try_from(other) {
                Ok(byte) if byte < 0x20 => {
                    quoted.push_str("\\u00");
                    quoted.push_str(&hex::encode([byte]));
                }
                _ => quoted.push(other),
            },
        }
    }
    quoted.push('"');
    quoted
}

mod tests;
