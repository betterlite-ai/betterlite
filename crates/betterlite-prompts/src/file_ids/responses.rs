use crate::arguments::json::{Json, Object};
use crate::file_ids::limits::{FileIdsError, check_responses_input};
use crate::file_ids::managed_id::{llm_output_file_id, original_file_id};
use crate::file_ids::mapping::FileIdMapping;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum ResponsesInput {
    Text(String),
    Items(Vec<ResponsesItem>),
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct ResponsesItem {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<ResponsesContent>,
    #[serde(flatten)]
    pub fields: Object,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum ResponsesContent {
    Text(String),
    Parts(Vec<ResponsesContentPart>),
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct ResponsesContentPart {
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(flatten)]
    pub fields: Object,
}

pub fn update_responses_file_ids(
    input: &ResponsesInput,
    model_id: Option<&str>,
    mapping: &FileIdMapping,
) -> Result<ResponsesInput, FileIdsError> {
    check_responses_input(input)?;
    let ResponsesInput::Items(items) = input else {
        return Ok(input.clone());
    };
    let mut updated = Vec::with_capacity(items.len());
    for item in items {
        let mut updated_item = item.clone();
        if let Some(ResponsesContent::Parts(parts)) = &mut updated_item.content {
            for part in parts.iter_mut() {
                update_content_part(part, model_id, mapping);
            }
        }
        updated.push(updated_item);
    }
    Ok(ResponsesInput::Items(updated))
}

fn update_content_part(
    part: &mut ResponsesContentPart,
    model_id: Option<&str>,
    mapping: &FileIdMapping,
) {
    if part.kind.as_deref() != Some("input_file") {
        return;
    }
    let Some(file_id) = part
        .fields
        .get("file_id")
        .and_then(Json::as_str)
        .filter(|value| !value.is_empty())
    else {
        return;
    };
    let provider = if let Some(model) = model_id.filter(|_| mapping.entries.contains_key(file_id)) {
        mapping
            .provider_file_id(file_id, model)
            .unwrap_or(file_id)
            .to_owned()
    } else if let Some(output) = llm_output_file_id(file_id) {
        output
    } else if let Some(original) = original_file_id(file_id) {
        original
    } else {
        return;
    };
    part.fields
        .insert("file_id".to_owned(), Json::Text(provider));
}
