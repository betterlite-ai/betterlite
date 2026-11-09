use std::str::FromStr as _;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde_json::Number;

use crate::fields::payload::StandardLoggingPayload;

#[derive(Debug, thiserror::Error)]
pub enum OutputError {
    #[error("the response cost could not be written as a JSON number")]
    Cost(#[source] serde_json::Error),
}

// LangsmithLogger._build_outputs_with_usage; a non-dict response becomes the output field, and usage always rides along.
pub fn build_outputs_with_usage(
    payload: &StandardLoggingPayload,
) -> Result<IndexMap<String, Json>, OutputError> {
    let mut outputs = match payload.response.as_ref() {
        Some(Json::Object(fields)) => fields.clone(),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_))
        | None => IndexMap::from([(
            "output".to_owned(),
            payload.response.clone().unwrap_or(Json::Null),
        )]),
    };
    let cost = Number::from_str(&payload.response_cost.normalize().to_string())
        .map_err(OutputError::Cost)?;
    outputs.insert(
        "usage_metadata".to_owned(),
        Json::Object(IndexMap::from([
            (
                "input_tokens".to_owned(),
                Json::Number(Number::from(payload.prompt_tokens)),
            ),
            (
                "output_tokens".to_owned(),
                Json::Number(Number::from(payload.completion_tokens)),
            ),
            (
                "total_tokens".to_owned(),
                Json::Number(Number::from(payload.total_tokens)),
            ),
            ("total_cost".to_owned(), Json::Number(cost)),
        ])),
    );
    Ok(outputs)
}
