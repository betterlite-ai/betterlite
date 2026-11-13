use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use serde::{Deserialize, Serialize};

use crate::generic_guardrail_contracts::GuardrailToolParam;

// LiteLLM's _TextContentPart, whose `type` literal is the discriminator and which forbids extra keys
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TextContentPart {
    #[serde(rename = "type")]
    pub part_type: TextPartType,
    pub text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TextPartType {
    #[serde(rename = "text")]
    Text,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImageUrl {
    pub url: String,
}

// LiteLLM's _ImageUrlContentPart, with the same forbid-extra rule
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImageUrlContentPart {
    #[serde(rename = "type")]
    pub part_type: ImageUrlPartType,
    pub image_url: ImageUrl,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ImageUrlPartType {
    #[serde(rename = "image_url")]
    ImageUrl,
}

// LiteLLM's _ContentPart union, discriminated by the `type` key
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ContentPart {
    Text(TextContentPart),
    ImageUrl(ImageUrlContentPart),
}

// LiteLLM's `str | list[_ContentPart] | None`
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MessageContent {
    Text(String),
    Parts(Vec<ContentPart>),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    #[serde(default)]
    pub content: Option<MessageContent>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GuardInput {
    pub messages: Vec<Message>,
    #[serde(default)]
    pub tools: Option<Vec<GuardrailToolParam>>,
}

// LiteLLM's _GuardChatCompletionsResult: every field defaults to None
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct GuardResult {
    #[serde(default)]
    pub guard_output: Option<GuardInput>,
    #[serde(default)]
    pub blocked: Option<bool>,
    #[serde(default)]
    pub transformed: Option<bool>,
    #[serde(default)]
    pub detectors: Option<BTreeMap<String, GuardrailValue>>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct GuardResponse {
    #[serde(default)]
    pub result: Option<GuardResult>,
}

// The two signal fields read before the full response is validated
#[derive(Debug, Clone, Default, Deserialize)]
pub struct GuardSignals {
    #[serde(default)]
    pub result: Option<GuardSignalsResult>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct GuardSignalsResult {
    #[serde(default)]
    pub blocked: Option<bool>,
    #[serde(default)]
    pub transformed: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum EventType {
    Input,
    Output,
}

// The body of POST {api_base}/v1/guard_chat_completions
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct GuardPayload {
    pub guard_input: GuardInput,
    pub event_type: EventType,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_id: Option<GuardrailValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extra_info: Option<BTreeMap<String, GuardrailValue>>,
}

// A redacted part goes back into a message as the dict LiteLLM's model_dump(mode="json") writes
#[must_use]
pub fn content_part_value(part: &ContentPart) -> GuardrailValue {
    match part {
        ContentPart::Text(text) => GuardrailValue::Object(BTreeMap::from([
            ("type".to_owned(), GuardrailValue::Text("text".to_owned())),
            ("text".to_owned(), GuardrailValue::Text(text.text.clone())),
        ])),
        ContentPart::ImageUrl(image) => GuardrailValue::Object(BTreeMap::from([
            (
                "type".to_owned(),
                GuardrailValue::Text("image_url".to_owned()),
            ),
            (
                "image_url".to_owned(),
                GuardrailValue::Object(BTreeMap::from([(
                    "url".to_owned(),
                    GuardrailValue::Text(image.image_url.url.clone()),
                )])),
            ),
        ])),
    }
}
