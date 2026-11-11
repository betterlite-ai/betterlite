use betterlite_models::decisions::Json;

use crate::datadog_llm_obs::constants::ROUTER_SPAN_FIELDS;
use crate::datadog_llm_obs::values::{Object, as_object};

#[rustfmt::skip]
pub const ROUTER_PROMPT_FIELDS: [&str; 4] = [
    "signals", "matched_keyword", "escalation_keyword", "classifier_crux",
];

// LiteLLM flattens the decision into six span tags and drops the prompt-quoting values when redacting
#[must_use]
pub fn router_span_fields(routing_decision: Option<&Json>, redact_prompt_text: bool) -> Object {
    let Some(decision) = routing_decision
        .and_then(as_object)
        .filter(|fields| !fields.is_empty())
    else {
        return Object::new();
    };
    let escalated = matches!(decision.get("escalated"), Some(Json::Bool(true)))
        || matches!(decision.get("context_escalated"), Some(Json::Bool(true)));
    let mut fields = Object::new();
    for (record_field, value) in decision {
        if redact_prompt_text && ROUTER_PROMPT_FIELDS.contains(&record_field.as_str()) {
            continue;
        }
        let Some(name) = span_name(record_field) else {
            continue;
        };
        if !matches!(value, Json::Null) {
            fields.insert(name.to_owned(), value.clone());
        }
    }
    fields.insert("router_escalated".to_owned(), Json::Bool(escalated));
    fields
}

fn span_name(record_field: &str) -> Option<&'static str> {
    ROUTER_SPAN_FIELDS
        .iter()
        .find(|(field, _name)| *field == record_field)
        .map(|(_field, name)| *name)
}

mod tests;
