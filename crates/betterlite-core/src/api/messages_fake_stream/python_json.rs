use serde_json::Number;

use super::value::Json;

pub(super) fn json_dumps_defaults(value: &Json) -> String {
    let mut out = String::new();
    write_value(&mut out, value);
    out
}

fn write_value(out: &mut String, value: &Json) {
    match value {
        Json::Null => out.push_str("null"),
        Json::Bool(true) => out.push_str("true"),
        Json::Bool(false) => out.push_str("false"),
        Json::Number(number) => out.push_str(&dumps_number(number)),
        Json::String(text) => write_string(out, text),
        Json::Array(items) => write_array(out, items),
        Json::Object(fields) => write_object(out, fields),
    }
}

fn write_array(out: &mut String, items: &[Json]) {
    out.push('[');
    for (position, item) in items.iter().enumerate() {
        if position != 0 {
            out.push_str(", ");
        }
        write_value(out, item);
    }
    out.push(']');
}

fn write_object<'entry>(
    out: &mut String,
    fields: impl IntoIterator<Item = (&'entry String, &'entry Json)>,
) {
    out.push('{');
    for (position, (key, item)) in fields.into_iter().enumerate() {
        if position != 0 {
            out.push_str(", ");
        }
        write_string(out, key);
        out.push_str(": ");
        write_value(out, item);
    }
    out.push('}');
}

fn write_string(out: &mut String, text: &str) {
    out.push('"');
    for character in text.chars() {
        write_char(out, character);
    }
    out.push('"');
}

const SIMPLE_ESCAPES: [(char, &str); 7] = [
    ('"', "\\\""),
    ('\\', "\\\\"),
    ('\n', "\\n"),
    ('\r', "\\r"),
    ('\t', "\\t"),
    ('\u{8}', "\\b"),
    ('\u{c}', "\\f"),
];

fn write_char(out: &mut String, character: char) {
    if let Some((_, escaped)) = SIMPLE_ESCAPES.iter().find(|(from, _)| *from == character) {
        out.push_str(escaped);
        return;
    }
    if (' '..='~').contains(&character) {
        out.push(character);
        return;
    }
    let mut units = [0_u16; 2];
    for unit in character.encode_utf16(&mut units) {
        let code = *unit;
        out.push_str("\\u");
        out.push(hex_digit(code >> 12));
        out.push(hex_digit(code >> 8));
        out.push(hex_digit(code >> 4));
        out.push(hex_digit(code));
    }
}

const fn hex_digit(nibble: u16) -> char {
    match nibble & 0xf {
        0 => '0',
        1 => '1',
        2 => '2',
        3 => '3',
        4 => '4',
        5 => '5',
        6 => '6',
        7 => '7',
        8 => '8',
        9 => '9',
        10 => 'a',
        11 => 'b',
        12 => 'c',
        13 => 'd',
        14 => 'e',
        _ => 'f',
    }
}

// arbitrary_precision keeps the source digits, so only a float-shaped number needs reprinting.
fn dumps_number(number: &Number) -> String {
    let text = number.to_string();
    if !text.contains(['.', 'e', 'E']) {
        return if text == "-0" { "0".to_owned() } else { text };
    }
    text.parse::<f64>().map_or(text, dumps_float)
}

fn dumps_float(value: f64) -> String {
    if value.is_infinite() {
        let infinity = if value.is_sign_positive() {
            "Infinity"
        } else {
            "-Infinity"
        };
        return infinity.to_owned();
    }
    let shortest = format!("{value:?}");
    let Some((mantissa, exponent)) = shortest.split_once('e') else {
        return shortest;
    };
    let (sign, digits) = exponent
        .strip_prefix('-')
        .map_or(("+", exponent), |digits| ("-", digits));
    format!("{mantissa}e{sign}{digits:0>2}")
}
