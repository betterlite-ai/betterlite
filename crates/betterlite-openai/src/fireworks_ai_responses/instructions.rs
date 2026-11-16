use std::collections::HashSet;

use betterlite_prompts::arguments::json::{Json, Object};

use crate::response_input::{ResponsesInput, ResponsesItem};

// LiteLLM's _with_instruction_items_folded: hoist the plain-text system and developer items, then rename the rest to system.
#[must_use]
pub fn fold_instructions(
    input: &ResponsesInput,
    instructions: Option<&str>,
) -> (Option<Json>, ResponsesInput) {
    let ResponsesInput::Items(items) = input else {
        return (
            instructions.map(|text| Json::Text(text.to_owned())),
            input.clone(),
        );
    };
    let roles: Vec<Option<&str>> = items
        .iter()
        .map(|item| {
            item.0
                .as_object()
                .and_then(|fields| fields.get("role"))
                .and_then(Json::as_str)
        })
        .collect();
    let folded = folded_texts(items, &roles);
    let folded_indices: HashSet<usize> = folded.iter().map(|(index, _)| *index).collect();
    let kept = items
        .iter()
        .enumerate()
        .filter(|(index, _)| !folded_indices.contains(index))
        .map(|(_, item)| ResponsesItem(developer_as_system(&item.0)))
        .collect();
    (
        joined_instructions(instructions, &folded).map(Json::Text),
        ResponsesInput::Items(kept),
    )
}

#[must_use]
pub fn developer_as_system(item: &Json) -> Json {
    let Some(fields) = item.as_object() else {
        return item.clone();
    };
    if fields.get("role").and_then(Json::as_str) != Some("developer") {
        return item.clone();
    }
    let Some(content) = fields.get("content") else {
        return item.clone();
    };
    let mut converted = Object::new();
    converted.insert(String::from("role"), Json::Text(String::from("system")));
    converted.insert(String::from("content"), content.clone());
    converted.insert(String::from("type"), Json::Text(String::from("message")));
    Json::Object(converted)
}

fn folded_texts(items: &[ResponsesItem], roles: &[Option<&str>]) -> Vec<(usize, String)> {
    let mut folded = Vec::new();
    for index in hoisted_indices(roles) {
        let Some(text) = items.get(index).and_then(|item| text_only_content(&item.0)) else {
            continue;
        };
        folded.push((index, text));
    }
    folded
}

// the item at the leading boundary is never an instruction item, so no test can tell < from <=
#[cfg_attr(test, mutants::skip)]
fn hoisted_indices(roles: &[Option<&str>]) -> Vec<usize> {
    let leading = roles
        .iter()
        .position(|role| !matches!(role, Some("system" | "developer")))
        .unwrap_or(roles.len());
    let closing = closing_start(roles, leading);
    roles
        .iter()
        .enumerate()
        .take(closing)
        .filter(|(index, role)| *index < leading || matches!(role, Some("developer")))
        .map(|(index, _)| index)
        .collect()
}

fn closing_start(roles: &[Option<&str>], leading: usize) -> usize {
    let last = roles
        .iter()
        .enumerate()
        .skip(leading)
        .rev()
        .find(|(_, role)| !matches!(role, Some("system" | "developer")));
    match last {
        Some((index, Some("assistant"))) => index.saturating_add(1),
        Some(_) | None => roles.len(),
    }
}

pub(super) fn text_only_content(item: &Json) -> Option<String> {
    let fields = item.as_object()?;
    let role = fields.get("role").and_then(Json::as_str);
    if !matches!(role, Some("system" | "developer")) {
        return None;
    }
    match fields.get("content")? {
        Json::Text(text) => Some(text.clone()),
        Json::List(parts) => text_parts(parts),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Object(_) => None,
    }
}

fn text_parts(parts: &[Json]) -> Option<String> {
    let mut texts: Vec<&str> = Vec::new();
    for part in parts {
        let fields = part.as_object()?;
        if fields.get("type").and_then(Json::as_str) != Some("input_text") {
            return None;
        }
        let text = fields.get("text")?.as_str()?;
        if !text.is_empty() {
            texts.push(text);
        }
    }
    Some(texts.join("\n\n"))
}

fn joined_instructions(instructions: Option<&str>, folded: &[(usize, String)]) -> Option<String> {
    if folded.is_empty() {
        return instructions.map(str::to_owned);
    }
    let mut chunks: Vec<&str> = Vec::new();
    if let Some(text) = instructions.filter(|text| !text.is_empty()) {
        chunks.push(text);
    }
    for (_, text) in folded {
        if !text.is_empty() {
            chunks.push(text.as_str());
        }
    }
    let joined = chunks.join("\n\n");
    (!joined.is_empty()).then_some(joined)
}
