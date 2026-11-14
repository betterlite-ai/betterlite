use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use crate::crowdstrike_aidr::error::CrowdStrikeError;
use crate::crowdstrike_aidr::models::{
    ContentPart, ImageUrl, ImageUrlContentPart, ImageUrlPartType, Message, MessageContent,
    TextContentPart, TextPartType,
};

// LiteLLM's _normalize_content: unrecognized content-part types are dropped, not replaced
pub fn normalize_content(raw: &GuardrailValue) -> Result<Option<MessageContent>, CrowdStrikeError> {
    match raw {
        GuardrailValue::Null => Ok(None),
        GuardrailValue::Text(text) => Ok(Some(MessageContent::Text(text.clone()))),
        GuardrailValue::Array(items) => {
            normalize_parts(items).map(|parts| Some(MessageContent::Parts(parts)))
        }
        other @ (GuardrailValue::Boolean(_)
        | GuardrailValue::SignedInteger(_)
        | GuardrailValue::UnsignedInteger(_)
        | GuardrailValue::Decimal(_)
        | GuardrailValue::Object(_)) => Ok(Some(MessageContent::Text(
            serde_json::to_string(other)
                .map_err(|source| CrowdStrikeError::JsonEncode { source })?,
        ))),
    }
}

fn normalize_parts(items: &[GuardrailValue]) -> Result<Vec<ContentPart>, CrowdStrikeError> {
    let mut parts = Vec::new();
    for item in items {
        let GuardrailValue::Object(block) = item else {
            parts.push(ContentPart::Text(TextContentPart {
                part_type: TextPartType::Text,
                text: serde_json::to_string(item)
                    .map_err(|source| CrowdStrikeError::JsonEncode { source })?,
            }));
            continue;
        };
        match block.get("type") {
            Some(GuardrailValue::Text(kind)) if kind == "text" => {
                if let Some(GuardrailValue::Text(text)) = block.get("text") {
                    parts.push(ContentPart::Text(TextContentPart {
                        part_type: TextPartType::Text,
                        text: text.clone(),
                    }));
                }
            }
            Some(GuardrailValue::Text(kind)) if kind == "image_url" => {
                parts.push(ContentPart::ImageUrl(ImageUrlContentPart {
                    part_type: ImageUrlPartType::ImageUrl,
                    image_url: image_url(block)?,
                }));
            }
            None
            | Some(
                GuardrailValue::Null
                | GuardrailValue::Boolean(_)
                | GuardrailValue::SignedInteger(_)
                | GuardrailValue::UnsignedInteger(_)
                | GuardrailValue::Decimal(_)
                | GuardrailValue::Text(_)
                | GuardrailValue::Array(_)
                | GuardrailValue::Object(_),
            ) => {}
        }
    }
    Ok(parts)
}

// LiteLLM takes a plain string image_url as the url and reads url from the object form
fn image_url(block: &BTreeMap<String, GuardrailValue>) -> Result<ImageUrl, CrowdStrikeError> {
    match block.get("image_url") {
        None | Some(GuardrailValue::Null) => Ok(ImageUrl { url: String::new() }),
        Some(GuardrailValue::Text(text)) => Ok(ImageUrl { url: text.clone() }),
        Some(GuardrailValue::Object(fields)) => match fields.get("url") {
            None | Some(GuardrailValue::Null) => Ok(ImageUrl { url: String::new() }),
            Some(GuardrailValue::Text(url)) => Ok(ImageUrl { url: url.clone() }),
            Some(_wrong) => Err(CrowdStrikeError::InvalidParam {
                name: "image_url.url",
            }),
        },
        Some(_wrong) => Err(CrowdStrikeError::InvalidParam { name: "image_url" }),
    }
}

#[must_use]
pub fn extract_text_from_content(content: &GuardrailValue) -> String {
    match content {
        GuardrailValue::Text(text) => text.clone(),
        GuardrailValue::Array(items) => text_parts_joined(items),
        GuardrailValue::Null
        | GuardrailValue::Boolean(_)
        | GuardrailValue::SignedInteger(_)
        | GuardrailValue::UnsignedInteger(_)
        | GuardrailValue::Decimal(_)
        | GuardrailValue::Object(_) => String::new(),
    }
}

fn text_parts_joined(items: &[GuardrailValue]) -> String {
    let mut texts = Vec::new();
    for item in items {
        let GuardrailValue::Object(fields) = item else {
            continue;
        };
        if let Some(GuardrailValue::Text(kind)) = fields.get("type")
            && kind == "text"
        {
            let text = match fields.get("text") {
                Some(GuardrailValue::Text(text)) => text.clone(),
                None
                | Some(
                    GuardrailValue::Null
                    | GuardrailValue::Boolean(_)
                    | GuardrailValue::SignedInteger(_)
                    | GuardrailValue::UnsignedInteger(_)
                    | GuardrailValue::Decimal(_)
                    | GuardrailValue::Array(_)
                    | GuardrailValue::Object(_),
                ) => String::new(),
            };
            texts.push(text);
        }
    }
    texts.join("\n")
}

#[must_use]
pub fn extract_text_from_message(message: &Message) -> String {
    match &message.content {
        None => String::new(),
        Some(MessageContent::Text(text)) => text.clone(),
        Some(MessageContent::Parts(parts)) => parts
            .iter()
            .filter_map(|part| match part {
                ContentPart::Text(text) => Some(text.text.clone()),
                ContentPart::ImageUrl(_) => None,
            })
            .collect::<Vec<_>>()
            .join("\n"),
    }
}

// LiteLLM's _merge_metadata_bags: a present bag counts even when empty, and the later bag wins a key
#[must_use]
pub fn merge_metadata_bags(
    request_data: &BTreeMap<String, GuardrailValue>,
) -> Option<BTreeMap<String, GuardrailValue>> {
    let mut merged = BTreeMap::new();
    let mut present = false;
    for key in ["metadata", "litellm_metadata"] {
        if let Some(GuardrailValue::Object(fields)) = request_data.get(key) {
            present = true;
            merged.extend(fields.clone());
        }
    }
    present.then_some(merged)
}

mod tests;
