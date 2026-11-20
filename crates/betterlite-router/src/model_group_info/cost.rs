use betterlite_core::api::request_form::scalars::scalar_float;
use betterlite_models::decisions::Json;
use betterlite_models::price_fields::decimal_lexeme::parse_decimal_lexeme;
use rust_decimal::Decimal;

// A cost value Python's float() keeps as an infinity or a decimal beyond Decimal's range cannot be a Decimal here.
#[derive(Debug, thiserror::Error)]
pub enum CostError {
    #[error("the cost value {value} is not a finite decimal")]
    NotRepresentable { value: String },
}

// _cost_value_as_float: null is absent, a bool is 0 or 1, text follows float(), and a list or object is silently absent.
pub fn cost_value(value: Option<&Json>) -> Result<Option<Decimal>, CostError> {
    match value {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Bool(flag)) => Ok(Some(if *flag { Decimal::ONE } else { Decimal::ZERO })),
        Some(Json::Number(number)) => decimal_from_lexeme(&number.to_string()).map(Some),
        Some(Json::Text(text)) => {
            let Some(parsed) = scalar_float(text) else {
                return Ok(None);
            };
            finite_decimal(parsed).map(Some)
        }
        Some(Json::List(_) | Json::Object(_)) => Ok(None),
    }
}

// float() accepted the text, so a decimal failure means the value has no Decimal form.
fn finite_decimal(value: f64) -> Result<Decimal, CostError> {
    Decimal::from_f64_retain(value).ok_or_else(|| CostError::NotRepresentable {
        value: value.to_string(),
    })
}

// The metadata and fallback costs that arrive as JSON numbers go through the same checked conversion.
pub fn decimal_from_lexeme(lexeme: &str) -> Result<Decimal, CostError> {
    parse_decimal_lexeme(lexeme).map_err(|_error| CostError::NotRepresentable {
        value: lexeme.to_owned(),
    })
}
