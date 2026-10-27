use serde_json::Number;

pub(super) fn text(number: &Number) -> String {
    let formatted = number.to_string();
    // Python switches below 1e-4 and pads exponents to at least two digits.
    if let Some((coefficient, exponent)) = formatted.split_once("e-") {
        return format!("{coefficient}e-{exponent:0>2}");
    }
    if let Some(fraction) = formatted.trim_start_matches('-').strip_prefix("0.") {
        let significant = fraction.trim_start_matches('0');
        let zeros = fraction.len().saturating_sub(significant.len());
        if zeros >= 4
            && let Some((first, rest)) = significant.split_at_checked(1)
        {
            let sign = if formatted.starts_with('-') { "-" } else { "" };
            let tail = if rest.is_empty() {
                String::new()
            } else {
                format!(".{rest}")
            };
            return format!("{sign}{first}{tail}e-{:02}", zeros.saturating_add(1));
        }
    }
    formatted
}

mod tests;
