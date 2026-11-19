use indexmap::IndexMap;
use serde_json::Number;

use super::annotations::NumericType;
use super::scalars::{scalar_float, scalar_int};
use super::values::{FormFields, FormValue, NonFinite};

// _numeric_form_value: a non-string stays as it is, and a text that the scalar rejects stays a string for the provider to reject.
fn numeric_value(text: &str, scalar: NumericType) -> Option<FormValue> {
    match scalar {
        NumericType::Int => scalar_int(text).map(FormValue::Number),
        NumericType::Float => scalar_float(text).map(float_value),
    }
}

// The typed form of float(): JSON has no non-finite numbers, so those become explicit variants until a consumer picks its wire format.
#[must_use]
pub fn float_value(value: f64) -> FormValue {
    if let Some(number) = Number::from_f64(value) {
        return FormValue::Number(number);
    }
    if value.is_nan() {
        return FormValue::NonFinite(NonFinite::Nan);
    }
    if value.is_sign_positive() {
        return FormValue::NonFinite(NonFinite::PositiveInfinity);
    }
    FormValue::NonFinite(NonFinite::NegativeInfinity)
}

#[must_use]
pub fn coerce_numeric_form_fields(
    fields: &FormFields,
    numeric: &IndexMap<String, NumericType>,
) -> FormFields {
    fields
        .iter()
        .map(|(name, value)| {
            let coerced = match (numeric.get(name), value) {
                (Some(scalar), FormValue::Text(text)) => {
                    numeric_value(text, *scalar).unwrap_or_else(|| value.clone())
                }
                _other => value.clone(),
            };
            (name.clone(), coerced)
        })
        .collect()
}
