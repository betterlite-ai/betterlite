use betterlite_prompts::arguments::json::Json;
use indexmap::IndexMap;
use serde::Serialize;

use crate::typesafe::TypeSafeGuardrail;
use crate::typesafe::typesafe_text_extraction::exchange_tool_text;
use crate::typesafe::typesafe_tool_extraction::{ToolCallEntry, tool_call_entries};
use crate::typesafe::typesafe_truncation::truncate_for_state;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TypeSafeExchange {
    pub tool_calls: Vec<ToolCallEntry>,
    pub result: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TypeSafeState {
    pub task: String,
    pub system: String,
    pub tool_exchanges: IndexMap<String, TypeSafeExchange>,
}

#[must_use]
pub fn build_state(
    guardrail: &TypeSafeGuardrail,
    messages: &[Json],
    candidates: &[Vec<usize>],
) -> TypeSafeState {
    let task = messages
        .iter()
        .rev()
        .find_map(|message| {
            let fields = message.as_object()?;
            (fields.get("role").and_then(Json::as_str) == Some("user"))
                .then(|| (guardrail.content_to_text)(fields.get("content")))
        })
        .unwrap_or_default();
    let system = messages
        .iter()
        .filter_map(Json::as_object)
        .filter(|fields| fields.get("role").and_then(Json::as_str) == Some("system"))
        .map(|fields| (guardrail.content_to_text)(fields.get("content")))
        .collect::<Vec<String>>()
        .join("\n\n");
    let mut tool_exchanges = IndexMap::with_capacity(candidates.len());
    for (ordinal, group) in candidates.iter().enumerate() {
        let tool_calls = group
            .first()
            .and_then(|index| messages.get(*index))
            .map(tool_call_entries)
            .unwrap_or_default();
        let result = truncate_for_state(
            &exchange_tool_text(guardrail, messages, group),
            guardrail.settings.max_result_chars_in_state.0,
        );
        tool_exchanges.insert(
            format!("e{ordinal}"),
            TypeSafeExchange { tool_calls, result },
        );
    }
    TypeSafeState {
        task,
        system,
        tool_exchanges,
    }
}

#[path = "typesafe_state_building/tests.rs"]
mod tests;
