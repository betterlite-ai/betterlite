use std::collections::BTreeMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use crate::guardrail_base::inputs::GuardrailLogTarget;
use crate::guardrail_records::error::GuardrailSpanError;
use crate::guardrail_records::metadata::add_guardrail_record;
use crate::guardrail_records::record::{
    GuardrailInvocationRecord, GuardrailRecordContext, GuardrailRecordInput, GuardrailSpanSink,
    GuardrailStatus,
};

use super::TypeSafeGuard;
use super::apply::CompactionCounts;

pub(super) const PROVIDER: &str = "typesafe";
pub(super) const UNAVAILABLE_MESSAGE: &str =
    "TypeSafe evaluation unavailable; request forwarded uncompacted";

pub(super) struct Timing {
    pub(super) started: SystemTime,
    pub(super) ended: SystemTime,
    pub(super) duration: Duration,
}

pub(super) struct Outcome {
    status: GuardrailStatus,
    response: GuardrailValue,
}

impl Outcome {
    pub(super) fn unavailable(model: &str) -> Self {
        Self {
            status: GuardrailStatus::GuardrailFailedToRespond,
            response: GuardrailValue::Object(BTreeMap::from([
                (
                    "error".to_owned(),
                    GuardrailValue::Text(UNAVAILABLE_MESSAGE.to_owned()),
                ),
                ("model".to_owned(), GuardrailValue::Text(model.to_owned())),
            ])),
        }
    }

    pub(super) fn compacted(compacted: &CompactionCounts, model: &str) -> Self {
        let counts = [
            ("chars_removed", compacted.chars_removed),
            ("exchanges_dropped", compacted.exchanges_dropped),
            ("exchanges_evaluated", compacted.exchanges_evaluated),
        ];
        let mut response = BTreeMap::new();
        for (key, count) in counts {
            response.insert(key.to_owned(), GuardrailValue::SignedInteger(count));
        }
        response.insert("model".to_owned(), GuardrailValue::Text(model.to_owned()));
        Self {
            status: GuardrailStatus::Success,
            response: GuardrailValue::Object(response),
        }
    }
}

struct NoSpan;

impl GuardrailSpanSink for NoSpan {
    fn emit(&self, _record: &GuardrailInvocationRecord) -> Result<(), GuardrailSpanError> {
        Ok(())
    }
}

impl Timing {
    pub(super) fn record(
        &self,
        guard: &TypeSafeGuard,
        request_data: &mut BTreeMap<String, GuardrailValue>,
        log_target: Option<&mut GuardrailLogTarget>,
        outcome: Outcome,
    ) {
        let mut context = GuardrailRecordContext {
            base: &guard.base,
            request_data,
            log_target,
            sink: &NoSpan,
        };
        let input = GuardrailRecordInput {
            guardrail_response: outcome.response,
            guardrail_status: outcome.status,
            start_time: Some(epoch_seconds(self.started)),
            end_time: Some(epoch_seconds(self.ended)),
            duration: Some(self.duration.as_secs_f64()),
            masked_entity_count: None,
            guardrail_provider: Some(PROVIDER.to_owned()),
            event_type: None,
            tracing_detail: None,
        };
        if let Err(error) = add_guardrail_record(&mut context, input) {
            tracing::warn!(%error, "TypeSafe: guardrail record could not be written");
        }
    }
}

fn epoch_seconds(time: SystemTime) -> f64 {
    time.duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}
