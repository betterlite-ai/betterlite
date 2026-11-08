use betterlite_models::decisions::Json;
use indexmap::{IndexMap, IndexSet};

use crate::fields::payload::records::StandardLoggingMetadata;
use crate::logging_metadata::decode::{DecodeError, decode};
use crate::logging_metadata::hash::is_valid_sha256_hash;
use crate::logging_metadata::keys::{RESOLVED_METADATA_KEYS, STANDARD_METADATA_KEYS};
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
    let mut merged = IndexMap::new();
    if let Some(metadata) = params.metadata.as_ref() {
        for (key, value) in metadata {
            if !UNSERIALIZABLE_METADATA_KEYS.contains(&key.as_str()) {
                merged.insert(key.clone(), value.clone());
            }
        }
    }
    if let Some(model_metadata) = params.litellm_metadata.as_ref() {
        for (key, value) in model_metadata {
            if !merged.contains_key(key) {
                merged.insert(key.clone(), value.clone());
            }
        }
    }
    merged
}

pub fn get_standard_logging_metadata(
    request: &StandardMetadataRequest<'_>,
) -> Result<StandardLoggingMetadata, DecodeError> {
    let source = request.metadata;
    let mut record = match request.metadata {
        Some(metadata) => decode("standard logging metadata", copied_fields(metadata))?,
        None => StandardLoggingMetadata::default(),
    };
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
    record.used_client_oauth_token = resolved_oauth_token(
        request.metadata,
        request.params,
        request.custom_llm_provider,
    );
    let Some(fields) = request.metadata else {
        return Ok(record);
    };
    merge_guardrails(&mut record, request.applied_guardrails);
    apply_user_api_key(&mut record, fields);
    apply_requester_metadata(&mut record, fields);
    Ok(record)
}

fn copied_fields(metadata: &MetadataMap) -> IndexMap<String, Json> {
    let mut fields = IndexMap::new();
    for key in STANDARD_METADATA_KEYS {
        if RESOLVED_METADATA_KEYS.contains(&key) {
            continue;
        }
        if let Some(value) = metadata.get(key) {
            fields.insert(key.to_owned(), value.clone());
        }
    }
    fields
}

fn prompt_management(request: &StandardMetadataRequest<'_>) -> Option<Json> {
    let params = request.params?;
    let prompt_id = params.prompt_id.as_ref()?;
    let integration = request.prompt_integration?;
    let mut fields = IndexMap::new();
    fields.insert("prompt_id".to_owned(), prompt_id.clone());
    fields.insert(
        "prompt_variables".to_owned(),
        params.prompt_variables.clone().unwrap_or(Json::Null),
    );
    fields.insert(
        "prompt_integration".to_owned(),
        Json::Text(integration.to_owned()),
    );
    Some(Json::Object(fields))
}

fn stamped_oauth_token<'input>(
    metadata: Option<&'input MetadataMap>,
    params: Option<&'input LitellmParams>,
) -> Option<&'input Json> {
    if let Some(litellm_metadata) = params.and_then(|source| source.litellm_metadata.as_ref())
        && let Some(value) = litellm_metadata.get("used_client_oauth_token")
    {
        return Some(value);
    }
    let fields = metadata?;
    fields.get("used_client_oauth_token")
}

fn resolved_oauth_token(
    metadata: Option<&MetadataMap>,
    params: Option<&LitellmParams>,
    custom_llm_provider: Option<&str>,
) -> Option<bool> {
    let Json::Bool(client_sent) = stamped_oauth_token(metadata, params)? else {
        return None;
    };
    Some(*client_sent && custom_llm_provider == Some("anthropic"))
}

fn merge_guardrails(record: &mut StandardLoggingMetadata, applied_guardrails: Option<&[String]>) {
    let Some(argument) = applied_guardrails.filter(|guardrails| !guardrails.is_empty()) else {
        return;
    };
    let Some(recorded) = record.applied_guardrails.as_ref() else {
        return;
    };
    let mut merged = IndexSet::new();
    merged.extend(argument.iter().cloned());
    merged.extend(recorded.iter().cloned());
    record.applied_guardrails = Some(merged.into_iter().collect());
}

fn apply_user_api_key(record: &mut StandardLoggingMetadata, metadata: &MetadataMap) {
    let Some(Json::Text(candidate)) = metadata.get("user_api_key") else {
        return;
    };
    if is_valid_sha256_hash(candidate) {
        record.user_api_key_hash = Some(candidate.clone());
    }
}

fn apply_requester_metadata(record: &mut StandardLoggingMetadata, metadata: &MetadataMap) {
    if record.requester_metadata.is_some() {
        return;
    }
    let Some(Json::Object(inner)) = metadata.get("metadata") else {
        return;
    };
    record.requester_metadata = Some(Json::Object(inner.clone()));
}

mod tests;
