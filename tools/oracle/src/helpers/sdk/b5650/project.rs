use betterlite_models::decisions::Json;
use serde_json::Value;

use crate::helpers::{HelperError, OrHelperError as _};

// A number becomes text, since 3e-07 and 3e-7 are one float with two spellings.
pub(super) fn project(value: &Json) -> Result<Value, Box<HelperError>> {
    let mut shown = serde_json::to_value(value).or_value_error()?;
    spell_numbers(&mut shown);
    Ok(shown)
}

fn spell_numbers(value: &mut Value) {
    match value {
        Value::Number(found) => {
            let text = found.to_string();
            *value = Value::String(if text.contains(['.', 'e', 'E']) {
                let bits = found.as_f64().map_or(0, f64::to_bits);
                format!("f:{bits:016x}")
            } else {
                format!("i:{text}")
            });
        }
        Value::Array(items) => items.iter_mut().for_each(spell_numbers),
        Value::Object(fields) => fields.values_mut().for_each(spell_numbers),
        Value::Null | Value::Bool(_) | Value::String(_) => {}
    }
}
