use betterlite_prompts::arguments::json::{Json, Object};
use serde::Serialize;

use super::error::BytezConfigError;
use super::messages::{NormalizedMessage, RawMessage, normalize_messages};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BytezRequestBody {
    pub messages: Vec<NormalizedMessage>,
    pub stream: Json,
    pub params: Object,
}

pub fn request_body(
    messages: &[RawMessage],
    optional_params: &Object,
) -> Result<BytezRequestBody, BytezConfigError> {
    let stream = optional_params
        .get("stream")
        .cloned()
        .unwrap_or(Json::Bool(false));
    let mut params = optional_params.clone();
    // a truthy stream moves to the body's own field; false and null stay in params as well
    if stream.is_truthy() {
        params.shift_remove("stream");
    }
    Ok(BytezRequestBody {
        messages: normalize_messages(messages)?,
        stream,
        params,
    })
}
