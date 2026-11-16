use std::time::UNIX_EPOCH;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;

use super::error::NomaLifecycleError;
use super::{NomaGuardrail, RequestData};
use crate::guardrail_records::error::GuardrailSpanError;
use crate::guardrail_records::metadata::add_guardrail_record;
use crate::guardrail_records::record::{
    GuardrailInvocationRecord, GuardrailRecordContext, GuardrailRecordInput, GuardrailSpanSink,
    GuardrailStatus,
};
use crate::noma_scan_policy::PROVIDER;
use crate::noma_scan_policy::error::NomaError;

// The record goes to the request data only; LiteLLM's failure path emits no span
struct NoSpanSink;

impl GuardrailSpanSink for NoSpanSink {
    fn emit(&self, _record: &GuardrailInvocationRecord) -> Result<(), GuardrailSpanError> {
        Ok(())
    }
}

static NO_SPAN_SINK: NoSpanSink = NoSpanSink;

impl NomaGuardrail {
    // The error path every hook shares: a block propagates, any other failure is recorded, then block_failures decides
    pub(super) fn handle_failure(
        &self,
        error: NomaLifecycleError,
        request_data: &mut RequestData,
        event_type: GuardrailEventHooks,
        hook_name: &str,
    ) -> Result<(), NomaLifecycleError> {
        if matches!(error, NomaLifecycleError::Noma(NomaError::Blocked(_))) {
            return Err(error);
        }
        let started = (self.policy.now)()
            .duration_since(UNIX_EPOCH)
            .map_err(|source| NomaLifecycleError::Clock { source })?
            .as_secs_f64();
        let mut context = GuardrailRecordContext {
            base: &self.base,
            request_data,
            log_target: None,
            sink: &NO_SPAN_SINK,
        };
        // LiteLLM stamps the failure with one clock reading, so it has no duration
        let input = GuardrailRecordInput {
            guardrail_response: GuardrailValue::Text(error.to_string()),
            guardrail_status: GuardrailStatus::GuardrailFailedToRespond,
            start_time: Some(started),
            end_time: Some(started),
            duration: Some(0.0),
            masked_entity_count: None,
            guardrail_provider: Some(PROVIDER.to_owned()),
            event_type: Some(event_type),
            tracing_detail: None,
        };
        add_guardrail_record(&mut context, input)
            .map_err(|source| NomaLifecycleError::Record { source })?;
        tracing::error!("Noma {hook_name} hook failed: {error}");
        if self.block_failures {
            return Err(error);
        }
        Ok(())
    }
}
