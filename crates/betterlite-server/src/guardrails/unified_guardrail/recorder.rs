use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

pub trait AppliedGuardrailRecorder {
    fn record(
        &self,
        request_data: &mut BTreeMap<String, GuardrailValue>,
        guardrail_name: Option<&str>,
    );
}

// LiteLLM's add_guardrail_to_applied_guardrails_header
pub struct MetadataGuardrailRecorder;

impl AppliedGuardrailRecorder for MetadataGuardrailRecorder {
    fn record(
        &self,
        request_data: &mut BTreeMap<String, GuardrailValue>,
        guardrail_name: Option<&str>,
    ) {
        let Some(name) = guardrail_name else {
            return;
        };
        let bucket_key = if request_data.contains_key("litellm_metadata") {
            "litellm_metadata"
        } else {
            "metadata"
        };
        let bucket = request_data
            .entry(bucket_key.to_owned())
            .or_insert(GuardrailValue::Null);
        if !matches!(bucket, GuardrailValue::Object(_)) {
            *bucket = GuardrailValue::Object(BTreeMap::new());
        }
        let GuardrailValue::Object(metadata) = bucket else {
            return;
        };
        let entry = GuardrailValue::Text(name.to_owned());
        match metadata.get_mut("applied_guardrails") {
            Some(GuardrailValue::Array(applied)) => {
                if !applied.contains(&entry) {
                    applied.push(entry);
                }
            }
            Some(_) | None => {
                metadata.insert(
                    "applied_guardrails".to_owned(),
                    GuardrailValue::Array(vec![entry]),
                );
            }
        }
    }
}
