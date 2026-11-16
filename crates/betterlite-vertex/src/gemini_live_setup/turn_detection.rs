use betterlite_prompts::arguments::json::Json;
use serde::Deserialize;

use super::wire::AutomaticActivityDetection;

#[derive(Clone, Debug, Deserialize)]
pub struct TurnDetection {
    #[serde(rename = "type")]
    pub kind: Option<String>,
    pub create_response: Option<bool>,
    pub prefix_padding_ms: Option<Json>,
    pub silence_duration_ms: Option<Json>,
}

#[must_use]
pub fn map_automatic_turn_detection(value: &TurnDetection) -> Option<AutomaticActivityDetection> {
    // semantic VAD without create_response has no Gemini equivalent, and disabled:true breaks native-audio sessions
    if value.kind.as_deref() == Some("semantic_vad") && value.create_response.is_none() {
        return None;
    }
    let disabled = match value.create_response {
        Some(create_response) => !create_response,
        None => value.kind.as_deref() != Some("server_vad"),
    };
    Some(AutomaticActivityDetection {
        disabled: Some(disabled),
        prefix_padding_ms: integer_only(&value.prefix_padding_ms),
        silence_duration_ms: integer_only(&value.silence_duration_ms),
        start_of_speech_sensitivity: None,
        end_of_speech_sensitivity: None,
    })
}

fn integer_only(value: &Option<Json>) -> Option<Json> {
    match value {
        Some(json) if is_json_integer(json) => Some(json.clone()),
        Some(_) | None => None,
    }
}

fn is_json_integer(value: &Json) -> bool {
    match value {
        // Python's isinstance(value, int) is true for bools, and LiteLLM copies those durations too
        Json::Bool(_) => true,
        Json::Number(number) => number.is_i64() || number.is_u64(),
        Json::Null | Json::Text(_) | Json::List(_) | Json::Object(_) => false,
    }
}
