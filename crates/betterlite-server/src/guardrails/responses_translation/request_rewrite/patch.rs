use std::collections::BTreeMap;
use std::ops::ControlFlow;

use serde_json::{Map, Value};

use super::error::RequestRewriteError;
use super::{RequestContext, RequestFields};
use crate::guardrails::responses_translation::input_provenance::{
    InputProvenance, input_item_provenance, rewritten_input_item,
};

fn instructions_after(
    changed: &[(usize, &Value)],
    offset: usize,
    raw_instructions: Option<&Value>,
) -> ControlFlow<(), Option<String>> {
    let instruction_rewrite = changed
        .first()
        .filter(|(index, _)| *index < offset)
        .map(|(_, row)| *row);
    let rewritten = match instruction_rewrite {
        Some(Value::Object(row)) => row.get("content"),
        Some(_) | None => raw_instructions,
    };
    match rewritten {
        None | Some(Value::Null) => ControlFlow::Continue(None),
        Some(Value::String(text)) => ControlFlow::Continue(Some(text.clone())),
        Some(_) => ControlFlow::Break(()),
    }
}

// None when a changed message has no single untainted item or an item cannot hold its rewrite
fn item_replacements(
    raw_items: &[Value],
    changed: &[(usize, &Value)],
    offset: usize,
    provenance: &InputProvenance,
) -> Result<Option<BTreeMap<usize, Map<String, Value>>>, RequestRewriteError> {
    let mut targets = Vec::new();
    for (index, row) in changed.iter().filter(|(index, _)| *index >= offset) {
        let body_index = index.saturating_sub(offset);
        match provenance.item_for_message.get(&body_index) {
            Some(item_index) if !provenance.tainted.contains(&body_index) => {
                targets.push((*item_index, *row));
            }
            Some(_) | None => return Ok(None),
        }
    }
    let mut rewritten: BTreeMap<usize, Option<Map<String, Value>>> = BTreeMap::new();
    for (item_index, row) in &targets {
        let Some(Value::Object(item)) = raw_items.get(*item_index) else {
            return Ok(None);
        };
        rewritten.insert(*item_index, rewritten_input_item(item, row)?);
    }
    if rewritten.len() != targets.len() {
        return Ok(None);
    }
    Ok(rewritten
        .into_iter()
        .map(|(index, patched)| patched.map(|item| (index, item)))
        .collect())
}

// Only the raw items whose message changed are rewritten, so the rest keep fields the chat form cannot hold
pub fn patched_request_fields(
    raw_input: Option<&Value>,
    raw_instructions: Option<&Value>,
    original_messages: &[Value],
    merged_messages: &[Value],
    context: &RequestContext<'_>,
) -> Result<Option<RequestFields>, RequestRewriteError> {
    let Some(Value::Array(raw_items)) = raw_input else {
        return Ok(None);
    };
    if original_messages.len() != merged_messages.len() {
        return Ok(None);
    }
    let offset =
        usize::from(matches!(raw_instructions, Some(Value::String(text)) if !text.is_empty()));
    let expected = original_messages.get(offset..).unwrap_or_default();
    let Some(provenance) =
        input_item_provenance(raw_items, expected, context.limits, context.cache)?
    else {
        return Ok(None);
    };
    let changed: Vec<(usize, &Value)> = original_messages
        .iter()
        .zip(merged_messages)
        .enumerate()
        .filter(|(_, (before, after))| before != after)
        .map(|(index, (_, after))| (index, after))
        .collect();
    let ControlFlow::Continue(instructions) =
        instructions_after(&changed, offset, raw_instructions)
    else {
        return Ok(None);
    };
    let Some(replacements) = item_replacements(raw_items, &changed, offset, &provenance)? else {
        return Ok(None);
    };
    let input = raw_items
        .iter()
        .enumerate()
        .map(|(index, item)| {
            replacements
                .get(&index)
                .map_or_else(|| item.clone(), |patched| Value::Object(patched.clone()))
        })
        .collect();
    Ok(Some(RequestFields {
        input,
        instructions,
    }))
}
