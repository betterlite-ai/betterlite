use betterlite_models::decisions::Json;
use serde::Serialize;

use super::BatchCreateError;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BedrockTag {
    pub key: String,
    pub value: String,
}

pub fn validate_bedrock_tags(raw: &Json) -> Result<Vec<BedrockTag>, BatchCreateError> {
    parse_tags(raw).ok_or_else(|| BatchCreateError::InvalidTags {
        raw: serde_json::to_string(raw).unwrap_or_default(),
    })
}

// pydantic's strict TypedDict adapter keeps only string key/value pairs and drops extra entries
fn parse_tags(raw: &Json) -> Option<Vec<BedrockTag>> {
    let Json::List(items) = raw else {
        return None;
    };
    let mut tags = Vec::with_capacity(items.len());
    for item in items {
        let Json::Object(fields) = item else {
            return None;
        };
        let Some(Json::Text(key)) = fields.get("key") else {
            return None;
        };
        let Some(Json::Text(value)) = fields.get("value") else {
            return None;
        };
        tags.push(BedrockTag {
            key: key.clone(),
            value: value.clone(),
        });
    }
    Some(tags)
}
