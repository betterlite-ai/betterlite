use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::guardrail_message_scope::StreamItem;
use betterlite_guardrails::unified_dispatch::emit::{
    StreamHttpError, a2a_jsonrpc_error_chunk, a2a_request_id,
};
use serde::Deserialize;

use super::emit::DetailInput;
use super::support::to_json;
use betterlite_oracle_helper_api::helper::HelperResult;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RequestIdRequest {
    pub(super) responses_so_far: Vec<GuardrailValue>,
    pub(super) request_data: BTreeMap<String, GuardrailValue>,
}

pub(super) fn request_id(request: &RequestIdRequest) -> HelperResult {
    let responses_so_far: Vec<StreamItem> = request
        .responses_so_far
        .iter()
        .cloned()
        .map(StreamItem::Json)
        .collect();
    to_json(&a2a_request_id(&responses_so_far, &request.request_data))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ErrorRequest {
    pub(super) status_code: u16,
    pub(super) detail: DetailInput,
    pub(super) request_id: GuardrailValue,
}

pub(super) fn error_chunk(request: &ErrorRequest) -> HelperResult {
    let error = StreamHttpError {
        status_code: request.status_code,
        detail: request.detail.clone().into_detail(),
    };
    to_json(&a2a_jsonrpc_error_chunk(&error, request.request_id.clone()))
}
