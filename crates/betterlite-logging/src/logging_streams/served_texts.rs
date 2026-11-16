use std::collections::BTreeMap;

use betterlite_models::decisions::Json;

use crate::fields::response::ServedTexts;

pub const SERVED_OUTPUT_TEXTS_KEY: &str = "served_output_texts";
// A stream choice index past this bound is left out; LiteLLM materializes every slot up to it.
pub const MAX_SERVED_CHOICES: usize = 1024;
// The joined text one stream is logged with; a piece that would pass this bound is left out.
pub const MAX_SERVED_TEXT_BYTES: usize = 16 * 1024 * 1024;

const MESSAGE_ROLES: [&str; 5] = ["assistant", "user", "system", "tool", "function"];

enum ChoiceIndex {
    Slot(usize),
    BeyondBound,
    Invalid,
}

// The chat branch of served_output_texts; the Anthropic and Responses branches belong to other tasks.
#[must_use]
pub fn served_output_texts(response: &Json) -> Option<ServedTexts> {
    let Json::Object(fields) = response else {
        return None;
    };
    let Some(Json::List(choices)) = fields.get("choices") else {
        return None;
    };
    let mut texts = Vec::with_capacity(choices.len());
    for choice in choices {
        texts.push(choice_text(choice)?);
    }
    texts.iter().any(Option::is_some).then_some(texts)
}

// The chat branch of served_stream_output_texts: the chunks' deltas joined per choice index.
#[must_use]
pub fn served_stream_output_texts(chunks: &[Json]) -> Option<ServedTexts> {
    let mut choice_count: usize = 0;
    let mut texts: BTreeMap<usize, String> = BTreeMap::new();
    let mut joined_bytes: usize = 0;
    let mut past_bound = false;
    for chunk in chunks {
        let Json::Object(fields) = chunk else {
            continue;
        };
        let Some(Json::List(choices)) = fields.get("choices") else {
            continue;
        };
        for choice in choices {
            let Json::Object(choice) = choice else {
                continue;
            };
            let index = match choice_index(choice.get("index")) {
                ChoiceIndex::Slot(index) => index,
                ChoiceIndex::BeyondBound => {
                    past_bound = true;
                    continue;
                }
                ChoiceIndex::Invalid => continue,
            };
            choice_count = choice_count.max(index.saturating_add(1));
            let content = match choice.get("delta") {
                Some(Json::Object(delta)) => delta.get("content"),
                None
                | Some(
                    Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_),
                ) => None,
            };
            let Some(Json::Text(piece)) = content else {
                continue;
            };
            let remaining = MAX_SERVED_TEXT_BYTES.saturating_sub(joined_bytes);
            if piece.len() <= remaining {
                joined_bytes = joined_bytes.saturating_add(piece.len());
                texts.entry(index).or_default().push_str(piece);
            }
        }
    }
    if past_bound {
        tracing::warn!(
            max_choices = MAX_SERVED_CHOICES,
            "a stream choice index is past the served-output bound and was left out"
        );
    }
    let texts: ServedTexts = (0..choice_count)
        .map(|index| texts.remove(&index))
        .collect();
    texts.iter().any(Option::is_some).then_some(texts)
}

fn choice_text(choice: &Json) -> Option<Option<String>> {
    let Json::Object(fields) = choice else {
        return None;
    };
    match fields.get("message") {
        None | Some(Json::Null) => content_text(None),
        Some(Json::Object(message)) => {
            if !role_allowed(message.get("role")) {
                return None;
            }
            content_text(message.get("content"))
        }
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => None,
    }
}

fn role_allowed(role: Option<&Json>) -> bool {
    match role {
        None | Some(Json::Null) => true,
        Some(Json::Text(name)) => MESSAGE_ROLES.contains(&name.as_str()),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => false,
    }
}

fn content_text(content: Option<&Json>) -> Option<Option<String>> {
    match content {
        None | Some(Json::Null) => Some(None),
        Some(Json::Text(text)) => Some(Some(text.clone())),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => None,
    }
}

fn choice_index(value: Option<&Json>) -> ChoiceIndex {
    match value {
        None | Some(Json::Null) => ChoiceIndex::Slot(0),
        Some(Json::Number(number)) => {
            match number.as_u64().and_then(|raw| usize::try_from(raw).ok()) {
                Some(index) if index < MAX_SERVED_CHOICES => ChoiceIndex::Slot(index),
                Some(_) => ChoiceIndex::BeyondBound,
                None => ChoiceIndex::Invalid,
            }
        }
        Some(Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_)) => {
            ChoiceIndex::Invalid
        }
    }
}
