use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_redact::structured::mask_sensitive_structure;
use indexmap::IndexSet;

// format_no_fallback_group_message: the lookup chain, then the configured keys in first-seen order without duplicates
#[must_use]
pub fn no_fallback_group_message(lookup_groups: &[String], fallbacks: &[JsonObject]) -> String {
    let requested = lookup_groups.join(" -> ");
    let configured = configured_keys(fallbacks);
    let configured_text = if configured.is_empty() {
        " No fallbacks are configured.".to_owned()
    } else {
        format!(" Fallbacks are configured for: {}.", configured.join(", "))
    };
    format!(
        "\n\nLiteLLM: model group '{requested}' failed with the error above and no fallback model group was found for it, so the request was not retried on another model.{configured_text} Add a fallbacks entry for that model group (Router fallbacks or proxy router_settings.fallbacks) to retry on another model."
    )
}

fn configured_keys(fallbacks: &[JsonObject]) -> Vec<String> {
    let mut keys: IndexSet<String> = IndexSet::new();
    for entry in fallbacks {
        keys.extend(entry.keys().cloned());
    }
    keys.into_iter().collect()
}

// format_fallback_outcome_message: Python's str(None) is what a missing model group renders as, and targets are masked first
#[must_use]
pub fn fallback_outcome_message(
    model_group: Option<&str>,
    fallback_model_group: Option<&[Json]>,
    fallback_failure_detail: &str,
) -> String {
    let lead = format!(
        "\n\nLiteLLM: model group '{}' failed with the error above.",
        model_group.unwrap_or("None")
    );
    let Some(targets) = fallback_model_group.filter(|group| !group.is_empty()) else {
        return format!("{lead} No fallback was attempted.");
    };
    let rendered = targets
        .iter()
        .map(target_text)
        .collect::<Vec<_>>()
        .join(", ");
    if fallback_failure_detail.is_empty() {
        return format!("{lead} Fallback model group(s) configured: {rendered}.");
    }
    format!("{lead} Fallback to {rendered} also failed: {fallback_failure_detail}")
}

// str(mask_sensitive_structure(target)): a string target renders as itself, and anything else as the JSON betterlite writes everywhere
fn target_text(target: &Json) -> String {
    if let Json::Text(text) = target {
        return text.clone();
    }
    // masking is typed on serde_json's Value, and a document that cannot be encoded must never hand back the secret
    let serialized = serde_json::to_value(target).unwrap_or_default();
    serde_json::to_string(&mask_sensitive_structure(&serialized)).unwrap_or_default()
}
