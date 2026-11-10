use betterlite_models::decisions::Json;
use serde_json::Number;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Style {
    Json,
    Python,
}

#[derive(Clone, Copy)]
enum Piece<'text> {
    Value(&'text Json),
    Key(&'text str),
    Literal(&'static str),
    Entry(&'text str, &'text Json),
}

// Python's str() of a value; a top-level string is itself, everything else takes repr text
#[must_use]
pub(crate) fn str_of(value: &Json) -> String {
    match value {
        Json::Text(text) => text.clone(),
        other
        @ (Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            render(other, Style::Python)
        }
    }
}

// json.dumps with its defaults: ", " and ": " separators, insertion order and ensure_ascii escapes
#[must_use]
pub(crate) fn render(value: &Json, style: Style) -> String {
    let mut out = String::new();
    let mut stack = vec![Piece::Value(value)];
    while let Some(piece) = stack.pop() {
        push_piece(&mut out, &mut stack, piece, style);
    }
    out
}

fn push_piece<'text>(
    out: &mut String,
    stack: &mut Vec<Piece<'text>>,
    piece: Piece<'text>,
    style: Style,
) {
    match piece {
        Piece::Value(Json::List(items)) => {
            open_container(out, stack, '[', "]");
            for (index, item) in items.iter().enumerate().rev() {
                stack.push(Piece::Value(item));
                push_separator(stack, index);
            }
        }
        Piece::Value(Json::Object(fields)) => {
            open_container(out, stack, '{', "}");
            for (index, (key, value)) in fields.iter().enumerate().rev() {
                stack.push(Piece::Entry(key.as_str(), value));
                push_separator(stack, index);
            }
        }
        Piece::Entry(key, value) => {
            stack.push(Piece::Value(value));
            stack.push(Piece::Literal(": "));
            stack.push(Piece::Key(key));
        }
        Piece::Literal(text) => out.push_str(text),
        Piece::Key(key) => push_quoted(out, key, style),
        Piece::Value(Json::Null) => out.push_str(null_text(style)),
        Piece::Value(Json::Bool(value)) => out.push_str(bool_text(*value, style)),
        Piece::Value(Json::Number(number)) => out.push_str(&number_text(number, style)),
        Piece::Value(Json::Text(text)) => push_quoted(out, text, style),
    }
}

fn open_container(out: &mut String, stack: &mut Vec<Piece<'_>>, open: char, close: &'static str) {
    out.push(open);
    stack.push(Piece::Literal(close));
}

fn push_separator(stack: &mut Vec<Piece<'_>>, index: usize) {
    if index != 0 {
        stack.push(Piece::Literal(", "));
    }
}

const fn null_text(style: Style) -> &'static str {
    match style {
        Style::Json => "null",
        Style::Python => "None",
    }
}

const fn bool_text(value: bool, style: Style) -> &'static str {
    match (style, value) {
        (Style::Json, true) => "true",
        (Style::Json, false) => "false",
        (Style::Python, true) => "True",
        (Style::Python, false) => "False",
    }
}

fn number_text(number: &Number, style: Style) -> String {
    let text = number.to_string();
    if !text.contains(['.', 'e', 'E']) {
        return if text == "-0" { "0".to_owned() } else { text };
    }
    let Ok(value) = text.parse::<f64>() else {
        return text;
    };
    if value.is_nan() {
        return non_finite_text("NaN", "nan", style);
    }
    if value.is_infinite() {
        return if value.is_sign_negative() {
            non_finite_text("-Infinity", "-inf", style)
        } else {
            non_finite_text("Infinity", "inf", style)
        };
    }
    shortest_float(value)
}

fn non_finite_text(json: &str, python: &str, style: Style) -> String {
    match style {
        Style::Json => json,
        Style::Python => python,
    }
    .to_owned()
}

// Rust's shortest float text switches to an exponent where Python's repr does, without its sign and padding
fn shortest_float(value: f64) -> String {
    let text = format!("{value:?}");
    match text.split_once('e') {
        None => text,
        Some((mantissa, power)) => power.strip_prefix('-').map_or_else(
            || format!("{mantissa}e+{power:0>2}"),
            |digits| format!("{mantissa}e-{digits:0>2}"),
        ),
    }
}

fn push_quoted(out: &mut String, text: &str, style: Style) {
    let quote = match style {
        Style::Json => '"',
        Style::Python if text.contains('\'') && !text.contains('"') => '"',
        Style::Python => '\'',
    };
    out.push(quote);
    for character in text.chars() {
        push_escaped(out, character, quote, style);
    }
    out.push(quote);
}

fn push_escaped(out: &mut String, character: char, quote: char, style: Style) {
    if character == quote || character == '\\' {
        out.push('\\');
        out.push(character);
        return;
    }
    push_character(out, character, style);
}

fn push_character(out: &mut String, character: char, style: Style) {
    let code = u32::from(character);
    match (style, character) {
        (Style::Json, '\u{8}') => out.push_str("\\b"),
        (Style::Json, '\u{c}') => out.push_str("\\f"),
        (Style::Json | Style::Python, '\n') => out.push_str("\\n"),
        (Style::Json | Style::Python, '\r') => out.push_str("\\r"),
        (Style::Json | Style::Python, '\t') => out.push_str("\\t"),
        (Style::Json, ' '..='~') => out.push(character),
        (Style::Python, printable) if is_printable(printable) => out.push(printable),
        (Style::Json, _) => push_unicode_escape(out, character),
        (Style::Python, _) if code <= 0xFF => push_hex_escape(out, 'x', code, 2),
        (Style::Python, _) if code <= 0xFFFF => push_hex_escape(out, 'u', code, 4),
        (Style::Python, _) => push_hex_escape(out, 'U', code, 8),
    }
}

fn push_unicode_escape(out: &mut String, character: char) {
    let mut units = [0_u16; 2];
    for unit in character.encode_utf16(&mut units) {
        push_hex_escape(out, 'u', u32::from(*unit), 4);
    }
}

fn push_hex_escape(out: &mut String, marker: char, value: u32, digits: u32) {
    out.push('\\');
    out.push(marker);
    for position in (0..digits).rev() {
        let shift = position.saturating_mul(4);
        let digit = value.checked_shr(shift).map_or(0, |shifted| shifted & 0xF);
        if let Some(character) = char::from_digit(digit, 16) {
            out.push(character);
        }
    }
}

// Python prints anything outside its Other and Separator categories, with ASCII space the one printable separator
const fn is_printable(character: char) -> bool {
    if character == ' ' {
        return true;
    }
    if character.is_control() || character.is_whitespace() {
        return false;
    }
    !matches!(
        character,
        '\u{ad}'
            | '\u{200b}'..='\u{200f}'
            | '\u{202a}'..='\u{202e}'
            | '\u{2060}'..='\u{206f}'
            | '\u{feff}'
            | '\u{fff9}'..='\u{fffb}'
            | '\u{e000}'..='\u{f8ff}'
            | '\u{f0000}'..='\u{ffffd}'
            | '\u{100000}'..='\u{10fffd}'
    )
}
