use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use serde_json::Number;

use super::HeadroomGuardrail;
use super::apply::{Compressed, UNAVAILABLE_MESSAGE};
use crate::guardrail_base::GuardrailBase;
use crate::guardrail_base::inputs::GuardrailLogTarget;
use crate::guardrail_base::settings::GuardrailSettings;
use crate::guardrail_records::error::GuardrailSpanError;
use crate::guardrail_records::metadata::add_guardrail_record;
use crate::guardrail_records::record::{
    GuardrailInvocationRecord, GuardrailRecordContext, GuardrailRecordInput, GuardrailSpanSink,
    GuardrailStatus,
};
use crate::headroom_settings::HEADROOM_GUARDRAIL_PROVIDER;
use crate::primitives::json_value::JsonValue;

const APPLIED_KEY: &str = "applied_guardrails";
const LITELLM_METADATA_KEY: &str = "litellm_metadata";
const METADATA_KEY: &str = "metadata";

struct NoSpan;

impl GuardrailSpanSink for NoSpan {
    fn emit(&self, _record: &GuardrailInvocationRecord) -> Result<(), GuardrailSpanError> {
        Ok(())
    }
}

pub(super) fn record_outcome(
    guardrail: &HeadroomGuardrail,
    request_data: &mut BTreeMap<String, GuardrailValue>,
    compressed: &Compressed,
    log_target: Option<&mut GuardrailLogTarget>,
) {
    let (status, response) = if compressed.result.succeeded {
        let stats = guardrail_value(&JsonValue::Object(compressed.result.stats.clone()));
        (GuardrailStatus::Success, stats)
    } else {
        let error = GuardrailValue::Object(BTreeMap::from([(
            "error".to_owned(),
            GuardrailValue::Text(UNAVAILABLE_MESSAGE.to_owned()),
        )]));
        (GuardrailStatus::GuardrailFailedToRespond, error)
    };
    let mut settings = GuardrailSettings::default();
    settings
        .guardrail_name
        .clone_from(&guardrail.guardrail_name);
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
        guardrail_response: response,
        guardrail_status: status,
        start_time: Some(compressed.started),
        end_time: Some(compressed.ended),
        duration: Some(compressed.started.mul_add(-1.0_f64, compressed.ended)),
        masked_entity_count: None,
        guardrail_provider: Some(HEADROOM_GUARDRAIL_PROVIDER.to_owned()),
        event_type: None,
        tracing_detail: None,
    };
    if let Err(error) = add_guardrail_record(&mut context, input) {
        tracing::warn!(%error, "Headroom: guardrail record could not be written");
    }
    append_applied(request_data, guardrail.guardrail_name.as_deref());
}

pub(super) fn guardrail_value(value: &JsonValue) -> GuardrailValue {
    match value {
        JsonValue::Null => GuardrailValue::Null,
        JsonValue::Bool(flag) => GuardrailValue::Boolean(*flag),
        JsonValue::Number(number) => number_value(number),
        JsonValue::String(text) => GuardrailValue::Text(text.clone()),
        JsonValue::Array(items) => {
            GuardrailValue::Array(items.iter().map(guardrail_value).collect())
        }
        JsonValue::Object(fields) => GuardrailValue::Object(
            fields
                .iter()
                .map(|(key, field)| (key.clone(), guardrail_value(field)))
                .collect(),
        ),
    }
}

fn number_value(number: &Number) -> GuardrailValue {
    if let Some(signed) = number.as_i64() {
        return GuardrailValue::SignedInteger(signed);
    }
    if let Some(unsigned) = number.as_u64() {
        return GuardrailValue::UnsignedInteger(unsigned);
    }
    number
        .as_f64()
        .map_or(GuardrailValue::Null, GuardrailValue::Decimal)
}

// LiteLLM's add_guardrail_to_applied_guardrails_header
fn append_applied(
    request_data: &mut BTreeMap<String, GuardrailValue>,
    guardrail_name: Option<&str>,
) {
    let Some(name) = guardrail_name else {
        return;
    };
    let key = if request_data.contains_key(LITELLM_METADATA_KEY) {
        LITELLM_METADATA_KEY
    } else {
        METADATA_KEY
    };
    let mut bucket = match request_data.remove(key) {
        Some(GuardrailValue::Object(fields)) => fields,
        _ => BTreeMap::new(),
    };
    let mut applied = match bucket.remove(APPLIED_KEY) {
        Some(GuardrailValue::Array(names)) => names,
        _ => Vec::new(),
    };
    let listed = applied
        .iter()
        .any(|existing| matches!(existing, GuardrailValue::Text(text) if text == name));
    if !listed {
        applied.push(GuardrailValue::Text(name.to_owned()));
    }
    bucket.insert(APPLIED_KEY.to_owned(), GuardrailValue::Array(applied));
    request_data.insert(key.to_owned(), GuardrailValue::Object(bucket));
}
