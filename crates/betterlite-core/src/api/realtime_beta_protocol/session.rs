use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde_json::Number;

use super::error::BetaProtocolError;

const AUDIO_FORMAT_TYPES: [(&str, &str, i32); 3] = [
    ("pcm16", "audio/pcm", 24_000),
    ("g711_ulaw", "audio/G711-ulaw", 8_000),
    ("g711_alaw", "audio/G711-alaw", 8_000),
];

pub fn remap_beta_session_to_ga(session: Json) -> Result<Json, BetaProtocolError> {
    let mut fields = match session {
        Json::Object(fields) => fields,
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            return Err(BetaProtocolError::Session);
        }
    };
    if !fields.contains_key("type") {
        fields.insert("type".to_owned(), Json::Text("realtime".to_owned()));
    }
    remap_modalities(&mut fields)?;
    let audio = lift_audio_fields(&mut fields);
    merge_audio(&mut fields, audio)?;
    Ok(Json::Object(fields))
}

fn remap_modalities(fields: &mut IndexMap<String, Json>) -> Result<(), BetaProtocolError> {
    let Some(modalities) = fields.shift_remove("modalities") else {
        return Ok(());
    };
    if fields.contains_key("output_modalities") {
        return Ok(());
    }
    let requested = match modalities {
        Json::Null => Vec::new(),
        Json::List(items) => items,
        Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_) => {
            return Err(BetaProtocolError::Modalities);
        }
    };
    let mut wants_audio = false;
    let mut wants_text = false;
    for modality in requested {
        match modality {
            Json::Text(name) => match name.to_lowercase().as_str() {
                "audio" => wants_audio = true,
                "text" => wants_text = true,
                _ => {}
            },
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
                return Err(BetaProtocolError::Modalities);
            }
        }
    }
    if wants_audio {
        fields.insert(
            "output_modalities".to_owned(),
            Json::List(vec![Json::Text("audio".to_owned())]),
        );
    } else if wants_text {
        fields.insert(
            "output_modalities".to_owned(),
            Json::List(vec![Json::Text("text".to_owned())]),
        );
    }
    Ok(())
}

fn lift_audio_fields(fields: &mut IndexMap<String, Json>) -> IndexMap<String, Json> {
    let mut input = IndexMap::new();
    let mut output = IndexMap::new();
    if let Some(voice) = fields.shift_remove("voice") {
        output.insert("voice".to_owned(), voice);
    }
    if let Some(raw) = fields.shift_remove("input_audio_format") {
        input.insert("format".to_owned(), audio_format(raw));
    }
    if let Some(raw) = fields.shift_remove("output_audio_format") {
        output.insert("format".to_owned(), audio_format(raw));
    }
    if let Some(turn_detection) = fields.shift_remove("turn_detection") {
        input.insert("turn_detection".to_owned(), turn_detection);
    }
    if let Some(transcription) = fields.shift_remove("input_audio_transcription") {
        input.insert("transcription".to_owned(), transcription);
    }
    let mut audio = IndexMap::new();
    if !input.is_empty() {
        audio.insert("input".to_owned(), Json::Object(input));
    }
    if !output.is_empty() {
        audio.insert("output".to_owned(), Json::Object(output));
    }
    audio
}

fn merge_audio(
    fields: &mut IndexMap<String, Json>,
    audio: IndexMap<String, Json>,
) -> Result<(), BetaProtocolError> {
    if audio.is_empty() {
        return Ok(());
    }
    let mut existing = match fields.shift_remove("audio") {
        None | Some(Json::Null) => IndexMap::new(),
        Some(Json::Object(existing)) => existing,
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            return Err(BetaProtocolError::Audio);
        }
    };
    for (sub_key, sub_value) in audio {
        let merged = match (existing.shift_remove(sub_key.as_str()), sub_value) {
            (Some(Json::Object(nested)), Json::Object(added)) => {
                let mut merged = nested;
                for (key, value) in added {
                    merged.insert(key, value);
                }
                Json::Object(merged)
            }
            (
                None
                | Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)),
                value,
            ) => value,
        };
        existing.insert(sub_key, merged);
    }
    fields.insert("audio".to_owned(), Json::Object(existing));
    Ok(())
}

fn audio_format(raw: Json) -> Json {
    let known = match &raw {
        Json::Text(text) => AUDIO_FORMAT_TYPES
            .iter()
            .find(|(name, _, _)| *name == text.as_str())
            .map(|(_, format, rate)| (*format, *rate)),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => None,
    };
    let Some((format, rate)) = known else {
        return raw;
    };
    let mut fields = IndexMap::new();
    fields.insert("type".to_owned(), Json::Text(format.to_owned()));
    fields.insert("rate".to_owned(), Json::Number(Number::from(rate)));
    Json::Object(fields)
}
