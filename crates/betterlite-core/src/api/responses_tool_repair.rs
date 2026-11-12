use indexmap::IndexMap;

use self::definition::{
    append_tool_call, create_tool_call_chunk, normalize_definition, reconstruct_from_tools,
};
use self::error::RepairError;
use self::value::Value;

pub mod definition;
pub mod error;
pub mod value;

mod tests;

// B2226 owns the TTL storage behind this; B913 writes it and B912 reads it.
pub trait ToolCallCache {
    fn cached_definition(&self, tool_call_id: &str) -> Option<Value>;
}

impl ToolCallCache for IndexMap<String, Value> {
    fn cached_definition(&self, tool_call_id: &str) -> Option<Value> {
        self.get(tool_call_id).cloned()
    }
}

// Only the first call of the nearest assistant is consulted, and only when its tool_calls is a list.
pub fn recover_call_id(assistant: &Value) -> Result<String, RepairError> {
    let Some(Value::List(calls)) = assistant.plain_field("tool_calls") else {
        return Ok(String::new());
    };
    let first_id = calls.first().and_then(|call| call.plain_field("id"));
    Ok(first_id
        .map(|id| id.text("id"))
        .transpose()?
        .unwrap_or_default())
}

// A dict's tool_calls may be any iterable; a dict iterates its keys, and strings and bytes are not calls.
pub fn tool_calls_of(assistant: &Value) -> Vec<Value> {
    match assistant.plain_field("tool_calls") {
        Some(Value::List(items) | Value::Tuple { items }) => items.clone(),
        Some(Value::Dict(map)) => map.keys().cloned().map(Value::Text).collect(),
        None
        | Some(
            Value::Null
            | Value::Bool(_)
            | Value::Number(_)
            | Value::Text(_)
            | Value::Bytes { .. }
            | Value::Instance { .. },
        ) => Vec::new(),
    }
}

#[must_use]
pub fn call_exists(calls: &[Value], tool_call_id: &str) -> bool {
    calls
        .iter()
        .any(|call| matches!(call.plain_field("id"), Some(Value::Text(id)) if id == tool_call_id))
}

fn restore_missing_call(
    assistant: &mut Value,
    tool_call_id: &str,
    tools: &[Value],
    cache: &dyn ToolCallCache,
) -> Result<(), RepairError> {
    let calls = tool_calls_of(assistant);
    if call_exists(&calls, tool_call_id) {
        return Ok(());
    }
    let mut definition = cache
        .cached_definition(tool_call_id)
        .filter(Value::has_content);
    if definition.is_none() {
        definition = reconstruct_from_tools(tool_call_id, tools)?;
    }
    if let Some(found) = definition
        && let Some(normalized) = normalize_definition(&found, tool_call_id)?
    {
        let chunk = create_tool_call_chunk(&normalized, tool_call_id, calls.len())?;
        append_tool_call(assistant, chunk);
    }
    Ok(())
}

// Returns the id the result ends with; an empty one means the result could not be matched to any call.
fn repair_tool_message(
    message: &mut Value,
    earlier: &mut [Value],
    tools: &[Value],
    cache: &dyn ToolCallCache,
) -> Result<String, RepairError> {
    let mut tool_call_id = match message.plain_field("tool_call_id") {
        Some(raw) => raw.text("tool_call_id")?,
        None => String::new(),
    };
    let mut previous = None;
    for candidate in earlier.iter_mut().rev() {
        if candidate.has_role("assistant")? {
            previous = Some(candidate);
            break;
        }
    }
    if tool_call_id.is_empty()
        && let Some(assistant) = &previous
    {
        tool_call_id = recover_call_id(assistant)?;
        if !tool_call_id.is_empty() {
            message.set_tool_call_id(&tool_call_id);
        }
    }
    if !tool_call_id.is_empty()
        && let Some(assistant) = previous
    {
        restore_missing_call(assistant, &tool_call_id, tools, cache)?;
    }
    Ok(tool_call_id)
}

// Works on a copy; each tool result is matched to the nearest earlier assistant, however many messages sit between.
pub fn repair_tool_results(
    messages: &[Value],
    tools: &[Value],
    cache: &dyn ToolCallCache,
) -> Result<Vec<Value>, RepairError> {
    let mut fixed = messages.to_vec();
    let mut non_tool_messages = 0_usize;
    for message in &fixed {
        if !message.has_role("tool")? {
            non_tool_messages = non_tool_messages.saturating_add(1);
        }
    }
    let mut unrecoverable = Vec::new();
    for index in 0..fixed.len() {
        let (earlier, rest) = fixed.split_at_mut(index);
        let Some(message) = rest.first_mut() else {
            continue;
        };
        // with only tool messages left, an empty-id result is kept so the history is never empty
        if message.has_role("tool")?
            && repair_tool_message(message, earlier, tools, cache)?.is_empty()
            && non_tool_messages > 0
        {
            unrecoverable.push(index);
        }
    }
    for index in unrecoverable.into_iter().rev() {
        fixed.remove(index);
    }
    Ok(fixed)
}
