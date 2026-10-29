use std::fmt::{self, Write as _};
use unicode_general_category::{GeneralCategory, get_general_category};

// Matches Python's repr() of a str, including str.isprintable()'s Unicode Other/Separator escaping.
pub(super) fn python_repr(text: &str) -> String {
    let quote = if text.contains('\'') && !text.contains('"') {
        '"'
    } else {
        '\''
    };
    let mut out = String::new();
    out.push(quote);
    for character in text.chars() {
        match character {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            _ if character == quote => {
                out.push('\\');
                out.push(quote);
            }
            // str.isprintable() rejects every Other and Separator category except the ASCII space.
            _ if character != ' '
                && matches!(
                    get_general_category(character),
                    GeneralCategory::Control
                        | GeneralCategory::Format
                        | GeneralCategory::PrivateUse
                        | GeneralCategory::Unassigned
                        | GeneralCategory::LineSeparator
                        | GeneralCategory::ParagraphSeparator
                        | GeneralCategory::SpaceSeparator
                ) =>
            {
                if write_hex_escape(character, &mut out).is_err() {
                    return out;
                }
            }
            _ => out.push(character),
        }
    }
    out.push(quote);
    out
}

fn write_hex_escape(character: char, out: &mut String) -> fmt::Result {
    let code = u32::from(character);
    match code {
        0..=0xff => write!(out, "\\x{code:02x}"),
        0x100..=0xffff => write!(out, "\\u{code:04x}"),
        _ => write!(out, "\\U{code:08x}"),
    }
}

pub(super) fn format_float(value: f64) -> String {
    // Scientific notation only outside (-4, 16] on the decimal point, matching CPython's format_float_short threshold.
    if value.is_nan() {
        return "nan".to_owned();
    }
    if value.is_infinite() {
        return if value.is_sign_negative() {
            "-inf".to_owned()
        } else {
            "inf".to_owned()
        };
    }
    if value == 0.0_f64 {
        return if value.is_sign_negative() {
            "-0.0".to_owned()
        } else {
            "0.0".to_owned()
        };
    }
    let negative = value.is_sign_negative();
    let Some((digits, exponent)) = shortest_digits(value.abs()) else {
        // Rust's {:e} formatter always emits a parseable sign+digit exponent for a finite f64; unreachable in practice.
        return format!("{value}");
    };
    let decimal_point = exponent.saturating_add(1);
    let body = if decimal_point <= -4_i32 || decimal_point > 16_i32 {
        scientific_body(&digits, exponent)
    } else {
        fixed_body(&digits, decimal_point)
    };
    if negative { format!("-{body}") } else { body }
}

// Rust's LowerExp already picks the shortest round-tripping digit string for f64.
fn shortest_digits(value: f64) -> Option<(String, i32)> {
    let formatted = format!("{value:e}");
    let (mantissa, exponent_text) = formatted.split_once('e')?;
    let exponent = exponent_text.parse::<i32>().ok()?;
    let digits = mantissa
        .chars()
        .filter(|character| *character != '.')
        .collect();
    Some((digits, exponent))
}

fn scientific_body(digits: &str, exponent: i32) -> String {
    let (first, rest) = digits.split_at(1);
    let mantissa = if rest.is_empty() {
        first.to_owned()
    } else {
        format!("{first}.{rest}")
    };
    let sign = if exponent < 0_i32 { '-' } else { '+' };
    format!("{mantissa}e{sign}{:02}", exponent.unsigned_abs())
}

fn fixed_body(digits: &str, decimal_point: i32) -> String {
    match usize::try_from(decimal_point) {
        Err(_) | Ok(0) => {
            let mut body = String::from("0.");
            for _ in 0..decimal_point.unsigned_abs() {
                body.push('0');
            }
            body.push_str(digits);
            body
        }
        Ok(point) if point >= digits.len() => {
            let mut body = digits.to_owned();
            for _ in digits.len()..point {
                body.push('0');
            }
            body.push_str(".0");
            body
        }
        Ok(point) => {
            let (integer_part, fractional_part) = digits.split_at(point);
            format!("{integer_part}.{fractional_part}")
        }
    }
}
