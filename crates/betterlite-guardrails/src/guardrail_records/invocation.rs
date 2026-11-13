use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

use betterlite_config::sections::guardrail_identity::item::{GuardrailFlag, GuardrailValue};
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;

use super::error::GuardrailRecordError;
use super::metadata::add_guardrail_record;
use super::record::{GuardrailRecordContext, GuardrailRecordInput, GuardrailStatus};
use super::summary::{PRE_CALL_CONTENT_KEYS, summarize_guardrail_response};
use super::sync::sync_context_target;
use crate::guardrail_base::failure::GuardrailFailure;
use crate::guardrail_base::inputs::GuardrailApiInputs;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuardrailHookName {
    PreCall,
    Moderation,
    PostCallSuccess,
    PostCallStreaming,
    StreamingIterator,
    ApplyGuardrail,
}

// What log_guardrail_information does around one hook call, as a value the caller starts and finishes
#[derive(Debug, Clone, PartialEq)]
pub struct GuardrailInvocation {
    pub self_recorded: GuardrailFlag,
    pub event_type: Option<GuardrailEventHooks>,
    pub baseline: Option<BTreeMap<String, GuardrailValue>>,
    pub started_at: SystemTime,
}

impl GuardrailInvocation {
    // the baseline is copied before the hook runs, since hooks edit their argument in place
    pub fn begin(
        hook: GuardrailHookName,
        request_data: &BTreeMap<String, GuardrailValue>,
        inputs: Option<&GuardrailApiInputs>,
        started_at: SystemTime,
    ) -> Result<Self, GuardrailRecordError> {
        let event_type = match hook {
            GuardrailHookName::PreCall => Some(GuardrailEventHooks::PreCall),
            GuardrailHookName::Moderation => Some(GuardrailEventHooks::DuringCall),
            GuardrailHookName::PostCallSuccess | GuardrailHookName::PostCallStreaming => {
                Some(GuardrailEventHooks::PostCall)
            }
            GuardrailHookName::StreamingIterator | GuardrailHookName::ApplyGuardrail => None,
        };
        let baseline = if hook == GuardrailHookName::ApplyGuardrail {
            inputs
                .map(|given| serde_json::to_value(given).and_then(serde_json::from_value))
                .transpose()
                .map_err(|source| GuardrailRecordError::Json { source })?
        } else if event_type == Some(GuardrailEventHooks::PreCall) {
            Some(
                request_data
                    .iter()
                    .filter(|(key, _value)| PRE_CALL_CONTENT_KEYS.contains(&key.as_str()))
                    .map(|(key, value)| (key.clone(), value.clone()))
                    .collect(),
            )
        } else {
            None
        };
        Ok(Self {
            self_recorded: false,
            event_type,
            baseline,
            started_at,
        })
    }

    pub fn record_own(
        &mut self,
        context: &mut GuardrailRecordContext<'_>,
        input: GuardrailRecordInput,
    ) -> Result<(), GuardrailRecordError> {
        add_guardrail_record(context, input)?;
        self.self_recorded = true;
        Ok(())
    }

    fn record_input(
        &self,
        response: GuardrailValue,
        status: GuardrailStatus,
        finished_at: SystemTime,
    ) -> GuardrailRecordInput {
        let start_time = self
            .started_at
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        let end_time = finished_at.duration_since(UNIX_EPOCH).unwrap_or_default();
        let duration = finished_at
            .duration_since(self.started_at)
            .unwrap_or_default();
        GuardrailRecordInput {
            guardrail_response: response,
            guardrail_status: status,
            start_time: Some(start_time.as_secs_f64()),
            end_time: Some(end_time.as_secs_f64()),
            duration: Some(duration.as_secs_f64()),
            masked_entity_count: None,
            guardrail_provider: None,
            event_type: self.event_type,
            tracing_detail: None,
        }
    }

    fn finish(
        &self,
        context: &mut GuardrailRecordContext<'_>,
        skip: bool,
        record: (GuardrailValue, GuardrailStatus),
        finished_at: SystemTime,
    ) -> Result<(), GuardrailRecordError> {
        let outcome = if skip {
            Ok(())
        } else {
            let (response, status) = record;
            add_guardrail_record(context, self.record_input(response, status, finished_at))
        };
        sync_context_target(context);
        outcome
    }

    pub fn finish_success(
        self,
        context: &mut GuardrailRecordContext<'_>,
        response: Option<&GuardrailValue>,
        finished_at: SystemTime,
    ) -> Result<(), GuardrailRecordError> {
        let skip = self.self_recorded || context.base.settings.switches.records_own_information;
        let summary =
            summarize_guardrail_response(response, self.baseline.as_ref(), self.event_type);
        self.finish(
            context,
            skip,
            (summary, GuardrailStatus::Success),
            finished_at,
        )
    }

    pub fn finish_failure(
        self,
        context: &mut GuardrailRecordContext<'_>,
        failure: GuardrailFailure,
        finished_at: SystemTime,
    ) -> Result<GuardrailFailure, GuardrailRecordError> {
        let status = if failure.is_intervention() {
            GuardrailStatus::GuardrailIntervened
        } else {
            GuardrailStatus::GuardrailFailedToRespond
        };
        let text = if context.base.settings.switches.deny_text_on_error {
            "deny".to_owned()
        } else {
            failure.to_string()
        };
        let skip = self.self_recorded;
        self.finish(
            context,
            skip,
            (GuardrailValue::Text(text), status),
            finished_at,
        )?;
        Ok(failure)
    }
}
