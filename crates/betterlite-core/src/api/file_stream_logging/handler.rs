use betterlite_models::controls::call_params::TrustedCredentials;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use thiserror::Error;

use crate::api::model_credentials::prepare_data_with_credentials;

// LiteLLM's FILE_CONTENT_STREAMING_PROVIDERS: the OpenAI-compatible batch and files providers plus vertex_ai
pub const STREAMING_PROVIDERS: [&str; 4] = ["openai", "hosted_vllm", "litellm_proxy", "vertex_ai"];

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum StreamingParamsError {
    #[error("the model credentials have no `custom_llm_provider`")]
    MissingProvider,
    #[error("the model credentials' `custom_llm_provider` must be text")]
    ProviderNotText,
    #[error("the streaming request has no file id")]
    MissingFileId,
    #[error("the streaming request's file id must be text")]
    FileIdNotText,
}

pub struct StreamingParamsCall<'call> {
    pub custom_llm_provider: &'call str,
    pub file_id: &'call str,
    pub data: &'call JsonObject,
    pub should_route: bool,
    pub original_file_id: Option<&'call str>,
    pub credentials: Option<&'call JsonObject>,
}

// The provider, file id and payload the streaming call runs with, plus the trusted snapshot kept out of it
pub struct ResolvedStreamingParams {
    pub provider: String,
    pub file_id: String,
    pub data: JsonObject,
    pub trusted_credentials: Option<TrustedCredentials>,
}

#[must_use]
pub fn should_stream_file_content(custom_llm_provider: Option<&str>) -> bool {
    let Some(provider) = custom_llm_provider else {
        return false;
    };
    STREAMING_PROVIDERS.contains(&provider)
}

// ast-grep-ignore: no-long-names
pub fn resolve_streaming_request_params(
    call: StreamingParamsCall<'_>,
) -> Result<ResolvedStreamingParams, StreamingParamsError> {
    let credentials = match call.credentials {
        Some(credentials) if call.should_route => credentials,
        Some(_) | None => return Ok(unrouted(&call)),
    };
    let provider = provider_text(credentials)?;
    let mut data = call.data.clone();
    let trusted_credentials =
        prepare_data_with_credentials(&mut data, credentials, call.original_file_id, true);
    data.shift_remove("model");
    let file_id = file_id_text(&data)?;
    Ok(ResolvedStreamingParams {
        provider,
        file_id,
        data,
        trusted_credentials,
    })
}

fn unrouted(call: &StreamingParamsCall<'_>) -> ResolvedStreamingParams {
    ResolvedStreamingParams {
        provider: call.custom_llm_provider.to_owned(),
        file_id: call.file_id.to_owned(),
        data: call.data.clone(),
        trusted_credentials: None,
    }
}

fn provider_text(credentials: &JsonObject) -> Result<String, StreamingParamsError> {
    match credentials.get("custom_llm_provider") {
        Some(Json::Text(text)) => Ok(text.clone()),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(StreamingParamsError::ProviderNotText)
        }
        None => Err(StreamingParamsError::MissingProvider),
    }
}

// prepare_data_with_credentials writes the original file id over the payload's, so the payload carries it either way
fn file_id_text(data: &JsonObject) -> Result<String, StreamingParamsError> {
    match data.get("file_id") {
        Some(Json::Text(text)) => Ok(text.clone()),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(StreamingParamsError::FileIdNotText)
        }
        None => Err(StreamingParamsError::MissingFileId),
    }
}
