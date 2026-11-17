use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use betterlite_openai::hosted_vllm_media::{image_edit, responses, transcription, videos};
use betterlite_openai::openai_base_config::Headers;
use indexmap::IndexMap;
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use super::invalid;
use betterlite_oracle_helper_api::error::HelperError;

// LiteLLM's call also names the model, which neither the responses nor the image edit config reads
#[derive(Debug, Deserialize)]
pub(crate) struct EnvCall {
    api_base: Option<String>,
    env: BTreeMap<String, String>,
    headers: Headers,
    api_key: Option<String>,
}

// LiteLLM's call also sets the environment, which the transcription config never reads
#[derive(Debug, Deserialize)]
pub(crate) struct TranscriptionCall {
    api_base: Option<String>,
    model: String,
    options: IndexMap<String, Json>,
    audio: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct VideoCall {
    api_base: Option<String>,
    env: BTreeMap<String, String>,
    headers: Headers,
    api_key: Option<String>,
    params_api_key: Option<String>,
    model: String,
    prompt: String,
    params: Map<String, Value>,
}

pub(super) fn responses_value(call: &EnvCall) -> Result<Value, Box<HelperError>> {
    let key = call.api_key.clone().map(SecretString::from);
    let url = responses::get_complete_url(call.api_base.as_deref(), &call.env)
        .map_err(|error| invalid(error.to_string()))?;
    let headers = responses::validate_environment(call.headers.clone(), key.as_ref(), &call.env);
    Ok(json!({
        "url": url,
        "headers": headers,
        "websocket": responses::SUPPORTS_NATIVE_WEBSOCKET,
    }))
}

pub(super) fn image_edit_value(call: &EnvCall) -> Result<Value, Box<HelperError>> {
    let key = call.api_key.clone().map(SecretString::from);
    let url = image_edit::get_complete_url(call.api_base.as_deref(), &call.env)
        .map_err(|error| invalid(error.to_string()))?;
    let headers = image_edit::validate_environment(call.headers.clone(), key.as_ref(), &call.env);
    let supported = image_edit::get_supported_openai_params();
    Ok(json!({
        "supported": supported,
        "url": url,
        "headers": headers,
    }))
}

pub(super) fn transcription_value(call: &TranscriptionCall) -> Result<Value, Box<HelperError>> {
    let url = transcription::get_complete_url(call.api_base.as_deref())
        .map_err(|error| invalid(error.to_string()))?;
    let data = transcription::transform_audio_transcription_request(&call.model, &call.options)
        .map_err(|error| invalid(error.to_string()))?;
    let mut fields = Map::new();
    if let Some(model) = &data.model {
        fields.insert(
            String::from("model"),
            serde_json::to_value(model).map_err(|error| invalid(error.to_string()))?,
        );
    }
    fields.insert(String::from("file"), Value::String(call.audio.clone()));
    for (name, value) in &data.options {
        fields.insert(
            name.clone(),
            serde_json::to_value(value).map_err(|error| invalid(error.to_string()))?,
        );
    }
    Ok(json!({
        "url": url,
        "data": Value::Object(fields),
    }))
}

pub(super) fn video_value(call: &VideoCall) -> Result<Value, Box<HelperError>> {
    let key = call.api_key.clone().map(SecretString::from);
    let params_key = call.params_api_key.clone().map(SecretString::from);
    let url = videos::get_complete_url(call.api_base.as_deref(), &call.env)
        .map_err(|error| invalid(error.to_string()))?;
    let headers = videos::validate_environment(
        call.headers.clone(),
        key.as_ref(),
        params_key.as_ref(),
        &call.env,
    );
    let form =
        videos::transform_video_create_request(&call.model, &call.prompt, &url, &call.params, None)
            .map_err(|error| invalid(error.to_string()))?;
    let supported = videos::get_supported_openai_params();
    Ok(json!({
        "supported": supported,
        "url": url,
        "headers": headers,
        "fields": Value::Object(form.fields),
    }))
}
