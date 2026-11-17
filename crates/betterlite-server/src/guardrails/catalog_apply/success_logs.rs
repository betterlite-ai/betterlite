use std::collections::BTreeMap;
use std::time::SystemTime;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::opaque::OpaqueParams;
use betterlite_config::sections::guardrail_params::routes::{
    ApplyGuardrailRequest, ApplyGuardrailResponse,
};

use crate::auth::auth_identity::AuthContext;

use super::ports::{ApplyHooks, ApplyLog, Body, SuccessPayload};

// LiteLLM's _emit_guardrail_success_logs: neither hook may fail the request
pub(super) struct SuccessLogging<'context> {
    pub(super) hooks: &'context dyn ApplyHooks,
    pub(super) caller: &'context AuthContext,
    pub(super) data: &'context Body,
    pub(super) log: Option<&'context dyn ApplyLog>,
    pub(super) started_at: SystemTime,
}

// LiteLLM's _patch_logging_obj_for_guardrail: the request's messages or one user turn of the text
pub(super) fn pass_through_messages(request: &ApplyGuardrailRequest) -> Vec<OpaqueParams> {
    match &request.messages {
        Some(messages) if !messages.is_empty() => messages.clone(),
        _ => vec![OpaqueParams(BTreeMap::from([
            ("role".to_owned(), GuardrailValue::Text("user".to_owned())),
            (
                "content".to_owned(),
                GuardrailValue::Text(request.text.clone()),
            ),
        ]))],
    }
}

pub(super) async fn emit(
    response: ApplyGuardrailResponse,
    logging: &SuccessLogging<'_>,
) -> ApplyGuardrailResponse {
    let fallback = response.clone();
    let hooked = match logging
        .hooks
        .post_call_success(logging.data, logging.caller, response)
        .await
    {
        Ok(Some(modified)) => modified,
        Ok(None) => fallback,
        Err(error) => {
            tracing::error!(error = %error, "apply_guardrail: post_call_success_hook failed");
            fallback
        }
    };
    let Some(log) = logging.log else {
        return hooked;
    };
    let payload = SuccessPayload {
        response: hooked.clone(),
    };
    let end_time = SystemTime::now();
    if log
        .async_success_handler(payload.clone(), logging.started_at, end_time, false)
        .await
        .is_err()
    {
        tracing::error!("apply_guardrail: async_success_handler failed");
    }
    if log
        .submit_success_handler(payload, logging.started_at, end_time, false)
        .is_err()
    {
        tracing::error!("apply_guardrail: success_handler submit failed");
    }
    hooked
}
