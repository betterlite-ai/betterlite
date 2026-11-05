use std::iter;

use crate::arguments::json::Json;

#[must_use]
pub fn declared_tool_call_ids(message: &Json) -> Vec<String> {
    let Some(Json::List(calls)) = message
        .as_object()
        .and_then(|fields| fields.get("tool_calls"))
    else {
        return Vec::new();
    };
    let mut ids = Vec::new();
    for call in calls {
        let Some(id) = call
            .as_object()
            .and_then(|fields| fields.get("id"))
            .and_then(Json::as_str)
            .filter(|id| !id.is_empty())
        else {
            continue;
        };
        if !ids.iter().any(|existing| existing == id) {
            ids.push(id.to_owned());
        }
    }
    ids
}

#[must_use]
pub fn group_tool_exchanges(messages: &[Json]) -> Vec<Vec<usize>> {
    let mut groups = Vec::new();
    let mut active_ids = Vec::new();
    for (index, message) in messages.iter().enumerate() {
        let role = message
            .as_object()
            .and_then(|fields| fields.get("role"))
            .and_then(Json::as_str);
        let tool_id = message
            .as_object()
            .and_then(|fields| fields.get("tool_call_id"))
            .and_then(Json::as_str);
        let belongs_to_active = matches!(role, Some("tool" | "function"))
            && tool_id
                .is_some_and(|candidate_id| active_ids.iter().any(|active| active == candidate_id));
        if !belongs_to_active {
            groups.push(Vec::new());
            active_ids = declared_tool_call_ids(message);
        }
        if let Some(group) = groups.last_mut() {
            group.push(index);
        }
    }
    groups
}

pub fn iter_tool_exchange_groups(messages: &[Json]) -> impl Iterator<Item = Vec<usize>> + '_ {
    let mut index = 0_usize;
    iter::from_fn(move || {
        let message = messages.get(index)?;
        let ids = declared_tool_call_ids(message);
        let start = index;
        index = index.checked_add(1)?;
        while let Some(next_message) = messages.get(index) {
            let role = next_message
                .as_object()
                .and_then(|fields| fields.get("role"))
                .and_then(Json::as_str);
            let tool_id = next_message
                .as_object()
                .and_then(|fields| fields.get("tool_call_id"))
                .and_then(Json::as_str);
            let belongs_to_active = matches!(role, Some("tool" | "function"))
                && tool_id.is_some_and(|candidate_id| ids.iter().any(|id| id == candidate_id));
            if !belongs_to_active {
                break;
            }
            index = index.checked_add(1)?;
        }
        Some((start..index).collect())
    })
}

#[must_use]
pub fn repair_indices(messages: &[Json], modify_params: bool) -> Vec<Vec<usize>> {
    if modify_params {
        group_tool_exchanges(messages)
    } else {
        (0..messages.len()).map(|index| vec![index]).collect()
    }
}
