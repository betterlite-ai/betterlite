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
    if let Some(beta_type) = beta_event_type(event_type) {
        fields.insert("type".to_owned(), Json::Text(beta_type.to_owned()));
    }
    if let Some(Json::Object(item)) = fields.get("item") {
        let translated = translate_item_content_types(item)?;
        fields.insert("item".to_owned(), Json::Object(translated));
    }
    translate_response_output(&mut fields)?;
    Ok(Some(Json::Object(fields)))
}

fn translate_response_output(fields: &mut IndexMap<String, Json>) -> Result<(), BetaProtocolError> {
    let Some(Json::Object(response)) = fields.get_mut("response") else {
        return Ok(());
    };
    let Some(Json::List(output)) = response.get("output") else {
        return Ok(());
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
    response.insert("output".to_owned(), Json::List(entries));
    Ok(())
}

pub fn translate_item_content_types(
    item: &IndexMap<String, Json>,
) -> Result<IndexMap<String, Json>, BetaProtocolError> {
    let Some(Json::List(content)) = item.get("content") else {
        return Ok(item.clone());
    };
    let mut translated_blocks = Vec::with_capacity(content.len());
    for block in content {
        translated_blocks.push(match block {
            Json::Object(fields) => {
                let translated_type = match fields.get("type") {
                    Some(Json::Text(name)) => beta_content_type(name),
                    None | Some(Json::Null) => None,
                    Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
                        return Err(BetaProtocolError::ContentType);
                    }
                };
                let mut translated = fields.clone();
                if let Some(name) = translated_type {
                    translated.insert("type".to_owned(), Json::Text(name.to_owned()));
                }
                Json::Object(translated)
            }
            other @ (Json::Null
            | Json::Bool(_)
            | Json::Number(_)
            | Json::Text(_)
            | Json::List(_)) => other.clone(),
        });
    }
    let mut translated_item = item.clone();
    translated_item.insert("content".to_owned(), Json::List(translated_blocks));
    Ok(translated_item)
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
