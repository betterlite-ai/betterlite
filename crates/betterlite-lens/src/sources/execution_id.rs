use std::fmt::Write as _;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE;

// json.dumps escapes everything outside the 0x20-0x7e range as \uXXXX by default (ensure_ascii=True).
fn python_json_string(value: &str) -> String {
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
            other => {
                let mut units = [0_u16; 2];
                for unit in other.encode_utf16(&mut units) {
                    let _ignored = write!(out, "\\u{unit:04x}");
                }
            }
        }
    }
    out.push('"');
    out
}

// litellm's opaque ID only matches byte-for-byte with json.dumps' ", " item separator.
#[must_use]
pub fn execution_id(source: &str, team_id: &str, trace_id: &str, trace_ref: &str) -> String {
    let elements = [source, team_id, trace_id, trace_ref]
        .into_iter()
        .map(python_json_string)
        .collect::<Vec<String>>()
        .join(", ");
    URL_SAFE.encode(format!("[{elements}]"))
}

mod tests;
