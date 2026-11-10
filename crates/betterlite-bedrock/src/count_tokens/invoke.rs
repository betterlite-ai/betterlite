use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use betterlite_logging::python_json::dumps_object;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde_json::Number;

use super::request::{CountTokensInput, CountTokensRequest, InvokeModelInput};

const ANTHROPIC_VERSION: &str = "bedrock-2023-05-31";
const DEFAULT_MAX_TOKENS: u16 = 1024;

#[must_use]
pub fn transform_invoke_model_request(request: &IndexMap<String, Json>) -> CountTokensRequest {
    let mut body = request.clone();
    body.shift_remove("model");
    if body.contains_key("messages") {
        body.entry("anthropic_version".to_owned())
            .or_insert_with(|| Json::Text(ANTHROPIC_VERSION.to_owned()));
        body.entry("max_tokens".to_owned())
            .or_insert_with(|| Json::Number(Number::from(DEFAULT_MAX_TOKENS)));
    }
    let encoded = STANDARD.encode(dumps_object(&body));
    CountTokensRequest {
        input: CountTokensInput::InvokeModel(InvokeModelInput { body: encoded }),
    }
}
