use betterlite_models::decisions::Json;
use indexmap::{IndexMap, IndexSet};

use crate::fields::payload::records::StandardLoggingMetadata;
use crate::logging_metadata::decode::DecodeError;
use crate::logging_metadata::metadata::get_standard_logging_metadata as public_standard_metadata;
use crate::logging_metadata::params::{LitellmParams, MetadataMap};

const UNSERIALIZABLE_METADATA_KEYS: [&str; 2] =
    ["user_api_key_auth", "user_api_key_budget_reservation"];

#[derive(Clone, Debug)]
pub struct StandardMetadataRequest<'input> {
    pub metadata: Option<&'input MetadataMap>,
    pub params: Option<&'input LitellmParams>,
    pub prompt_integration: Option<&'input str>,
    pub applied_guardrails: Option<&'input [String]>,
    pub mcp_tool_call_metadata: Option<&'input Json>,
    pub vector_store_request_metadata: Option<&'input [Json]>,
    pub usage_object: Option<&'input Json>,
    pub custom_llm_provider: Option<&'input str>,
}

#[must_use]
pub fn merge_litellm_metadata(params: &LitellmParams) -> MetadataMap {
    let mut merged: MetadataMap = params
        .metadata
        .iter()
        .flatten()
        .filter(|(key, _value)| !UNSERIALIZABLE_METADATA_KEYS.contains(&key.as_str()))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    for (key, value) in params.litellm_metadata.iter().flatten() {
        merged.entry(key.clone()).or_insert_with(|| value.clone());
    }
    merged
}

pub fn get_standard_logging_metadata(
    request: &StandardMetadataRequest<'_>,
) -> Result<StandardLoggingMetadata, DecodeError> {
    let source = request.metadata;
    let mut record = public_standard_metadata(request.metadata)?;
    if !source.is_some_and(|fields| fields.contains_key("prompt_management_metadata")) {
        record.prompt_management_metadata = prompt_management(request);
    }
    if !source.is_some_and(|fields| fields.contains_key("mcp_tool_call_metadata")) {
        record.mcp_tool_call_metadata = request.mcp_tool_call_metadata.cloned();
    }
    if !source.is_some_and(|fields| fields.contains_key("vector_store_request_metadata")) {
        record.vector_store_request_metadata =
            request.vector_store_request_metadata.map(<[Json]>::to_vec);
    }
    if !source.is_some_and(|fields| fields.contains_key("usage_object")) {
        record.usage_object = request.usage_object.cloned();
    }
    if !source.is_some_and(|fields| fields.contains_key("applied_guardrails")) {
        record.applied_guardrails = request.applied_guardrails.map(<[String]>::to_vec);
    }
    record.used_client_oauth_token = resolved_oauth_token(request);
    let Some(fields) = request.metadata else {
        return Ok(record);
    };
    if let Some(argument) = request
        .applied_guardrails
        .filter(|guardrails| !guardrails.is_empty())
        && let Some(Json::List(_)) = fields.get("applied_guardrails")
        && let Some(recorded) = record.applied_guardrails.as_ref()
    {
        let merged: IndexSet<&String> = argument.iter().chain(recorded).collect();
        record.applied_guardrails = Some(merged.into_iter().cloned().collect());
    }
    if record.requester_metadata.is_none()
        && let Some(Json::Object(inner)) = fields.get("metadata")
    {
        record.requester_metadata = Some(Json::Object(inner.clone()));
    }
    Ok(record)
}

fn prompt_management(request: &StandardMetadataRequest<'_>) -> Option<Json> {
    let params = request.params?;
    Some(Json::Object(IndexMap::from([
        ("prompt_id".to_owned(), params.prompt_id.clone()?),
        (
            "prompt_variables".to_owned(),
            params.prompt_variables.clone().unwrap_or(Json::Null),
        ),
        (
            "prompt_integration".to_owned(),
            Json::Text(request.prompt_integration?.to_owned()),
        ),
    ])))
}

fn resolved_oauth_token(request: &StandardMetadataRequest<'_>) -> Option<bool> {
    let stamped = request
        .params
        .and_then(|source| source.litellm_metadata.as_ref())
        .and_then(|litellm_metadata| litellm_metadata.get("used_client_oauth_token"))
        .or_else(|| request.metadata?.get("used_client_oauth_token"));
    let Some(Json::Bool(client_sent)) = stamped else {
        return None;
    };
    Some(*client_sent && request.custom_llm_provider == Some("anthropic"))
}

mod tests;
