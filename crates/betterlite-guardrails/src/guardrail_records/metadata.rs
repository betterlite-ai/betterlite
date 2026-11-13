use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use super::error::GuardrailRecordError;
use super::record::{
    GuardrailInvocationRecord, GuardrailRecordContext, GuardrailRecordInput, GuardrailRecordMode,
};
use super::sanitize::sanitize_guardrail_response;
use super::sync::sync_context_target;

pub(super) const RECORDS_KEY: &str = "standard_logging_guardrail_information";

// LiteLLM's get_metadata_variable_name_from_kwargs
pub(super) fn metadata_key(request_data: &BTreeMap<String, GuardrailValue>) -> &'static str {
    if request_data.contains_key("litellm_metadata") {
        "litellm_metadata"
    } else {
        "metadata"
    }
}

// LiteLLM's get_or_create_metadata_bucket and _append_guardrail_info
fn append_entry(request_data: &mut BTreeMap<String, GuardrailValue>, entry: GuardrailValue) {
    let key = metadata_key(request_data);
    let mut bucket = if let Some(GuardrailValue::Object(fields)) = request_data.remove(key) {
        fields
    } else {
        BTreeMap::new()
    };
    let entries = match bucket.remove(RECORDS_KEY) {
        None | Some(GuardrailValue::Null) => vec![entry],
        Some(GuardrailValue::Array(mut entries)) => {
            entries.push(entry);
            entries
        }
        Some(other) => vec![other, entry],
    };
    bucket.insert(RECORDS_KEY.to_owned(), GuardrailValue::Array(entries));
    request_data.insert(key.to_owned(), GuardrailValue::Object(bucket));
}

pub fn add_guardrail_record(
    context: &mut GuardrailRecordContext<'_>,
    input: GuardrailRecordInput,
) -> Result<(), GuardrailRecordError> {
    let settings = &context.base.settings;
    let guardrail_mode = match (input.event_type, &settings.event_hook) {
        (Some(event_type), _) => Some(GuardrailRecordMode::Event(event_type)),
        (None, Some(configured)) => Some(GuardrailRecordMode::Configured(configured.clone())),
        (None, None) => None,
    };
    let record = GuardrailInvocationRecord {
        guardrail_name: settings.guardrail_name.clone(),
        guardrail_provider: input.guardrail_provider,
        guardrail_mode,
        guardrail_response: sanitize_guardrail_response(input.guardrail_response)?,
        guardrail_status: input.guardrail_status,
        start_time: input.start_time,
        end_time: input.end_time,
        duration: input.duration,
        masked_entity_count: input.masked_entity_count,
        tracing_detail: input.tracing_detail.unwrap_or_default(),
    };
    let entry = serde_json::to_value(&record)
        .and_then(serde_json::from_value)
        .map_err(|source| GuardrailRecordError::Json { source })?;
    append_entry(context.request_data, entry);
    sync_context_target(context);
    // a span failure must never reach the request
    if let Err(error) = context.sink.emit(&record) {
        tracing::debug!("guardrail span was not emitted: {error}");
    }
    Ok(())
}
