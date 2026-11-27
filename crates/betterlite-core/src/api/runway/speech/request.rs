use betterlite_openai::api::speech_provider_contract::TextToSpeechRequestData;
use betterlite_openai::openai_base_config::Object;
use indexmap::IndexMap;
use serde_json::json;

use super::mapping::VOICE_KEY;
use super::{DEFAULT_MODEL, DEFAULT_VOICE_PRESET_ID, DEFAULT_VOICE_TYPE};

// the source's voice argument is unused here: the mapped voice travels in optional_params
#[must_use]
pub fn speech_request(
    model: &str,
    input: &str,
    optional_params: &Object,
    headers: &IndexMap<String, String>,
) -> TextToSpeechRequestData {
    let voice = optional_params
        .get(VOICE_KEY)
        .filter(|value| !value.is_null())
        .cloned()
        .unwrap_or_else(|| {
            json!({
                "type": DEFAULT_VOICE_TYPE,
                "presetId": DEFAULT_VOICE_PRESET_ID,
            })
        });
    let mut body = Object::new();
    let name = if model.is_empty() {
        DEFAULT_MODEL
    } else {
        model
    };
    body.insert(String::from("model"), json!(name));
    body.insert(String::from("promptText"), json!(input));
    body.insert(String::from("voice"), voice);
    for (parameter, value) in optional_params {
        if !body.contains_key(parameter) && parameter != VOICE_KEY {
            body.insert(parameter.clone(), value.clone());
        }
    }
    TextToSpeechRequestData {
        dict_body: Some(body),
        ssml_body: None,
        headers: Some(headers.clone()),
    }
}
