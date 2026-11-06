use crate::arguments::json::{Json, Object};
use crate::file_ids::managed_id::{is_unified_vector_id, provider_resource_id};
use crate::file_ids::mapping::FileIdMapping;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct ResponsesTool {
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(flatten)]
    pub fields: Object,
}

#[must_use]
pub fn decode_vector_store_ids(tools: Option<&[ResponsesTool]>) -> Option<Vec<ResponsesTool>> {
    let present = tools?;
    let mut updated = Vec::with_capacity(present.len());
    for tool in present {
        updated.push(decode_tool_vector_store_ids(tool));
    }
    Some(updated)
}

fn decode_tool_vector_store_ids(tool: &ResponsesTool) -> ResponsesTool {
    if tool.kind.as_deref() != Some("file_search") {
        return tool.clone();
    }
    let Some(Json::List(ids)) = tool.fields.get("vector_store_ids") else {
        return tool.clone();
    };
    let mut decoded_ids = Vec::with_capacity(ids.len());
    for id in ids {
        let Json::Text(text) = id else {
            decoded_ids.push(id.clone());
            continue;
        };
        if !is_unified_vector_id(text) {
            decoded_ids.push(id.clone());
            continue;
        }
        if let Some(provider) = provider_resource_id(text) {
            decoded_ids.push(Json::Text(provider));
        } else {
            tracing::warn!(
                vector_store_id = %text,
                "a file_search tool carries a unified vector_store_id with no provider resource id; passing it unchanged"
            );
            decoded_ids.push(id.clone());
        }
    }
    let mut updated = tool.clone();
    updated
        .fields
        .insert("vector_store_ids".to_owned(), Json::List(decoded_ids));
    updated
}

#[must_use]
pub fn update_tools_with_file_ids(
    tools: Option<&[ResponsesTool]>,
    model_id: Option<&str>,
    mapping: &FileIdMapping,
) -> Option<Vec<ResponsesTool>> {
    let mut updated = decode_vector_store_ids(tools)?;
    let Some(model) = model_id else {
        return Some(updated);
    };
    if mapping.entries.is_empty() {
        return Some(updated);
    }
    for tool in &mut updated {
        if tool.kind.as_deref() == Some("code_interpreter") {
            map_container_file_ids(tool, model, mapping);
        }
    }
    Some(updated)
}

fn map_container_file_ids(tool: &mut ResponsesTool, model_id: &str, mapping: &FileIdMapping) {
    let Some(Json::Object(container)) = tool.fields.get_mut("container") else {
        return;
    };
    let Some(Json::List(file_ids)) = container.get_mut("file_ids") else {
        return;
    };
    for file_id in file_ids.iter_mut() {
        let Json::Text(text) = file_id else {
            continue;
        };
        if let Some(provider) = mapping.provider_file_id(text, model_id) {
            *file_id = Json::Text(provider.to_owned());
        }
    }
}
