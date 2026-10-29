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
    let (digits, exponent) = shortest_digits(value.abs());
    let decimal_point = exponent.saturating_add(1);
    let body = if decimal_point <= -4_i32 || decimal_point > 16_i32 {
        scientific_body(&digits, exponent)
    } else {
        fixed_body(&digits, decimal_point)
    };
    if negative { format!("-{body}") } else { body }
}

// Rust's LowerExp already picks the shortest round-tripping digit string for f64.
fn shortest_digits(value: f64) -> (String, i32) {
    let formatted = format!("{value:e}");
    let (mantissa, exponent_text) = formatted
        .split_once('e')
        .unwrap_or((formatted.as_str(), "0"));
    let exponent = exponent_text.parse::<i32>().unwrap_or(0_i32);
    let digits = mantissa
        .chars()
        .filter(|character| *character != '.')
        .collect();
    (digits, exponent)
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
    let digit_count = i32::try_from(digits.len()).unwrap_or(i32::MAX);
    if decimal_point <= 0 {
        let leading_zeros = 0_i32.saturating_sub(decimal_point);
        let zeros = "0".repeat(usize::try_from(leading_zeros).unwrap_or(0));
        format!("0.{zeros}{digits}")
    } else if decimal_point >= digit_count {
        let trailing_zeros = decimal_point.saturating_sub(digit_count);
        let zeros = "0".repeat(usize::try_from(trailing_zeros).unwrap_or(0));
        format!("{digits}{zeros}.0")
    } else {
        let split_at = usize::try_from(decimal_point).unwrap_or(0);
        let (integer_part, fractional_part) = digits.split_at(split_at);
        format!("{integer_part}.{fractional_part}")
    }
}
