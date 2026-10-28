use betterlite_models::price_fields::{PriceFields, parse_decimal_lexeme};
use serde::Deserialize;
use serde_json::Value;

pub(crate) struct Op;

#[derive(Deserialize)]
#[serde(untagged)]
pub(crate) enum Input {
    Source { source: String },
    Direct(Box<PriceFields>),
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let from_source;
        let parsed = match input {
            Input::Source { source } => {
                from_source = serde_json::from_str::<PriceFields>(source).map_err(|error| {
                    Box::new(crate::helpers::HelperError {
                        class: "PriceFieldError".to_owned(),
                        status: None,
                        error_type: None,
                        param: None,
                        code: None,
                        message: error.to_string(),
                    })
                })?;
                &from_source
            }
            Input::Direct(prices) => prices.as_ref(),
        };
        let mut projected = serde_json::to_value(parsed).map_err(|error| {
            Box::new(crate::helpers::HelperError {
                class: "Serialize".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: error.to_string(),
            })
        })?;
        canonical(&mut projected);
        Ok(projected)
    }
}

fn canonical(value: &mut Value) {
    match value {
        Value::Object(fields) => {
            for (key, nested) in fields {
                if !matches!(key.as_str(), "windows" | "hours_utc" | "weekdays") {
                    canonical(nested);
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                canonical(item);
            }
        }
        Value::String(text) => {
            if let Ok(decimal) = parse_decimal_lexeme(text) {
                *text = decimal.normalize().to_string();
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}

mod tests;
