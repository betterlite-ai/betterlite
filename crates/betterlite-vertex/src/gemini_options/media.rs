use betterlite_prompts::arguments::json::{Json, Object};

use crate::gemini_thinking::audio::{
    AudioParam, ResponseModality, map_audio_params, map_response_modalities,
};

use super::convert;
use super::error::GeminiOptionsError;

pub fn map_audio(value: &Json, optional: &mut Object) -> Result<(), GeminiOptionsError> {
    match value {
        Json::Object(_) => {}
        Json::Null => return Ok(()),
        Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            return Err(GeminiOptionsError::FieldType {
                param: "audio".to_owned(),
                expected: "an object",
            });
        }
    }
    let audio: AudioParam = convert::decode(value, "audio")?;
    optional.insert(
        "speechConfig".to_owned(),
        Json::Object(map_audio_params(&audio)?),
    );
    Ok(())
}

pub fn map_modalities(value: &Json, optional: &mut Object) -> Result<(), GeminiOptionsError> {
    let Json::List(items) = value else {
        return Err(GeminiOptionsError::FieldType {
            param: "modalities".to_owned(),
            expected: "a list of strings",
        });
    };
    let mut requested = Vec::with_capacity(items.len());
    for item in items {
        let Json::Text(text) = item else {
            return Err(GeminiOptionsError::FieldType {
                param: "modalities".to_owned(),
                expected: "a list of strings",
            });
        };
        requested.push(text.clone());
    }
    let modalities = map_response_modalities(&requested);
    optional.insert(
        "responseModalities".to_owned(),
        convert::encode(modalities)?,
    );
    Ok(())
}

// LiteLLM adds AUDIO to responseModalities whenever a speechConfig is present
pub fn ensure_audio_modality(optional: &mut Object) -> Result<(), GeminiOptionsError> {
    if !optional.contains_key("speechConfig") {
        return Ok(());
    }
    if !optional.contains_key("responseModalities") {
        optional.insert(
            "responseModalities".to_owned(),
            convert::encode([ResponseModality::Audio])?,
        );
        return Ok(());
    }
    let Some(Json::List(modalities)) = optional.get_mut("responseModalities") else {
        return Err(GeminiOptionsError::NotList {
            param: "responseModalities".to_owned(),
        });
    };
    let audio = convert::encode(ResponseModality::Audio)?;
    if !modalities.contains(&audio) {
        modalities.push(audio);
    }
    Ok(())
}
