use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use super::Compressed;
use super::results::Applied;
use crate::compresr_settings::CompresrGuardrail;
use crate::guardrail_base::GuardrailBase;
use crate::guardrail_base::inputs::GuardrailLogTarget;
use crate::guardrail_base::settings::GuardrailSettings;
use crate::guardrail_records::error::GuardrailSpanError;
use crate::guardrail_records::metadata::add_guardrail_record;
use crate::guardrail_records::record::{
    GuardrailInvocationRecord, GuardrailRecordContext, GuardrailRecordInput, GuardrailSpanSink,
    GuardrailStatus,
};

const GUARDRAIL_PROVIDER: &str = "compresr";

struct NoSpan;

impl GuardrailSpanSink for NoSpan {
    fn emit(&self, _record: &GuardrailInvocationRecord) -> Result<(), GuardrailSpanError> {
        Ok(())
    }
}

fn compression_stats(model: &str, applied: &Applied) -> GuardrailValue {
    let counts = [
        ("messages_compressed", applied.messages_compressed),
        ("tokens_before", applied.tokens_before),
        ("tokens_after", applied.tokens_after),
        (
            "tokens_saved",
            applied.tokens_before.saturating_sub(applied.tokens_after),
        ),
    ];
    let mut stats: BTreeMap<String, GuardrailValue> = counts
        .into_iter()
        .map(|(name, count)| (name.to_owned(), GuardrailValue::SignedInteger(count)))
        .collect();
    stats.insert(
        "compression_model".to_owned(),
        GuardrailValue::Text(model.to_owned()),
    );
    GuardrailValue::Object(stats)
}

pub(super) fn record_success(
    guardrail: &CompresrGuardrail,
    request_data: &mut BTreeMap<String, GuardrailValue>,
    log_target: Option<&mut GuardrailLogTarget>,
    applied: &Applied,
    window: &Compressed<'_>,
) {
    let mut settings = GuardrailSettings::default();
    let guardrail_name = Some(guardrail.guardrail_name.clone());
    settings.guardrail_name.clone_from(&guardrail_name);
    settings
        .event_hook
        .clone_from(&guardrail.runtime.event_hook);
    let base = GuardrailBase {
        settings,
        mode: None,
    };
    let mut context = GuardrailRecordContext {
        base: &base,
        request_data,
        log_target,
        sink: &NoSpan,
    };
    let input = GuardrailRecordInput {
        guardrail_response: compression_stats(&guardrail.transport.settings.model, applied),
        guardrail_status: GuardrailStatus::Success,
        start_time: Some(window.started.as_secs_f64()),
        end_time: Some(window.ended.as_secs_f64()),
        duration: Some(window.ended.saturating_sub(window.started).as_secs_f64()),
        masked_entity_count: None,
        guardrail_provider: Some(GUARDRAIL_PROVIDER.to_owned()),
        event_type: None,
        tracing_detail: None,
    };
    if let Err(error) = add_guardrail_record(&mut context, input) {
        tracing::warn!(%error, "Compresr: guardrail record could not be written");
    }
}
