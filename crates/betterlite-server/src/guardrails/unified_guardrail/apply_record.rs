use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::SystemTime;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use betterlite_guardrails::generic_guardrail_contracts::GuardrailInputType;
use betterlite_guardrails::guardrail_base::GuardrailBase;
use betterlite_guardrails::guardrail_base::contract::GuardrailContract;
use betterlite_guardrails::guardrail_base::failure::GuardrailFailure;
use betterlite_guardrails::guardrail_base::inputs::{GuardrailApiInputs, GuardrailLogTarget};
use betterlite_guardrails::guardrail_records::error::GuardrailRecordError;
use betterlite_guardrails::guardrail_records::invocation::{
    GuardrailHookName, GuardrailInvocation,
};
use betterlite_guardrails::guardrail_records::record::{GuardrailRecordContext, GuardrailSpanSink};

pub trait InvocationClock: Send + Sync {
    fn now(&self) -> SystemTime;
}

// LiteLLM's log_guardrail_information around apply_guardrail; __init_subclass__ wraps every subclass the same way
pub struct RecordedGuardrail<G> {
    pub guardrail: Arc<G>,
    pub base: Arc<GuardrailBase>,
    pub sink: Arc<dyn GuardrailSpanSink + Send + Sync>,
    pub clock: Arc<dyn InvocationClock>,
}

impl<G: GuardrailContract + Send + Sync> GuardrailContract for RecordedGuardrail<G> {
    async fn apply_guardrail(
        &self,
        inputs: GuardrailApiInputs,
        request_data: &mut BTreeMap<String, GuardrailValue>,
        input_type: GuardrailInputType,
        mut logging_obj: Option<&mut GuardrailLogTarget>,
    ) -> Result<GuardrailApiInputs, GuardrailFailure> {
        let started = GuardrailInvocation::begin(
            GuardrailHookName::ApplyGuardrail,
            request_data,
            Some(&inputs),
            self.clock.now(),
        );
        let outcome = self
            .guardrail
            .apply_guardrail(inputs, request_data, input_type, logging_obj.as_deref_mut())
            .await;
        let finished_at = self.clock.now();
        let mut context = GuardrailRecordContext {
            base: &self.base,
            request_data,
            log_target: logging_obj,
            sink: self.sink.as_ref(),
        };
        let recorded = started.and_then(|invocation| match &outcome {
            Ok(returned) => serde_json::to_value(returned)
                .and_then(serde_json::from_value::<GuardrailValue>)
                .map_err(|source| GuardrailRecordError::Json { source })
                .and_then(|summary| {
                    invocation.finish_success(&mut context, Some(&summary), finished_at)
                }),
            Err(failure) => invocation
                .finish_failure(&mut context, failure.clone(), finished_at)
                .map(|_returned| ()),
        });
        if let Err(error) = recorded {
            tracing::warn!("the apply_guardrail call was not recorded: {error}");
        }
        outcome
    }

    fn get_config_model(&self) -> Option<GuardrailValue> {
        self.guardrail.get_config_model()
    }

    fn get_supported_event_hooks(&self) -> Option<Vec<GuardrailEventHooks>> {
        self.guardrail.get_supported_event_hooks()
    }

    fn supports_tool_result_scan(&self) -> bool {
        self.guardrail.supports_tool_result_scan()
    }

    fn messages_cover_full_request(&self) -> bool {
        self.guardrail.messages_cover_full_request()
    }
}
