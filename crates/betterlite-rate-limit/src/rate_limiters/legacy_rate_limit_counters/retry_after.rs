const FIXED_MIN: f64 = 1e-4;
const FIXED_MAX: f64 = 1e16;

// str(float): the retry-after text LiteLLM builds from timedelta.total_seconds()
#[must_use]
pub(crate) fn float_text(value: f64) -> String {
    if value.is_nan() {
        return String::from("nan");
    }
    if value.is_infinite() {
        return if value.is_sign_positive() {
            String::from("inf")
        } else {
            String::from("-inf")
        };
    }
    // CPython writes fixed notation only inside FIXED_MIN..FIXED_MAX
    let magnitude = value.abs();
    if magnitude == 0.0_f64 || (FIXED_MIN..FIXED_MAX).contains(&magnitude) {
        let fixed = value.to_string();
        return if fixed.contains('.') {
            fixed
        } else {
            format!("{fixed}.0")
        };
    }
    let scientific = format!("{value:e}");
    let Some((mantissa, power)) = scientific.split_once('e') else {
        return scientific;
    };
    // CPython writes the exponent with a sign and at least two digits
    let (sign, digits) = power
        .strip_prefix('-')
        .map_or(('+', power), |digits| ('-', digits));
    format!("{mantissa}e{sign}{digits:0>2}")
}
