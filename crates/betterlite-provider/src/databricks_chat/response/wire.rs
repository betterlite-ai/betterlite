use betterlite_prompts::arguments::json::Object;
use serde::Deserialize;
use serde::de::IgnoredAny;

use crate::databricks_chat::content::DatabricksContent;
use crate::databricks_chat::error::DatabricksChatError;
use crate::databricks_chat::function::DatabricksFunction;
use crate::databricks_chat::keyed::Keyed;
use crate::databricks_chat::response::ResponseInput;

#[derive(Debug, Deserialize)]
pub(super) struct DatabricksResponse {
    pub id: String,
    pub created: i64,
    #[serde(default)]
    pub model: Option<String>,
    pub choices: Vec<DatabricksChoice>,
    // Usage(**block) forwards every key it does not name, so the block stays open here
    pub usage: Object,
}

#[derive(Debug, Deserialize)]
pub(super) struct DatabricksChoice {
    #[serde(default)]
    pub index: Keyed<i64>,
    pub message: DatabricksMessage,
    #[serde(default)]
    pub finish_reason: Keyed<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct DatabricksMessage {
    #[serde(default)]
    pub content: Keyed<DatabricksContent>,
    #[serde(default)]
    pub tool_calls: Option<Vec<DatabricksToolCall>>,
    #[serde(default)]
    pub reasoning_content: Keyed<String>,
    #[serde(default)]
    pub reasoning: Keyed<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct DatabricksToolCall {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(rename = "type", default)]
    pub kind: Option<String>,
    pub function: DatabricksFunction,
    #[serde(default)]
    pub custom: Option<IgnoredAny>,
    // ChatCompletionMessageToolCall keeps the call keys it does not name, such as index
    #[serde(flatten)]
    pub extra: Object,
}

// DatabricksResponse(**raw_response.json()): only a body that is not a JSON object keeps the response status
pub(super) fn parse_body(
    input: &ResponseInput<'_>,
) -> Result<DatabricksResponse, DatabricksChatError> {
    let typed_error = match serde_json::from_str(input.body) {
        Ok(body) => return Ok(body),
        Err(source) => source,
    };
    match serde_json::from_str::<Object>(input.body) {
        Ok(_) => Err(DatabricksChatError::Decode(typed_error)),
        Err(error) => Err(DatabricksChatError::Response {
            status_code: input.status_code,
            message: format!(
                "Unable to get json response - {error}, Original Response: {}",
                input.body
            ),
            headers: input.headers.clone(),
        }),
    }
}
