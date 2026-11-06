use betterlite_prompts::arguments::json::Json;

use crate::typesafe::typesafe_protected_indices::protected_indices;
use crate::typesafe::typesafe_text_extraction::exchange_tool_text;
use crate::typesafe::{MAX_EXCHANGES_EVALUATED, TypeSafeGuardrail};

#[must_use]
pub fn candidate_exchanges(guardrail: &TypeSafeGuardrail, messages: &[Json]) -> Vec<Vec<usize>> {
    let protected = protected_indices(
        messages,
        guardrail.get_protected_indices,
        guardrail.group_tool_exchanges,
    );
    let mut candidates = Vec::new();
    for group in (guardrail.group_tool_exchanges)(messages) {
        if group.len() < 2 {
            continue;
        }
        let starts_with_assistant = group
            .first()
            .and_then(|index| messages.get(*index))
            .and_then(Json::as_object)
            .and_then(|fields| fields.get("role"))
            .and_then(Json::as_str)
            == Some("assistant");
        if !starts_with_assistant {
            continue;
        }
        if group.iter().any(|member| protected.contains(member)) {
            continue;
        }
        if exchange_tool_text(guardrail, messages, &group)
            .chars()
            .count()
            < guardrail.settings.min_chars_to_evaluate.0
        {
            continue;
        }
        candidates.push(group);
    }
    let keep_from = candidates.len().saturating_sub(MAX_EXCHANGES_EVALUATED);
    candidates.split_off(keep_from)
}

#[path = "typesafe_candidate_exchanges/tests.rs"]
mod tests;
