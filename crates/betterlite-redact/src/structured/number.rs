use serde_json::Number;

pub(super) fn text(number: &Number) -> String {
    if let Some(integer) = number.as_i64() {
        return integer.to_string();
    }
    if let Some(integer) = number.as_u64() {
        return integer.to_string();
    }
    // JSON integer digits are already canonical; retaining them avoids rounding past u64.
    let digits = number.as_str();
    if !digits.contains(['.', 'e', 'E']) {
        return digits.to_owned();
    }
    let Some(value) = number.as_f64() else {
        return if digits.starts_with('-') {
            "-inf"
        } else {
            "inf"
        }
        .to_owned();
    };
    let formatted = format!("{value:?}");
    // Python uses the same notation thresholds but requires a sign and two exponent digits.
    if let Some((coefficient, exponent)) = formatted.split_once('e') {
        let (sign, magnitude) = exponent
            .strip_prefix('-')
            .map_or(("+", exponent), |magnitude| ("-", magnitude));
        return format!("{coefficient}e{sign}{magnitude:0>2}");
    }
    formatted
}

mod tests;
