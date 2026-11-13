use betterlite_prompts::arguments::json::Object;
use serde::{Deserialize, Serialize};

use super::presence::Given;

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ResponseKind {
    Text,
    JsonObject,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum JsonSchemaKind {
    JsonSchema,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ResponseFormat {
    #[serde(rename = "type", default, skip_serializing_if = "Given::is_missing")]
    pub type_: Given<ResponseKind>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct JSONResponseSchema {
    #[serde(default, skip_serializing_if = "Given::is_missing")]
    pub description: Given<String>,
    pub name: String,
    #[serde(rename = "schema", default, skip_serializing_if = "Given::is_missing")]
    pub schema: Given<Object>,
    #[serde(default, skip_serializing_if = "Given::is_missing")]
    pub strict: Given<bool>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ResponseFormatJSONSchema {
    #[serde(rename = "type", default, skip_serializing_if = "Given::is_missing")]
    pub type_: Given<JsonSchemaKind>,
    pub json_schema: JSONResponseSchema,
}

// The pair Template.response_format accepts, in the order pydantic tries them.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum ResponseFormatChoice {
    Plain(ResponseFormat),
    JsonSchema(ResponseFormatJSONSchema),
}
