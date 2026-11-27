use std::future::Future;

use betterlite_openai::api::speech_provider_contract::SpeechVoice;
use betterlite_openai::credentials::LiteLlmGlobals;
use betterlite_openai::openai_base_config::Object;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::json;

use super::error::RunwaySpeechError;
use crate::api::runway::tasks::settings::RunwaySettings;

// the module globals and provider settings LiteLLM's dispatch method resolves against
pub struct DispatchEnvironment<'data> {
    pub globals: &'data LiteLlmGlobals,
    pub settings: &'data RunwaySettings,
}

// RunwayMLTextToSpeechConfig.dispatch_text_to_speech's arguments, before the handler builds the request
pub struct DispatchCall<'data> {
    pub model: &'data str,
    pub input: &'data str,
    pub voice: Option<&'data SpeechVoice>,
    pub optional_params: &'data Object,
    pub litellm_params: Object,
    pub api_base: Option<String>,
    pub api_key: Option<SecretString>,
    pub timeout_seconds: f64,
    pub extra_headers: Option<&'data Object>,
    pub aspeech: bool,
    pub environment: &'data DispatchEnvironment<'data>,
}

// the handler call: provider=runwayml, client=None, _is_async=aspeech, with the resolved pair written into litellm_params
pub struct HandlerRequest<'data> {
    pub model: &'data str,
    pub input: &'data str,
    pub voice: Option<&'data SpeechVoice>,
    pub optional_params: &'data Object,
    pub litellm_params: &'data Object,
    pub api_base: &'data str,
    pub api_key: Option<&'data SecretString>,
    pub timeout_seconds: f64,
    pub extra_headers: Option<&'data Object>,
    pub aspeech: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HandlerFailure {
    pub class: String,
    pub status: Option<u16>,
    pub message: String,
}

// the text-to-speech handler the Runway composition fills with its own HTTP client
pub trait TextToSpeechHandler: Send {
    type Output: Send;

    fn text_to_speech(
        &mut self,
        request: HandlerRequest<'_>,
    ) -> impl Future<Output = Result<Self::Output, HandlerFailure>> + Send;
}

pub async fn dispatch_text_to_speech<H>(
    call: DispatchCall<'_>,
    handler: &mut H,
) -> Result<H::Output, RunwaySpeechError>
where
    H: TextToSpeechHandler,
{
    let api_base = resolve_base(&call)?;
    let api_key = resolve_key(&call)?;
    let litellm_params = resolved_params(&call, &api_base, api_key.as_ref());
    let request = HandlerRequest {
        model: call.model,
        input: call.input,
        voice: call.voice,
        optional_params: call.optional_params,
        litellm_params: &litellm_params,
        api_base: &api_base,
        api_key: api_key.as_ref(),
        timeout_seconds: call.timeout_seconds,
        extra_headers: call.extra_headers,
        aspeech: call.aspeech,
    };
    handler
        .text_to_speech(request)
        .await
        .map_err(|failure| RunwaySpeechError::Handler {
            class: failure.class,
            status: failure.status,
            message: failure.message,
        })
}

// the source writes the resolved pair over whatever the record held, null included
fn resolved_params(
    call: &DispatchCall<'_>,
    api_base: &str,
    api_key: Option<&SecretString>,
) -> Object {
    let mut params = call.litellm_params.clone();
    let resolved_key = match api_key {
        Some(key) => json!(key.expose_secret()),
        None => json!(null),
    };
    params.insert(String::from("api_key"), resolved_key);
    params.insert(String::from("api_base"), json!(api_base));
    params
}

// the explicit base, the call's litellm_params, litellm.api_base, then RUNWAYML_API_BASE or LiteLLM's host
fn resolve_base(call: &DispatchCall<'_>) -> Result<String, RunwaySpeechError> {
    let from_params = params_text(&call.litellm_params, "api_base")?;
    Ok([
        call.api_base.as_deref().filter(|base| !base.is_empty()),
        from_params.filter(|base| !base.is_empty()),
        call.environment
            .globals
            .api_base
            .as_deref()
            .filter(|base| !base.is_empty()),
    ]
    .into_iter()
    .flatten()
    .next()
    .map_or_else(|| call.environment.settings.api_base.clone(), str::to_owned))
}

// the explicit key, the call's litellm_params, litellm.api_key, RUNWAYML_API_SECRET, then RUNWAYML_API_KEY
fn resolve_key(call: &DispatchCall<'_>) -> Result<Option<SecretString>, RunwaySpeechError> {
    let from_params = params_text(&call.litellm_params, "api_key")?.map(SecretString::from);
    let settings = call.environment.settings;
    let candidates = [
        call.api_key.as_ref(),
        from_params.as_ref(),
        call.environment.globals.api_key.as_ref(),
        settings.api_secret.as_ref(),
        settings.api_key.as_ref(),
    ];
    Ok(candidates
        .into_iter()
        .flatten()
        .find(|key| !key.expose_secret().is_empty())
        .cloned())
}

fn params_text<'params>(
    params: &'params Object,
    name: &'static str,
) -> Result<Option<&'params str>, RunwaySpeechError> {
    match params.get(name) {
        Some(value) if value.is_null() => Ok(None),
        Some(value) => value
            .as_str()
            .map(Some)
            .ok_or(RunwaySpeechError::InvalidOption { option: name }),
        None => Ok(None),
    }
}
