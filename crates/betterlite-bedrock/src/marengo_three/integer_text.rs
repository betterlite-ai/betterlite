use serde_json::Number;

// pydantic caps integer text at 4300 characters and counts the minus sign
const MAX_POSITIVE_DIGITS: usize = 4300;
const MAX_NEGATIVE_DIGITS: usize = 4299;

fn grouped_digits(text: &str, zero_fraction: bool) -> Option<String> {
    let whole = match text.split_once('.') {
        None => text,
        Some((whole, zeros))
            if zero_fraction && !zeros.is_empty() && zeros.bytes().all(|byte| byte == b'0') =>
        {
            whole
        }
        Some(_) => return None,
    };
    let mut digits = String::with_capacity(whole.len());
    let mut after_digit = false;
    for character in whole.chars() {
        if character == '_' && after_digit {
            after_digit = false;
        } else if character.is_ascii_digit() {
            digits.push(character);
            after_digit = true;
        } else {
            return None;
        }
    }
    after_digit.then_some(digits)
}

// pydantic skips a leading run of zeros and underscores, then takes one minus sign: "0-1" is -1
fn signed_digits(text: &str, zero_fraction: bool) -> Option<(bool, String)> {
    let mut negative = text.starts_with('-');
    let unsigned = text.strip_prefix(['+', '-']).unwrap_or(text);
    if !unsigned.starts_with('0') {
        return grouped_digits(unsigned, zero_fraction).map(|digits| (negative, digits));
    }
    let rest = unsigned.trim_start_matches(['0', '_']);
    let run = unsigned.strip_suffix(rest).unwrap_or_default();
    let digits = match rest.strip_prefix('-') {
        Some(after_sign) if !negative => {
            negative = true;
            let unpadded = after_sign.strip_prefix('_').unwrap_or(after_sign);
            let whole = unpadded.split('.').next().unwrap_or_default();
            if whole.starts_with('0') && whole != "0" {
                return None;
            }
            grouped_digits(unpadded, zero_fraction)?
        }
        _ if rest.is_empty() || rest.starts_with('.') => {
            if run.ends_with('_') {
                return None;
            }
            grouped_digits(&format!("0{rest}"), zero_fraction)?
        }
        _ => grouped_digits(rest, zero_fraction)?,
    };
    Some((negative, digits))
}

pub(super) fn whole_digits(text: &str, zero_fraction: bool) -> Option<Number> {
    let (negative, digits) = signed_digits(text, zero_fraction)?;
    let significant = digits.trim_start_matches('0');
    let allowed = if negative {
        MAX_NEGATIVE_DIGITS
    } else {
        MAX_POSITIVE_DIGITS
    };
    if significant.len() > allowed {
        return None;
    }
    if significant.is_empty() {
        return Some(Number::from(0_u8));
    }
    let sign = if negative { "-" } else { "" };
    format!("{sign}{significant}").parse::<Number>().ok()
}
