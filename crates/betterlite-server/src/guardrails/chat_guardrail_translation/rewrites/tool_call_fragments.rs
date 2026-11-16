use std::collections::BTreeSet;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::guardrail_message_scope::Object;

use crate::guardrails::chat_guardrail_translation::extraction::json_access::{
    array_field, field, integer_value, object_items,
};

mod tests;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FragmentPosition {
    pub chunk: usize,
    pub choice: usize,
    pub tool_call: usize,
}

struct Fragment {
    index: i64,
    position: FragmentPosition,
    identified: bool,
    named: bool,
}

// A fragment that is not an object is not a tool-call fragment and is left out.
fn describe(fragment: &GuardrailValue, position: FragmentPosition) -> Option<Fragment> {
    let GuardrailValue::Object(_) = fragment else {
        return None;
    };
    let function_name = field(fragment, "function").and_then(|function| field(function, "name"));
    Some(Fragment {
        index: integer_value(field(fragment, "index")).unwrap_or(0),
        position,
        identified: matches!(field(fragment, "id"), Some(GuardrailValue::Text(id)) if !id.is_empty()),
        named: matches!(function_name, Some(GuardrailValue::Text(name)) if !name.is_empty()),
    })
}

// Groups the stream's tool-call fragments by their index, in index order, keeping only indices that carry an id and a name somewhere.
#[must_use]
pub fn function_tool_call_fragments(responses_so_far: &[Object]) -> Vec<Vec<FragmentPosition>> {
    let mut fragments: Vec<Fragment> = Vec::new();
    for (chunk, response) in responses_so_far.iter().enumerate() {
        for (choice, entry) in object_items(response, "choices").iter().enumerate() {
            let Some(delta) = field(entry, "delta") else {
                continue;
            };
            for (tool_call, raw) in array_field(delta, "tool_calls").iter().enumerate() {
                let position = FragmentPosition {
                    chunk,
                    choice,
                    tool_call,
                };
                fragments.extend(describe(raw, position));
            }
        }
    }
    let identified: BTreeSet<i64> = fragments
        .iter()
        .filter(|fragment| fragment.identified)
        .map(|fragment| fragment.index)
        .collect();
    let named: BTreeSet<i64> = fragments
        .iter()
        .filter(|fragment| fragment.named)
        .map(|fragment| fragment.index)
        .collect();
    identified
        .intersection(&named)
        .map(|kept| {
            fragments
                .iter()
                .filter(|fragment| fragment.index == *kept)
                .map(|fragment| fragment.position)
                .collect()
        })
        .collect()
}
