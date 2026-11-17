use betterlite_logging::python_json::dumps_object;
use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_objects::sdk::output::FunctionCall;
use betterlite_openai::api::responses_objects::sdk::shell::OutputApplyPatch;
use betterlite_openai::api::responses_objects::sdk::unions::OutputApplyPatchOperationValue;
use indexmap::IndexMap;
use serde_json::{Map, Value};

use super::error::ResponseChatResultError as Error;
use crate::api::response_chat_history::tool_fields::output_tool_fields;
use crate::api::responses_codex_tools::records::AttributeValue;
use crate::api::responses_tool_results::outbound_call::{CallIdentity, FunctionToolCall};
use crate::api::responses_tool_results::records::{FunctionChunk, ToolCallChunk};

// the typed converter hardcodes the call index to 0 while the caller still counts the calls
pub(super) fn typed_function_chunk(call: &FunctionCall) -> Result<ToolCallChunk, Error> {
    let provider_specific_fields = call
        .extra
        .get("provider_specific_fields")
        .cloned()
        .map(AttributeValue::Plain);
    let identity = CallIdentity::new().map_err(|source| Error::Tool { source })?;
    let chunk = identity
        .function_tool_call(&FunctionToolCall {
            name: &call.name,
            arguments: &call.arguments,
            id: call.id.as_deref(),
            call_id: Some(call.call_id.as_str()),
            provider_specific_fields: provider_specific_fields.as_ref(),
        })
        .map_err(|source| Error::Tool { source })?;
    Ok(chat_tool_chunk(chunk))
}

pub(super) fn is_raw_tool_item(fields: &Map<String, Value>) -> bool {
    matches!(
        fields.get("type").and_then(Value::as_str),
        Some("function_call" | "custom_tool_call")
    )
}

pub(super) fn raw_tool_chunk(
    fields: &Map<String, Value>,
    index: usize,
) -> Result<ToolCallChunk, Error> {
    let chunk = output_tool_fields(fields, index).map_err(|source| Error::History { source })?;
    Ok(chat_tool_chunk(chunk))
}

pub(super) fn apply_patch_chunk(call: &OutputApplyPatch, index: usize) -> ToolCallChunk {
    ToolCallChunk {
        id: call.call_id.clone(),
        kind: "function".to_owned(),
        function: FunctionChunk {
            name: "apply_patch".to_owned(),
            arguments: dumps_object(&operation_dump(&call.operation)),
            provider_specific_fields: None,
        },
        index,
        provider_specific_fields: None,
    }
}

// Message's Function model keeps only name and arguments, so the call is the sole carrier of these
fn chat_tool_chunk(mut chunk: ToolCallChunk) -> ToolCallChunk {
    chunk.function.provider_specific_fields = None;
    chunk
}

// json.dumps(operation.model_dump()) prints the pydantic fields in their declared order, extras last
fn operation_dump(operation: &OutputApplyPatchOperationValue) -> IndexMap<String, Json> {
    let mut dumped = IndexMap::new();
    let extra = match operation {
        OutputApplyPatchOperationValue::OperationCreateFile(file) => {
            dump_file_fields(&mut dumped, &file.path, &file.diff, "create_file");
            &file.extra
        }
        OutputApplyPatchOperationValue::OperationUpdateFile(file) => {
            dump_file_fields(&mut dumped, &file.path, &file.diff, "update_file");
            &file.extra
        }
        OutputApplyPatchOperationValue::OperationDeleteFile(file) => {
            dumped.insert("path".to_owned(), Json::Text(file.path.clone()));
            dumped.insert("type".to_owned(), Json::Text("delete_file".to_owned()));
            &file.extra
        }
    };
    for (key, value) in extra {
        dumped.insert(key.clone(), value.clone());
    }
    dumped
}

fn dump_file_fields(dumped: &mut IndexMap<String, Json>, path: &str, diff: &str, kind: &str) {
    dumped.insert("diff".to_owned(), Json::Text(diff.to_owned()));
    dumped.insert("path".to_owned(), Json::Text(path.to_owned()));
    dumped.insert("type".to_owned(), Json::Text(kind.to_owned()));
}
