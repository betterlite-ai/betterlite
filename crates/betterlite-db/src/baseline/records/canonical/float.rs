pub(super) fn format(value: f64) -> String {
    // CPython's json float repr: shortest round-tripping digits, fixed notation only while the decimal point sits in (-3, 16], two exponent digits, and a ".0" tail on an integral fixed value.
    if value.is_nan() {
        return "NaN".to_owned();
    }
    if value.is_infinite() {
        return if value.is_sign_negative() {
            "-Infinity".to_owned()
        } else {
            "Infinity".to_owned()
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
    let (digits, point) = shortest(value.abs());
    let body = if point <= -4_i32 || point > 16_i32 {
        scientific(&digits, point.saturating_sub(1))
    } else {
        fixed(&digits, point)
    };
    if negative { format!("-{body}") } else { body }
}

// Rust's LowerExp already carries the shortest round-tripping digit string; point is one past its exponent.
fn shortest(value: f64) -> (String, i32) {
    let formatted = format!("{value:e}");
    let (mantissa, exponent_text) = formatted.split_once('e').unwrap_or_default();
    let exponent = exponent_text.parse::<i32>().unwrap_or_default();
    let digits = mantissa
        .chars()
        .filter(|character| *character != '.')
        .collect();
    (digits, exponent.saturating_add(1))
}

fn scientific(digits: &str, exponent: i32) -> String {
    let mut characters = digits.chars();
    let Some(first) = characters.next() else {
        return "0.0".to_owned();
    };
    let rest: String = characters.collect();
    let mut out = String::new();
    out.push(first);
    if !rest.is_empty() {
        out.push('.');
        out.push_str(&rest);
    }
    out.push('e');
    out.push(if exponent < 0_i32 { '-' } else { '+' });
    out.push_str(&format!("{:02}", exponent.unsigned_abs()));
    out
}

fn fixed(digits: &str, point: i32) -> String {
    match usize::try_from(point) {
        Err(_) => {
            let zeros = usize::try_from(point.unsigned_abs()).unwrap_or_default();
            format!("0.{}{digits}", "0".repeat(zeros))
        }
        Ok(0) => format!("0.{digits}"),
        Ok(point) if point >= digits.len() => {
            let mut body = digits.to_owned();
            body.push_str(&"0".repeat(point.saturating_sub(digits.len())));
            body.push_str(".0");
            body
        }
        Ok(point) => {
            let (whole, fraction) = digits.split_at(point);
            format!("{whole}.{fraction}")
        }
    }
}

mod tests;
