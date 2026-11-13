use betterlite_prompts::arguments::json::{Json, Object};

use crate::gemini_thinking::audio::{AudioParam, ResponseModality, map_audio_params};

use super::convert;
use super::error::GeminiOptionsError;

pub fn map_audio(value: &Json, optional: &mut Object) -> Result<(), GeminiOptionsError> {
    let audio: AudioParam = convert::decode(value, "audio")?;
    let speech = map_audio_params(&audio)?;
    optional.insert("speechConfig".to_owned(), Json::Object(speech));
    Ok(())
}

// LiteLLM adds AUDIO to responseModalities whenever a speechConfig is present
pub fn ensure_audio_modality(optional: &mut Object) -> Result<(), GeminiOptionsError> {
    if !optional.contains_key("speechConfig") {
        return Ok(());
    }
    let audio = convert::encode(ResponseModality::Audio)?;
    if !optional.contains_key("responseModalities") {
        optional.insert("responseModalities".to_owned(), Json::List(vec![audio]));
        return Ok(());
    }
    let Some(Json::List(modalities)) = optional.get_mut("responseModalities") else {
        return Err(GeminiOptionsError::NotList {
            param: "responseModalities".to_owned(),
        });
    };
    if !modalities.contains(&audio) {
        modalities.push(audio);
    }
    Ok(())
}
