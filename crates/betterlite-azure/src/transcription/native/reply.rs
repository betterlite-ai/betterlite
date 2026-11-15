use betterlite_models::decisions::Json;
use betterlite_openai::api::transcription_contract::TranscriptionResponse;
use betterlite_openai::openai_base_config::{Headers, Object};
use indexmap::IndexMap;

use super::NativeError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeTranscriptionReply {
    pub response: TranscriptionResponse,
    // transform_audio_transcription_response stores the whole native JSON in _hidden_params
    pub hidden_params: Json,
}

// transform_audio_transcription_response over the raw reply bytes
pub fn decode_native_transcription(
    status: u16,
    headers: &Headers,
    body: &[u8],
) -> Result<NativeTranscriptionReply, NativeError> {
    let document =
        serde_json::from_slice::<Json>(body).map_err(|source| NativeError::Json { source })?;
    let Json::Object(fields) = document else {
        return Err(NativeError::NotObject);
    };
    // a missing or null status passes, and so does exactly "Success"
    if let Some(value) = fields.get("RecognitionStatus")
        && !matches!(value, Json::Null)
        && !matches!(value, Json::Text(text) if text == "Success")
    {
        return Err(NativeError::RecognitionStatus {
            status,
            headers: headers.clone(),
            value: value_text(value),
        });
    }
    let text = extract_text(&fields)?;
    Ok(NativeTranscriptionReply {
        response: TranscriptionResponse {
            text: Some(text),
            extra: Object::new(),
        },
        hidden_params: Json::Object(fields),
    })
}

fn extract_text(fields: &IndexMap<String, Json>) -> Result<String, NativeError> {
    // isinstance(response_json.get("DisplayText"), str): any other value falls through to NBest
    if let Some(Json::Text(text)) = fields.get("DisplayText") {
        return Ok(text.clone());
    }
    let Some(Json::List(items)) = fields.get("NBest") else {
        return Ok(String::new());
    };
    let Some(Json::Object(best)) = items.first() else {
        return Ok(String::new());
    };
    match best.get("Display") {
        Some(Json::Text(text)) if !text.is_empty() => Ok(text.clone()),
        Some(Json::Text(_) | Json::Null) | None => nbest_text(best, "Lexical"),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(NativeError::Field {
                field: String::from("NBest[0].Display"),
                problem: "is not a string",
            })
        }
    }
}

fn nbest_text(best: &IndexMap<String, Json>, name: &str) -> Result<String, NativeError> {
    match best.get(name) {
        None | Some(Json::Null) => Ok(String::new()),
        Some(Json::Text(text)) => Ok(text.clone()),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(NativeError::Field {
                field: format!("NBest[0].{name}"),
                problem: "is not a string",
            })
        }
    }
}

// the status value as the message shows it: text as given, any other JSON value in its JSON form
fn value_text(value: &Json) -> String {
    match value {
        Json::Text(text) => text.clone(),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
            serde_json::to_string(value).unwrap_or_default()
        }
    }
}
