use betterlite_models::decisions::Json;

use crate::cold_storage_key::stamp_cold_storage_object_key;
use crate::fields::payload::records::StandardLoggingMetadata;
use crate::logging_metadata::params::{LitellmParams, MetadataMap};
use crate::logging_metadata::setup::{StandardMetadataRequest, get_standard_logging_metadata};
use crate::payload::compose::PayloadComposition;
use crate::payload::compose::PayloadRequest;
use crate::payload::compose::identifiers::Identifiers;
use crate::payload::compose::inputs::optional_text;
use crate::payload::error::PayloadError;

pub(crate) struct StandardMetadataInput<'input> {
    pub request: &'input PayloadRequest<'input>,
    pub params: &'input LitellmParams,
    pub metadata: &'input MetadataMap,
    pub usage_object: &'input Json,
    pub identifiers: &'input Identifiers,
}

pub(crate) fn standard_metadata(
    input: &StandardMetadataInput<'_>,
    composition: &PayloadComposition<'_>,
) -> Result<StandardLoggingMetadata, PayloadError> {
    let request = input.request;
    let details = &request.state.model_call_details;
    let vector_store = match details.get("vector_store_request_metadata") {
        None | Some(Json::Null) => None,
        Some(Json::List(items)) => Some(items.as_slice()),
        Some(_) => {
            return Err(PayloadError::FieldType {
                field: "vector_store_request_metadata",
            });
        }
    };
    let prompt_integration =
        optional_text(details.get("prompt_integration"), "prompt_integration")?;
    let mut record = get_standard_logging_metadata(&StandardMetadataRequest {
        metadata: Some(input.metadata),
        params: Some(input.params),
        prompt_integration: prompt_integration.as_deref(),
        applied_guardrails: request.state.applied_guardrails.as_deref(),
        mcp_tool_call_metadata: details.get("mcp_tool_call_metadata"),
        vector_store_request_metadata: vector_store,
        usage_object: Some(input.usage_object),
        custom_llm_provider: request.state.custom_llm_provider.as_deref(),
    })
    .map_err(|source| PayloadError::Decode {
        field: "standard logging metadata",
        source,
    })?;
    let target = composition
        .sources
        .cold_storage
        .as_ref()
        .and_then(|locator| locator.locate());
    stamp_cold_storage_object_key(
        &mut record,
        Some(request.start_time.naive_local()),
        Some(input.identifiers.response_id.as_str()),
        target.as_ref(),
    );
    Ok(record)
}
