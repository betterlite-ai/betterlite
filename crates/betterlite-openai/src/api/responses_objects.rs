use crate::api::responses_shapes::wire::decode_responses_shapes;
use crate::api::responses_shapes::{
    CustomToolCallOutputItem, GenericResponseOutputItem, OutputCodeInterpreterCall,
    OutputFunctionToolCall, OutputImageGenerationCall,
};
use betterlite_models::decisions::Json;
use rust_decimal::Decimal;
use serde::de::{DeserializeOwned, IgnoredAny};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub mod request_params;
mod response_wire;
pub mod sdk;
mod tests;
mod usage_wire;
pub mod wire;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ResponseOutputItem {
    Sdk(Box<sdk::unions::SdkOutput>),
    #[serde(skip_deserializing)]
    Message(GenericResponseOutputItem),
    #[serde(skip_deserializing)]
    FunctionCall(OutputFunctionToolCall),
    #[serde(skip_deserializing)]
    ImageGeneration(OutputImageGenerationCall),
    #[serde(skip_deserializing)]
    CodeInterpreter(OutputCodeInterpreterCall),
    #[serde(skip_deserializing)]
    CustomTool(CustomToolCallOutputItem),
    Unknown(BTreeMap<String, Json>),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutputTokensDetails {
    pub audio_tokens: Option<i64>,
    pub reasoning_tokens: Option<i64>,
    pub text_tokens: Option<i64>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Json>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InputTokensDetails {
    pub audio_tokens: Option<i64>,
    #[serde(
        default,
        deserialize_with = "serde_with::As::<serde_with::DefaultOnNull<serde_with::Same>>::deserialize"
    )]
    pub cached_tokens: i64,
    pub cached_tokens_details: Option<sdk::response::CachedTokensDetails>,
    pub image_tokens: Option<i64>,
    pub text_tokens: Option<i64>,
    pub video_tokens: Option<i64>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Json>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResponseApiUsage {
    pub input_tokens: i64,
    pub input_tokens_details: Option<InputTokensDetails>,
    pub output_tokens: i64,
    pub output_tokens_details: Option<OutputTokensDetails>,
    pub total_tokens: i64,
    #[serde(
        default,
        serialize_with = "rust_decimal::serde::arbitrary_precision_option::serialize",
        deserialize_with = "serde_with::As::<serde_with::TryFromInto<usage_wire::CostInput>>::deserialize"
    )]
    pub cost: Option<Decimal>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Json>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WebSearchToolUsage {
    pub num_requests: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResponsesToolUsage {
    pub web_search: Option<WebSearchToolUsage>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResponsesApiTruncation {
    Auto,
    Disabled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResponsesApiStatus {
    Completed,
    Failed,
    InProgress,
    Cancelled,
    Queued,
    Incomplete,
}

#[serde_with::serde_as]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[expect(
    clippy::partial_pub_fields,
    reason = "the private SDK attribute must remain hidden from response consumers"
)]
pub struct ResponsesApiResponse {
    pub id: String,
    pub created_at: i64,
    pub error: Option<BTreeMap<String, Json>>,
    pub incomplete_details: Option<sdk::response::IncompleteDetails>,
    pub instructions: Option<String>,
    pub metadata: Option<BTreeMap<String, Json>>,
    pub model: Option<String>,
    pub object: Option<String>,
    #[serde_as(serialize_as = "serde_with::FromInto<response_wire::OutputDump>")]
    pub output: Vec<ResponseOutputItem>,
    pub parallel_tool_calls: Option<bool>,
    #[serde(default, with = "rust_decimal::serde::arbitrary_precision_option")]
    pub temperature: Option<Decimal>,
    pub tool_choice: Option<sdk::response::ToolChoice>,
    pub tools: Option<sdk::response::ResponseTools>,
    #[serde(default, with = "rust_decimal::serde::arbitrary_precision_option")]
    pub top_p: Option<Decimal>,
    pub max_output_tokens: Option<i64>,
    pub previous_response_id: Option<String>,
    pub reasoning: Option<BTreeMap<String, Json>>,
    pub status: Option<String>,
    pub text: Option<BTreeMap<String, Json>>,
    pub truncation: Option<ResponsesApiTruncation>,
    pub usage: Option<ResponseApiUsage>,
    pub user: Option<String>,
    pub store: Option<bool>,
    #[serde(default, skip_serializing)]
    _hidden_params: IgnoredAny,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Json>,
}

pub fn decode_responses_objects<T>(
    bytes: &[u8],
    limits: wire::WireDecodeLimits,
) -> Result<T, wire::ResponsesObjectsDecodeError>
where
    T: DeserializeOwned,
{
    let decoded = decode_responses_shapes(bytes, limits);
    match decoded {
        Ok(response) => Ok(response),
        Err(source) => Err(wire::ResponsesObjectsDecodeError::ResponsesShapes(source)),
    }
}

impl ResponsesApiResponse {
    #[must_use]
    pub fn output_text(&self) -> String {
        self.output
            .iter()
            .filter_map(|item| match item {
                ResponseOutputItem::Sdk(output) => Some(output.output_text()),
                ResponseOutputItem::Message(message) => Some(
                    message
                        .content
                        .iter()
                        .filter(|part| {
                            message.r#type == "message"
                                && part.r#type.as_deref() == Some("output_text")
                        })
                        .map(|part| part.text.as_deref().unwrap_or_default())
                        .collect::<String>(),
                ),
                ResponseOutputItem::Unknown(fields) => unknown_output_text(fields),
                ResponseOutputItem::FunctionCall(_)
                | ResponseOutputItem::ImageGeneration(_)
                | ResponseOutputItem::CodeInterpreter(_)
                | ResponseOutputItem::CustomTool(_) => None,
            })
            .collect()
    }
}

fn unknown_output_text(fields: &BTreeMap<String, Json>) -> Option<String> {
    if !matches!(fields.get("type"), Some(Json::Text(item_type)) if item_type == "message") {
        return None;
    }
    let Some(Json::List(content)) = fields.get("content") else {
        return Some(String::new());
    };
    Some(
        content
            .iter()
            .filter_map(|part| {
                let Json::Object(part_fields) = part else {
                    return None;
                };
                let output_text = matches!(part_fields.get("type"), Some(Json::Text(part_type)) if part_type == "output_text");
                if !output_text {
                    return None;
                }
                match part_fields.get("text") {
                    Some(Json::Text(text)) => Some(text.as_str()),
                    _ => Some(""),
                }
            })
            .collect(),
    )
}
