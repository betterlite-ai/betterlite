use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;

pub(super) const fn stage_name(stage: GuardrailEventHooks) -> &'static str {
    match stage {
        GuardrailEventHooks::PreCall => "pre_call",
        GuardrailEventHooks::PostCall => "post_call",
        GuardrailEventHooks::DuringCall => "during_call",
        GuardrailEventHooks::LoggingOnly => "logging_only",
        GuardrailEventHooks::PreMcpCall => "pre_mcp_call",
        GuardrailEventHooks::DuringMcpCall => "during_mcp_call",
        GuardrailEventHooks::PostMcpCall => "post_mcp_call",
        GuardrailEventHooks::RealtimeInputTranscription => "realtime_input_transcription",
    }
}

// LiteLLM writes the entry as a dict in this key order, and its header serializer keeps that order
pub(super) fn scan_entry_value(
    guardrail: Option<&str>,
    provider: &str,
    stage: GuardrailEventHooks,
    scan_id: &str,
) -> GuardrailValue {
    let mut fields = BTreeMap::new();
    let name = guardrail.map_or(GuardrailValue::Null, |found| {
        GuardrailValue::Text(found.to_owned())
    });
    fields.insert("guardrail".to_owned(), name);
    fields.insert(
        "stage".to_owned(),
        GuardrailValue::Text(stage_name(stage).to_owned()),
    );
    fields.insert(
        "provider".to_owned(),
        GuardrailValue::Text(provider.to_owned()),
    );
    fields.insert(
        "scan_id".to_owned(),
        GuardrailValue::Text(scan_id.to_owned()),
    );
    GuardrailValue::Object(fields)
}
