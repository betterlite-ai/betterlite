use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;

use crate::guardrail_metadata::append_guardrail_response;
use crate::guardrail_records::record::{
    GuardrailInvocationRecord, GuardrailRecordContext, GuardrailRecordInput, GuardrailRecordMode,
    GuardrailStatus, GuardrailTracingDetail,
};
use crate::model_armor::decisions::build_logging_response;
use crate::model_armor::transport::tokens::ArmorTokens;

use super::error_handling::HookError;
use super::timing::RecordTimes;
use super::{Data, HookContext, MODEL_ARMOR_PROVIDER, ModelArmorHooks, metadata};

pub(super) enum ApplyVerdict {
    Success { response: GuardrailValue },
    Flagged { response: GuardrailValue },
    FailedToRespond { detail: String },
}

// LiteLLM's add_guardrail_response_to_standard_logging_object call in async_post_call_success_hook
pub(super) fn append_post_call<T>(
    hooks: &ModelArmorHooks<'_, T>,
    data: &Data,
    model_call_details: Option<&mut BTreeMap<String, GuardrailValue>>,
    body: &GuardrailValue,
    blocked: bool,
) -> Result<(), HookError<T::Error>>
where
    T: ArmorTokens,
{
    let logged = GuardrailValue::Object(BTreeMap::from([
        (
            "model_armor_response".to_owned(),
            build_logging_response(body, hooks.guardrail.sanitize_error_detail),
        ),
        (
            "model_armor_status".to_owned(),
            GuardrailValue::Text(
                if blocked {
                    metadata::BLOCKED
                } else {
                    metadata::SUCCESS
                }
                .to_owned(),
            ),
        ),
    ]));
    let record = GuardrailInvocationRecord {
        guardrail_name: hooks.guardrail.base.settings.guardrail_name.clone(),
        guardrail_provider: Some(MODEL_ARMOR_PROVIDER.to_owned()),
        guardrail_mode: Some(GuardrailRecordMode::Event(GuardrailEventHooks::PostCall)),
        guardrail_response: logged,
        guardrail_status: GuardrailStatus::Success,
        start_time: metadata::request_start_time(data),
        end_time: None,
        duration: None,
        masked_entity_count: None,
        tracing_detail: GuardrailTracingDetail::default(),
    };
    append_guardrail_response(model_call_details, &record).map_err(HookError::Metadata)
}

// LiteLLM's apply_guardrail record: the provider always set, the verdict in the status, no event type
pub(super) fn record_apply_guardrail<T>(
    hooks: &ModelArmorHooks<'_, T>,
    data: &mut Data,
    context: &mut HookContext<'_>,
    verdict: ApplyVerdict,
    times: RecordTimes,
) -> Result<(), HookError<T::Error>>
where
    T: ArmorTokens,
{
    let (guardrail_response, guardrail_status) = match verdict {
        ApplyVerdict::Success { response } => (response, GuardrailStatus::Success),
        ApplyVerdict::Flagged { response } => (response, GuardrailStatus::GuardrailFlagged),
        ApplyVerdict::FailedToRespond { detail } => (
            GuardrailValue::Text(detail),
            GuardrailStatus::GuardrailFailedToRespond,
        ),
    };
    let input = GuardrailRecordInput {
        guardrail_response,
        guardrail_status,
        start_time: Some(times.start_time),
        end_time: Some(times.end_time),
        duration: Some(times.duration),
        masked_entity_count: None,
        guardrail_provider: Some(MODEL_ARMOR_PROVIDER.to_owned()),
        event_type: None,
        tracing_detail: None,
    };
    let sink = context.sink;
    let invocation = &mut *context.invocation;
    let log_target = context.log_target.as_deref_mut();
    let mut record = GuardrailRecordContext {
        base: &hooks.guardrail.base,
        request_data: data,
        log_target,
        sink,
    };
    invocation
        .record_own(&mut record, input)
        .map_err(|source| HookError::Record { source })
}

mod tests;
