use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::Serialize;
use serde_json::Number;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CountTokensResult {
    pub input_tokens: Json,
}

#[must_use]
pub fn transform_response(response: &IndexMap<String, Json>) -> CountTokensResult {
    CountTokensResult {
        input_tokens: response
            .get("inputTokens")
            .cloned()
            .unwrap_or(Json::Number(Number::from(0_u8))),
    }
}
