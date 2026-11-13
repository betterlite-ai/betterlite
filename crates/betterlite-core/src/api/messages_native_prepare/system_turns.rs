use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::system_placement::text_parts::CONVERTED_SYSTEM_NOTE;

use super::billing::filter_billing_headers;

const SYSTEM_ROLE: &str = "system";
const USER_ROLE: &str = "user";
const TOOL_RESULT_TYPE: &str = "tool_result";

#[must_use]
pub fn as_system_content_blocks(value: Option<&Json>) -> Vec<Json> {
    match value {
        None | Some(Json::Null) => Vec::new(),
        Some(Json::List(items)) => items.clone(),
        Some(Json::Text(text)) => vec![text_block(text)],
        Some(other) => vec![other.clone()],
    }
}

fn text_block(text: &str) -> Json {
    Json::Object(Object::from([
        ("type".to_owned(), Json::Text("text".to_owned())),
        ("text".to_owned(), Json::Text(text.to_owned())),
    ]))
}

#[must_use]
pub fn is_system_role_message(message: &Json) -> bool {
    message
        .as_object()
        .and_then(|fields| fields.get("role"))
        .and_then(Json::as_str)
        == Some(SYSTEM_ROLE)
}

// Only the content is carried: LiteLLM builds a fresh turn instead of rewriting the reminder in place
#[must_use]
pub fn system_role_message_as_user(message: &Object) -> Object {
    let mut content = vec![text_block(CONVERTED_SYSTEM_NOTE)];
    content.extend(as_system_content_blocks(message.get("content")));
    Object::from([
        ("role".to_owned(), Json::Text(USER_ROLE.to_owned())),
        ("content".to_owned(), Json::List(content)),
    ])
}

#[must_use]
pub fn opens_with_tool_results(message: &Json) -> bool {
    let Some(fields) = message.as_object() else {
        return false;
    };
    if fields.get("role").and_then(Json::as_str) != Some(USER_ROLE) {
        return false;
    }
    let Some(Json::List(content)) = fields.get("content") else {
        return false;
    };
    let Some(first) = content.first().and_then(Json::as_object) else {
        return false;
    };
    first.get("type").and_then(Json::as_str) == Some(TOOL_RESULT_TYPE)
}

// A tool_result has to open the merged user turn, so a reminder wedged before it moves after that turn
fn place_system_run<'run>(system_run: &'run [Json], follower: &'run [Json]) -> Vec<&'run Json> {
    let mut placed = Vec::with_capacity(system_run.len().saturating_add(follower.len()));
    let (head, tail) = match follower.split_first() {
        Some((first, rest)) if opens_with_tool_results(first) => (Some(first), rest),
        Some(_) | None => (None, follower),
    };
    placed.extend(head);
    placed.extend(system_run);
    placed.extend(tail);
    placed
}

fn runs(messages: &[Json]) -> Vec<&[Json]> {
    messages
        .chunk_by(|left, right| is_system_role_message(left) == is_system_role_message(right))
        .collect()
}

#[must_use]
pub fn place_system_turns(messages: &[Json]) -> Vec<&Json> {
    let runs = runs(messages);
    let mut placed = Vec::with_capacity(messages.len());
    let starts_with_system = runs
        .first()
        .is_some_and(|run| run.first().is_some_and(is_system_role_message));
    let start = if starts_with_system {
        0
    } else {
        placed.extend(runs.first().copied().unwrap_or(&[]));
        1
    };
    let mut pairs = runs.iter().skip(start).copied();
    while let Some(system_run) = pairs.next() {
        let follower = pairs.next().unwrap_or(&[]);
        placed.extend(place_system_run(system_run, follower));
    }
    placed
}

#[must_use]
pub fn convert_mid_system_turns(messages: &[Json]) -> Vec<Json> {
    place_system_turns(messages)
        .into_iter()
        .map(|message| match message.as_object() {
            Some(fields) if is_system_role_message(message) => {
                Json::Object(system_role_message_as_user(fields))
            }
            Some(_) | None => message.clone(),
        })
        .collect()
}

// LiteLLM's _normalize_system_role_messages: hoist the leading run, place or convert the rest, then rebuild system
pub fn normalize_system_turns(request: &mut Object, supports_mid_system: bool) {
    let Some(Json::List(messages)) = request.get("messages").cloned() else {
        return;
    };
    let leading_count = messages
        .iter()
        .position(|message| !is_system_role_message(message))
        .unwrap_or(messages.len());
    let (hoisted, rest) = messages.split_at(leading_count);
    let remaining = if supports_mid_system {
        rest.to_vec()
    } else {
        convert_mid_system_turns(rest)
    };
    request.insert("messages".to_owned(), Json::List(remaining));
    let mut system_content = as_system_content_blocks(request.get("system"));
    for message in hoisted {
        system_content.extend(as_system_content_blocks(
            message.as_object().and_then(|fields| fields.get("content")),
        ));
    }
    match filter_billing_headers(&Json::List(system_content)) {
        Some(filtered) => {
            request.insert("system".to_owned(), filtered);
        }
        None => {
            request.shift_remove("system");
        }
    }
}
