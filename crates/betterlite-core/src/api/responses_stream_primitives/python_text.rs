use betterlite_logging::analytics::python::repr;
use betterlite_models::decisions::Json;

// Python's str() switches to exponent notation outside this magnitude range
const FIXED_MIN: f64 = 1e-4;
const FIXED_MAX: f64 = 1e16;

#[must_use]
pub fn python_float_text(value: f64) -> String {
    if value.is_nan() {
        return "nan".to_owned();
    }
    if value.is_infinite() {
        return if value.is_sign_positive() {
            "inf".to_owned()
        } else {
            "-inf".to_owned()
        };
    }
    let magnitude = value.abs();
    if magnitude == 0.0 || (FIXED_MIN..FIXED_MAX).contains(&magnitude) {
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

#[must_use]
pub fn python_value_text(value: &Json) -> String {
    match value {
        Json::Text(text) => text.clone(),
        Json::Null => "None".to_owned(),
        Json::Bool(true) => "True".to_owned(),
        Json::Bool(false) => "False".to_owned(),
        Json::Number(number) => python_number_text(number),
        Json::List(_) | Json::Object(_) => {
            // the formatter writes into a Vec, so its io error branch is unreachable
            repr(value).unwrap_or_default()
        }
    }
}

fn python_number_text(number: &serde_json::Number) -> String {
    let text = number.to_string();
    if text.contains(['.', 'e', 'E']) {
        return number.as_f64().map_or(text, python_float_text);
    }
    if text == "-0" {
        return "0".to_owned();
    }
    text
}
