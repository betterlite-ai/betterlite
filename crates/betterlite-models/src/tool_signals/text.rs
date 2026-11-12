pub(crate) fn valid_underscored_digits(text: &str) -> bool {
    // PEP 515: a digit run may carry single underscores between two digits, never leading, trailing or doubled.
    let bytes = text.as_bytes();
    let (Some(&first), Some(&last)) = (bytes.first(), bytes.last()) else {
        return false;
    };
    if first == b'_' || last == b'_' {
        return false;
    }
    let mut previous_was_underscore = false;
    for &current in bytes {
        match current {
            b'_' if previous_was_underscore => return false,
            b'_' => previous_was_underscore = true,
            digit if digit.is_ascii_digit() => previous_was_underscore = false,
            _ => return false,
        }
    }
    !previous_was_underscore
}

pub(crate) fn digit_magnitude(text: &str) -> Option<i128> {
    text.chars()
        .filter(|character| *character != '_')
        .collect::<String>()
        .parse()
        .ok()
}

// The string grammar float() accepts: a sign, then inf/nan or a decimal mantissa with an optional exponent.
pub(crate) fn is_float_literal(text: &str) -> bool {
    let body = text.strip_prefix(['+', '-']).unwrap_or(text);
    if body.eq_ignore_ascii_case("inf")
        || body.eq_ignore_ascii_case("infinity")
        || body.eq_ignore_ascii_case("nan")
    {
        return true;
    }
    let (mantissa, exponent) = match body.split_once(['e', 'E']) {
        Some((mantissa, exponent)) => (mantissa, Some(exponent)),
        None => (body, None),
    };
    is_decimal_mantissa(mantissa)
        && exponent.is_none_or(|exponent_text| {
            let digits = exponent_text
                .strip_prefix(['+', '-'])
                .unwrap_or(exponent_text);
            valid_underscored_digits(digits)
        })
}

fn is_decimal_mantissa(text: &str) -> bool {
    let Some((int_part, frac_part)) = text.split_once('.') else {
        return valid_underscored_digits(text);
    };
    (!int_part.is_empty() || !frac_part.is_empty())
        && (int_part.is_empty() || valid_underscored_digits(int_part))
        && (frac_part.is_empty() || valid_underscored_digits(frac_part))
}

mod tests;
