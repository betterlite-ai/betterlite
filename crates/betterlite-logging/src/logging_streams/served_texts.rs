use std::collections::BTreeMap;

use betterlite_models::decisions::Json;

use crate::fields::response::ServedTexts;

pub const SERVED_OUTPUT_TEXTS_KEY: &str = "served_output_texts";
// A stream choice index past this bound is left out; LiteLLM materializes every slot up to it.
pub const MAX_SERVED_CHOICES: usize = 1024;
// The joined text one stream is logged with; a piece that would pass this bound is left out.
pub const MAX_SERVED_TEXT_BYTES: usize = 16 * 1024 * 1024;

const MESSAGE_ROLES: [&str; 5] = ["assistant", "user", "system", "tool", "function"];

// What one chat choice contributes: its text, a null slot kept for its index, or a failed extraction.
enum ChoiceText {
    Text(String),
    Hole,
    Invalid,
}

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
        match choice_text(choice) {
            ChoiceText::Text(text) => texts.push(Some(text)),
            ChoiceText::Hole => texts.push(None),
            ChoiceText::Invalid => return None,
        }
    }
    texts.iter().any(Option::is_some).then_some(texts)
}

// The bounded chat branch of served_stream_output_texts, fed one chunk at a time.
#[derive(Default)]
pub struct StreamTexts {
    choice_count: usize,
    texts: BTreeMap<usize, String>,
    joined_bytes: usize,
    past_bound: bool,
}

impl StreamTexts {
    // Adds the deltas one streamed chunk carries; a piece past the byte bound is left out.
    pub fn push(&mut self, chunk: &Json) {
        let Json::Object(fields) = chunk else {
            return;
        };
        let Some(Json::List(choices)) = fields.get("choices") else {
            return;
        };
        for choice in choices {
            self.push_choice(choice);
        }
    }

    // The joined texts per choice index, holes included, or None when no choice carried text.
    #[must_use]
    pub fn finish(mut self) -> Option<ServedTexts> {
        if self.past_bound {
            tracing::warn!(
                max_choices = MAX_SERVED_CHOICES,
                "a stream choice index is past the served-output bound and was left out"
            );
        }
        let texts: ServedTexts = (0..self.choice_count)
            .map(|index| self.texts.remove(&index))
            .collect();
        texts.iter().any(Option::is_some).then_some(texts)
    }

    fn push_choice(&mut self, entry: &Json) {
        let Json::Object(choice) = entry else {
            return;
        };
        let index = match choice_index(choice.get("index")) {
            ChoiceIndex::Slot(index) => index,
            ChoiceIndex::BeyondBound => {
                self.past_bound = true;
                return;
            }
            ChoiceIndex::Invalid => return,
        };
        self.choice_count = self.choice_count.max(index.saturating_add(1));
        let Some(piece) = delta_text(choice.get("delta")) else {
            return;
        };
        self.append(index, piece);
    }

    fn append(&mut self, index: usize, piece: &str) {
        let remaining = MAX_SERVED_TEXT_BYTES.saturating_sub(self.joined_bytes);
        if piece.len() > remaining {
            return;
        }
        self.joined_bytes = self.joined_bytes.saturating_add(piece.len());
        self.texts.entry(index).or_default().push_str(piece);
    }
}

fn delta_text(slot: Option<&Json>) -> Option<&str> {
    let Some(Json::Object(delta)) = slot else {
        return None;
    };
    match delta.get("content") {
        Some(Json::Text(piece)) => Some(piece),
        None
        | Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            None
        }
    }
}

fn choice_text(choice: &Json) -> ChoiceText {
    let Json::Object(fields) = choice else {
        return ChoiceText::Invalid;
    };
    match fields.get("message") {
        None | Some(Json::Null) => content_text(None),
        Some(Json::Object(message)) => {
            if !role_allowed(message.get("role")) {
                return ChoiceText::Invalid;
            }
            content_text(message.get("content"))
        }
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            ChoiceText::Invalid
        }
    }
}

fn role_allowed(role: Option<&Json>) -> bool {
    match role {
        None | Some(Json::Null) => true,
        Some(Json::Text(name)) => MESSAGE_ROLES.contains(&name.as_str()),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => false,
    }
}

fn content_text(content: Option<&Json>) -> ChoiceText {
    match content {
        None | Some(Json::Null) => ChoiceText::Hole,
        Some(Json::Text(text)) => ChoiceText::Text(text.clone()),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            ChoiceText::Invalid
        }
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
