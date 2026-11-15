use crate::guardrail_records::record::{GuardrailRecordContext, GuardrailRecordInput};
use crate::model_armor::transport::tokens::ArmorTokens;

use super::error_handling::HookError;
use super::{Data, HookContext, ModelArmorHooks, metadata, timing};

impl<T: ArmorTokens> ModelArmorHooks<'_, T> {
    // LiteLLM's ModelArmorGuardrail._process_response: log only the Model Armor payload this request stored
    pub(super) fn process_response(
        &self,
        data: &mut Data,
        context: &mut HookContext<'_>,
    ) -> Result<(), HookError<T::Error>> {
        let guardrail_response = metadata::recorded_response(data);
        let guardrail_status = metadata::recorded_status(data);
        let started_at = context.invocation.started_at;
        let times = timing::between(started_at, (self.clock)());
        let input = GuardrailRecordInput {
            guardrail_response,
            guardrail_status,
            start_time: Some(times.start_time),
            end_time: Some(times.end_time),
            duration: Some(times.duration),
            masked_entity_count: None,
            guardrail_provider: None,
            event_type: context.invocation.event_type,
            tracing_detail: None,
        };
        let sink = context.sink;
        let invocation = &mut *context.invocation;
        let log_target = context.log_target.as_deref_mut();
        let mut record = GuardrailRecordContext {
            base: &self.guardrail.base,
            request_data: data,
            log_target,
            sink,
        };
        invocation
            .record_own(&mut record, input)
            .map_err(|source| HookError::Record { source })
    }
}

mod tests;
