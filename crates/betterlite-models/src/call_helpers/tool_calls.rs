use std::collections::BTreeSet;

use indexmap::IndexMap;

use crate::decisions::Json;

pub fn remove_index_from_tool_calls(messages: &mut [IndexMap<String, Json>]) {
    for message in messages {
        let Some(Json::List(tool_calls)) = message.get_mut("tool_calls") else {
            continue;
        };
        for tool_call in tool_calls {
            if let Json::Object(fields) = tool_call {
                fields.shift_remove("index");
            }
        }
    }
}

pub fn remove_items_at_indices<Item>(items: &mut Vec<Item>, indices: &[i64]) {
    let positions: BTreeSet<i64> = indices.iter().copied().collect();
    for index in positions.into_iter().rev() {
        let Ok(position) = usize::try_from(index) else {
            continue;
        };
        if position < items.len() {
            items.remove(position);
        }
    }
}
