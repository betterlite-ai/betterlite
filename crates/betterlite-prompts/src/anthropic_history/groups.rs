use serde_json::Value;

pub fn declared_tool_call_ids(message: &Value) -> Vec<String> {
    let Some(calls) = message
        .as_object()
        .and_then(|object| object.get("tool_calls"))
    else {
        return Vec::new();
    };
    let Some(calls) = calls.as_array() else {
        return Vec::new();
    };
    let mut ids = Vec::new();
    for id in calls
        .iter()
        .filter_map(|call| call.get("id").and_then(Value::as_str))
        .filter(|id| !id.is_empty())
    {
        if !ids.iter().any(|existing| existing == id) {
            ids.push(id.to_owned());
        }
    }
    ids
}

pub fn group_tool_exchanges(messages: &[Value]) -> Vec<Vec<usize>> {
    let mut groups = Vec::new();
    let mut active_ids = Vec::new();
    for (index, message) in messages.iter().enumerate() {
        let role = message.get("role").and_then(Value::as_str);
        let tool_id = message.get("tool_call_id").and_then(Value::as_str);
        let belongs_to_active = matches!(role, Some("tool" | "function"))
            && tool_id.is_some_and(|id| active_ids.iter().any(|active| active == id));
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

pub fn iter_tool_exchange_groups(messages: &[Value]) -> impl Iterator<Item = Vec<usize>> {
    let mut index = 0;
    std::iter::from_fn(move || {
        if index >= messages.len() {
            return None;
        }
        let ids = declared_tool_call_ids(&messages[index]);
        let start = index;
        index += 1;
        while !ids.is_empty() && index < messages.len() {
            let role = messages[index].get("role").and_then(Value::as_str);
            let id = messages[index].get("tool_call_id").and_then(Value::as_str);
            if !matches!(role, Some("tool" | "function"))
                || !id.is_some_and(|id| ids.iter().any(|declared| declared == id))
            {
                break;
            }
            index += 1;
        }
        Some((start..index).collect())
    })
}

pub fn repair_indices(messages: &[Value], modify_params: bool) -> Vec<Vec<usize>> {
    if modify_params {
        group_tool_exchanges(messages)
    } else {
        (0..messages.len()).map(|index| vec![index]).collect()
    }
}
