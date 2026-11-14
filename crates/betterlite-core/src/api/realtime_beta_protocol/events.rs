use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::error::BetaProtocolError;

const GA_TO_BETA_EVENT_TYPES: [(&str, &str); 7] = [
    ("conversation.item.added", "conversation.item.created"),
    ("response.output_text.delta", "response.text.delta"),
    ("response.output_audio.delta", "response.audio.delta"),
    (
        "response.output_audio_transcript.delta",
        "response.audio_transcript.delta",
    ),
    ("response.output_text.done", "response.text.done"),
    ("response.output_audio.done", "response.audio.done"),
    (
        "response.output_audio_transcript.done",
        "response.audio_transcript.done",
    ),
];

const GA_TO_BETA_CONTENT_TYPES: [(&str, &str); 2] =
    [("output_text", "text"), ("output_audio", "audio")];

pub fn translate_event_to_beta(event: Json) -> Result<Option<Json>, BetaProtocolError> {
    let mut fields = match event {
        Json::Object(fields) => fields,
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            return Err(BetaProtocolError::Event);
        }
    };
    let event_type = match fields.get("type") {
        None | Some(Json::Null) => "",
        Some(Json::Text(text)) => text.as_str(),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            return Err(BetaProtocolError::EventType);
        }
    };
    if event_type == "conversation.item.done" {
        return Ok(None);
    }
    let renamed = beta_event_type(event_type);
    let has_item = matches!(fields.get("item"), Some(Json::Object(_)));
    let has_response_output = match fields.get("response") {
        Some(Json::Object(response)) => matches!(response.get("output"), Some(Json::List(_))),
        None
        | Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            false
        }
    };
    if let Some(renamed) = renamed {
        fields.insert("type".to_owned(), Json::Text(renamed.to_owned()));
    }
    if has_item {
        let translated = match fields.get("item") {
            Some(Json::Object(item)) => Some(Json::Object(translate_item_content_types(item)?)),
            None
            | Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
                None
            }
        };
        if let Some(item) = translated {
            fields.insert("item".to_owned(), item);
        }
    }
    if has_response_output {
        if let Some(Json::Object(response)) = fields.get("response") {
            let translated = translate_response_output(response)?;
            fields.insert("response".to_owned(), Json::Object(translated));
        }
    }
    Ok(Some(Json::Object(fields)))
}

pub fn translate_item_content_types(
    item: &IndexMap<String, Json>,
) -> Result<IndexMap<String, Json>, BetaProtocolError> {
    let Some(Json::List(content)) = item.get("content") else {
        return Ok(item.clone());
    };
    let mut translated = Vec::with_capacity(content.len());
    for block in content {
        translated.push(match block {
            Json::Object(fields) => Json::Object(translate_content_block(fields)?),
            other @ (Json::Null
            | Json::Bool(_)
            | Json::Number(_)
            | Json::Text(_)
            | Json::List(_)) => other.clone(),
        });
    }
    let mut translated_item = item.clone();
    translated_item.insert("content".to_owned(), Json::List(translated));
    Ok(translated_item)
}

fn translate_response_output(
    response: &IndexMap<String, Json>,
) -> Result<IndexMap<String, Json>, BetaProtocolError> {
    let mut translated = response.clone();
    let Some(Json::List(output)) = response.get("output") else {
        return Ok(translated);
    };
    let mut entries = Vec::with_capacity(output.len());
    for entry in output {
        entries.push(match entry {
            Json::Object(item) => Json::Object(translate_item_content_types(item)?),
            other @ (Json::Null
            | Json::Bool(_)
            | Json::Number(_)
            | Json::Text(_)
            | Json::List(_)) => other.clone(),
        });
    }
    translated.insert("output".to_owned(), Json::List(entries));
    Ok(translated)
}

fn translate_content_block(
    fields: &IndexMap<String, Json>,
) -> Result<IndexMap<String, Json>, BetaProtocolError> {
    let translated_type = match fields.get("type") {
        Some(Json::Text(name)) => beta_content_type(name),
        None | Some(Json::Null) => None,
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            return Err(BetaProtocolError::ContentType);
        }
    };
    let mut block = fields.clone();
    if let Some(name) = translated_type {
        block.insert("type".to_owned(), Json::Text(name.to_owned()));
    }
    Ok(block)
}

fn beta_event_type(event_type: &str) -> Option<&'static str> {
    GA_TO_BETA_EVENT_TYPES
        .iter()
        .find(|(native, _)| *native == event_type)
        .map(|(_, beta)| *beta)
}

fn beta_content_type(content_type: &str) -> Option<&'static str> {
    GA_TO_BETA_CONTENT_TYPES
        .iter()
        .find(|(native, _)| *native == content_type)
        .map(|(_, beta)| *beta)
}
