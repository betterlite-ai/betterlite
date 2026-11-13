use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::params::error::ParamsError;
use crate::params::value;

// Both spellings overwrite max_tokens, so the last one in request order wins
pub fn apply(
    optional_params: &mut IndexMap<String, Json>,
    param: &'static str,
    value: &Json,
) -> Result<(), ParamsError> {
    let max_tokens = match value {
        Json::Bool(_) => value.clone(),
        Json::Number(number) if value::is_integer_literal(number) => value.clone(),
        Json::Number(number) => {
            let tokens =
                value::rounded_tokens(number).ok_or(ParamsError::OutOfRange { field: param })?;
            Json::Number(serde_json::Number::from(tokens))
        }
        Json::Null | Json::Text(_) | Json::List(_) | Json::Object(_) => {
            return Err(ParamsError::NotNumber { field: param });
        }
    };
    optional_params.insert("max_tokens".to_owned(), max_tokens);
    Ok(())
}
