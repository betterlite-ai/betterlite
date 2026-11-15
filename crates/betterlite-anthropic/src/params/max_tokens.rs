use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use num_traits::cast::cast;

use crate::params::error::ParamsError;

// Both spellings overwrite max_tokens, so the last one in request order wins
pub fn apply(
    optional_params: &mut IndexMap<String, Json>,
    param: &'static str,
    value: &Json,
) -> Result<(), ParamsError> {
    let max_tokens = match value {
        Json::Bool(_) => value.clone(),
        Json::Number(number) if !number.to_string().contains(['.', 'e', 'E']) => value.clone(),
        Json::Number(number) => {
            let parsed = number
                .as_f64()
                .ok_or(ParamsError::OutOfRange { field: param })?;
            let tokens = cast::<f64, i64>(parsed.round_ties_even())
                .ok_or(ParamsError::OutOfRange { field: param })?;
            Json::Number(serde_json::Number::from(tokens.max(1)))
        }
        Json::Null | Json::Text(_) | Json::List(_) | Json::Object(_) => {
            return Err(ParamsError::NotNumber { field: param });
        }
    };
    optional_params.insert("max_tokens".to_owned(), max_tokens);
    Ok(())
}
