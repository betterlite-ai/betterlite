use betterlite_config::sections::cisco_verdict_contracts::InspectionType;

use crate::cisco_configuration::records::{CiscoVerdict, ScanContext};
use crate::cisco_conversation::Fields;
use crate::cisco_conversation::json::Json;
use crate::cisco_verdict::Action;
use crate::cisco_verdict_enforcement::strip_sensitive_keys;
use crate::cisco_verdict_enforcement::support::{
    direction_label, insert_serialized, rule_name, surface_label,
};

// Mirrors LiteLLM's fn _stash_verdict_on_request, except a non-object existing metadata value is left alone instead of overwritten.
pub fn stash_verdict_on_request(
    request_data: &mut Fields,
    context: ScanContext,
    verdict: &CiscoVerdict,
) {
    let mut metadata = match Json::field(request_data, "metadata") {
        Json::Object(existing) => existing,
        Json::Null => Fields::new(),
        Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => return,
    };
    let prefix = format!(
        "cisco_ai_defense_{}_{}",
        surface_label(context.surface),
        direction_label(context.direction)
    );
    insert_serialized(
        &mut metadata,
        &format!("{prefix}_is_safe"),
        &verdict.is_safe,
    );
    insert_verdict_metadata_fields(&mut metadata, &prefix, verdict);
    insert_serialized(request_data, "metadata", &metadata);
}

fn insert_verdict_metadata_fields(metadata: &mut Fields, prefix: &str, verdict: &CiscoVerdict) {
    if let Some(action) = verdict.action {
        insert_serialized(metadata, &format!("{prefix}_action"), action.as_str());
    }
    if !verdict.classifications.is_empty() {
        insert_serialized(
            metadata,
            &format!("{prefix}_classifications"),
            &verdict.classifications,
        );
    }
    if let Some(severity) = verdict
        .severity
        .as_deref()
        .filter(|value| !value.is_empty())
    {
        insert_serialized(metadata, &format!("{prefix}_severity"), severity);
    }
    if !verdict.rules.is_empty() {
        let names: Vec<Option<String>> = verdict.rules.iter().map(rule_name).collect();
        insert_serialized(metadata, &format!("{prefix}_rules"), &names);
    }
    if let Some(event_id) = verdict
        .event_id
        .as_deref()
        .filter(|value| !value.is_empty())
    {
        insert_serialized(metadata, &format!("{prefix}_event_id"), event_id);
    }
}

// Mirrors LiteLLM's fn _sanitize_response_for_logging: strips sensitive keys and tags the result with the scan surface and resolved action.
#[must_use]
pub fn sanitize_response_for_logging(
    inspect_response: &Json,
    surface: InspectionType,
    action: Option<Action>,
) -> Fields {
    let Json::Object(fields) = inspect_response else {
        let mut fallback = Fields::new();
        insert_serialized(&mut fallback, "surface", surface_label(surface));
        if let Some(resolved_action) = action {
            insert_serialized(&mut fallback, "action", resolved_action.as_str());
        }
        return fallback;
    };
    let mut sanitized = strip_sensitive_keys(fields);
    insert_serialized(&mut sanitized, "surface", surface_label(surface));
    if let Some(resolved_action) = action {
        insert_serialized(&mut sanitized, "action", resolved_action.as_str());
    }
    sanitized
}

mod tests;
