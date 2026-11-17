use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::fields::response::{InitResponseObject, ResponsePrivacy, ServedTexts};
use crate::logging_state::LoggingState;
use crate::logging_streams::overlay::overlay_served_output_texts;
use crate::logging_streams::served_texts::SERVED_OUTPUT_TEXTS_KEY;
use crate::spend_bodies::redaction::redact_response;

// LiteLLM reads _hidden_params off a model result and keeps the dumped body, so a dict result splits the same way.
#[must_use]
pub fn init_response_object(response: &Json) -> InitResponseObject {
    match response {
        Json::Null => InitResponseObject::Absent,
        Json::Object(fields) => match fields.get("_hidden_params") {
            Some(Json::Object(hidden)) => InitResponseObject::Projected {
                body: fields.clone(),
                hidden_params: Some(hidden.clone()),
            },
            Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_))
            | None => InitResponseObject::Mapping(fields.clone()),
        },
        Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            InitResponseObject::Other
        }
    }
}

// record_served_output_texts: the texts the client actually received replace that choice's message content.
pub fn record_served_output_texts(state: &mut LoggingState, texts: Option<ServedTexts>) {
    let Some(served) = texts else {
        return;
    };
    let encoded = served
        .into_iter()
        .map(|text| text.map_or(Json::Null, Json::Text))
        .collect();
    state
        .model_call_details
        .insert(SERVED_OUTPUT_TEXTS_KEY.to_owned(), Json::List(encoded));
}

// The key holds what record_served_output_texts wrote; a non-list or non-text slot is no text at all.
#[must_use]
pub fn served_texts(state: &LoggingState) -> Option<ServedTexts> {
    let Some(Json::List(items)) = state.model_call_details.get(SERVED_OUTPUT_TEXTS_KEY) else {
        return None;
    };
    Some(
        items
            .iter()
            .map(|item| match item {
                Json::Text(text) => Some(text.clone()),
                Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
                    None
                }
            })
            .collect(),
    )
}

// The final logged response: served texts first, then the request's redaction policy.
pub struct PayloadResponsePrivacy {
    pub served_texts: Option<ServedTexts>,
    pub redact_messages: bool,
}

impl PayloadResponsePrivacy {
    #[must_use]
    pub fn from_state(state: &LoggingState, redact_messages: bool) -> Self {
        Self {
            served_texts: served_texts(state),
            redact_messages,
        }
    }
}

impl ResponsePrivacy for PayloadResponsePrivacy {
    fn overlay(&self, response: Json, _served_texts: Option<&ServedTexts>) -> Json {
        overlay_served_output_texts(&response, self.served_texts.as_ref())
    }

    fn redact(&self, response: Json) -> Json {
        if self.redact_messages {
            redact_response(response)
        } else {
            response
        }
    }
}

// The response's own model, when the body carries one and it is usable text.
#[must_use]
pub fn response_model_name(response: &Json) -> Option<&str> {
    let Json::Object(fields) = response else {
        return None;
    };
    match fields.get("model") {
        Some(Json::Text(name)) if !name.is_empty() => Some(name),
        Some(
            Json::Null
            | Json::Bool(_)
            | Json::Number(_)
            | Json::Text(_)
            | Json::List(_)
            | Json::Object(_),
        )
        | None => None,
    }
}

#[must_use]
pub fn object_field<'input>(value: &'input Json, name: &str) -> Option<&'input Json> {
    let Json::Object(fields) = value else {
        return None;
    };
    fields.get(name)
}

#[must_use]
pub fn object_or_empty(value: Option<&Json>) -> IndexMap<String, Json> {
    match value {
        Some(Json::Object(fields)) => fields.clone(),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_))
        | None => IndexMap::new(),
    }
}
