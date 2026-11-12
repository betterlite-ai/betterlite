use betterlite_logging::python_json::dumps_object;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use regex::Regex;
use serde::Deserialize;

use super::error::ToolResultsError;
use super::records::{FunctionChunk, ToolCallChunk};
use crate::api::responses_codex_tools::records::{AttributeValue, attribute_fields};

#[derive(Debug, Clone, Copy)]
pub struct FunctionToolCall<'item> {
    pub name: &'item str,
    pub arguments: &'item str,
    pub id: Option<&'item str>,
    pub call_id: Option<&'item str>,
    pub provider_specific_fields: Option<&'item AttributeValue>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ApplyPatchOperation {
    CreateFile { diff: String, path: String },
    UpdateFile { diff: String, path: String },
    DeleteFile { path: String },
}

pub struct CallIdentity {
    index_shaped: Regex,
}

impl CallIdentity {
    pub fn new() -> Result<Self, ToolResultsError> {
        let index_shaped =
            Regex::new(r"^call_\d+$").map_err(|source| ToolResultsError::Pattern { source })?;
        Ok(Self { index_shaped })
    }

    // Bedrock Mantle resets call_0, call_1 on every response, so only that shape falls back to the unique id
    #[must_use]
    pub fn tool_call_id(&self, item_id: Option<&str>, call_id: Option<&str>) -> String {
        let present_call = call_id.filter(|id| !id.is_empty());
        let present_item = item_id.filter(|id| !id.is_empty());
        match present_call {
            Some(id) if !self.index_shaped.is_match(id) => id.to_owned(),
            Some(_) | None => present_item.or(present_call).unwrap_or_default().to_owned(),
        }
    }

    pub fn function_tool_call(
        &self,
        item: &FunctionToolCall<'_>,
    ) -> Result<ToolCallChunk, ToolResultsError> {
        let fields = match item.provider_specific_fields {
            None => IndexMap::new(),
            Some(AttributeValue::Plain(Json::Object(fields))) => fields.clone(),
            Some(other) => {
                attribute_fields(other).map_err(|source| ToolResultsError::Attributes { source })?
            }
        };
        let provider_specific_fields = (!fields.is_empty()).then_some(fields);
        Ok(ToolCallChunk {
            id: self.tool_call_id(item.id, item.call_id),
            kind: "function".to_owned(),
            function: FunctionChunk {
                name: item.name.to_owned(),
                arguments: item.arguments.to_owned(),
                provider_specific_fields: provider_specific_fields.clone(),
            },
            index: 0,
            provider_specific_fields,
        })
    }
}

// the field order of the OpenAI models, which json.dumps keeps
fn model_dump(operation: &ApplyPatchOperation) -> IndexMap<String, Json> {
    let (kind, target, patch) = match operation {
        ApplyPatchOperation::CreateFile { diff, path } => ("create_file", path, Some(diff)),
        ApplyPatchOperation::UpdateFile { diff, path } => ("update_file", path, Some(diff)),
        ApplyPatchOperation::DeleteFile { path } => ("delete_file", path, None),
    };
    let mut dumped = IndexMap::new();
    if let Some(text) = patch {
        dumped.insert("diff".to_owned(), Json::Text(text.clone()));
    }
    dumped.insert("path".to_owned(), Json::Text(target.clone()));
    dumped.insert("type".to_owned(), Json::Text(kind.to_owned()));
    dumped
}

#[must_use]
pub fn apply_patch_tool_call(
    call_id: &str,
    operation: &ApplyPatchOperation,
    index: usize,
) -> ToolCallChunk {
    ToolCallChunk {
        id: call_id.to_owned(),
        kind: "function".to_owned(),
        function: FunctionChunk {
            name: "apply_patch".to_owned(),
            arguments: dumps_object(&model_dump(operation)),
            provider_specific_fields: None,
        },
        index,
        provider_specific_fields: None,
    }
}
