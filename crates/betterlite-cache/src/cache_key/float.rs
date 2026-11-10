pub(super) fn render_float(value: f64) -> String {
    if value == 0.0_f64 {
        return if value.is_sign_negative() {
            "-0.0".to_owned()
        } else {
            "0.0".to_owned()
        };
    }
    let negative = value.is_sign_negative();
    let formatted = format!("{:e}", value.abs());
    let Some((mantissa, exponent)) = formatted.split_once('e') else {
        return formatted;
    };
    let Ok(exponent_value) = exponent.parse::<i32>() else {
        return formatted;
    };
    let digits = mantissa
        .chars()
        .filter(char::is_ascii_digit)
        .collect::<String>();
    let point = exponent_value.saturating_add(1);
    // Python uses scientific notation only outside a decimal point of (-4, 16], and pads the exponent to two digits.
    let body = if point <= -4_i32 || point > 16_i32 {
        scientific(&digits, exponent_value)
    } else {
        fixed(&digits, point)
    };
    if negative { format!("-{body}") } else { body }
}

fn scientific(digits: &str, exponent: i32) -> String {
    let Some((first, rest)) = digits.split_at_checked(1) else {
        return digits.to_owned();
    };
    let mantissa = if rest.is_empty() {
        first.to_owned()
    } else {
        format!("{first}.{rest}")
    };
    let sign = if exponent.is_negative() { '-' } else { '+' };
    format!("{mantissa}e{sign}{:02}", exponent.unsigned_abs())
}

fn fixed(digits: &str, point: i32) -> String {
    let Ok(point_length) = usize::try_from(point) else {
        let zeros = usize::try_from(point.unsigned_abs()).unwrap_or_default();
        return format!("0.{}{digits}", "0".repeat(zeros));
    };
    let Some((whole, fraction)) = digits.split_at_checked(point_length) else {
        let zeros = point_length.saturating_sub(digits.len());
        return format!("{digits}{}.0", "0".repeat(zeros));
    };
    // A zero decimal point has no whole part and a point at the last digit has no fraction, and Python prints both as 0.
    let whole_part = if whole.is_empty() { "0" } else { whole };
    let fraction_part = if fraction.is_empty() { "0" } else { fraction };
    format!("{whole_part}.{fraction_part}")
}
