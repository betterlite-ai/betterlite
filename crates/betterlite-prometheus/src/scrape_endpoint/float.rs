pub(crate) fn float_to_go_string(value: f64) -> String {
    if value.is_infinite() {
        return if value.is_sign_positive() {
            "+Inf".to_owned()
        } else {
            "-Inf".to_owned()
        };
    }
    if value.is_nan() {
        return "NaN".to_owned();
    }
    let repr = float_repr(value);
    match repr.find('.') {
        Some(dot) if value.is_sign_positive() && dot > 6 => {
            let characters: Vec<char> = repr.chars().collect();
            let first = characters.first().copied().unwrap_or_default();
            let integer: String = characters.get(1..dot).unwrap_or_default().iter().collect();
            let fraction: String = characters
                .get(dot.saturating_add(1)..)
                .unwrap_or_default()
                .iter()
                .collect();
            let mantissa = format!("{first}.{integer}{fraction}");
            let trimmed = mantissa.trim_end_matches(['0', '.']);
            format!("{trimmed}e+0{}", dot.saturating_sub(1))
        }
        _ => repr,
    }
}

// Python's repr of a float, whose exponent is always signed and at least two digits wide; Rust's Debug differs only there
fn float_repr(value: f64) -> String {
    let debug = format!("{value:?}");
    let Some((mantissa, exponent)) = debug.split_once('e') else {
        return debug;
    };
    let (sign, digits) = exponent
        .strip_prefix('-')
        .map_or(('+', exponent), |rest| ('-', rest));
    let padded = if digits.len() < 2 {
        format!("0{digits}")
    } else {
        digits.to_owned()
    };
    format!("{mantissa}e{sign}{padded}")
}
