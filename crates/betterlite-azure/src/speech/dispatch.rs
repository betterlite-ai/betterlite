use std::collections::BTreeMap;
use std::future::Future;
use std::time::Duration;

use betterlite_config::loader::EnvironmentLookup;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use secrecy::{ExposeSecret, SecretString};

use super::error::AzureSpeechError;

pub const SUPPORTED_OPENAI_PARAMS: [&str; 3] = ["voice", "response_format", "speed"];

// The LiteLLM module globals the credential chain reads before the environment.
#[derive(Debug, Default, Clone)]
pub struct SpeechGlobals {
    pub api_base: Option<String>,
    pub api_key: Option<SecretString>,
    pub azure_key: Option<SecretString>,
}

pub struct SpeechEnvironment<'data> {
    pub globals: &'data SpeechGlobals,
    pub environment: &'data dyn EnvironmentLookup,
}

// AzureAVATextToSpeechConfig.dispatch_text_to_speech's arguments, before main.py's parameter mapping.
pub struct DispatchCall<'data> {
    pub model: &'data str,
    pub input: &'data str,
    pub voice: Option<&'data Json>,
    pub optional_params: &'data IndexMap<String, Json>,
    pub litellm_params: IndexMap<String, Json>,
    pub api_base: Option<String>,
    pub api_key: Option<SecretString>,
    pub timeout: Option<Duration>,
    pub extra_headers: Option<&'data BTreeMap<String, String>>,
    pub aspeech: bool,
    pub environment: &'data SpeechEnvironment<'data>,
}

// The handler port call: provider=azure, client=None, _is_async=aspeech, with the resolved credentials in litellm_params.
pub struct HandlerRequest<'data> {
    pub model: &'data str,
    pub input: &'data str,
    pub voice: Option<&'data str>,
    pub optional_params: &'data IndexMap<String, Json>,
    pub litellm_params: &'data IndexMap<String, Json>,
    pub extra_headers: Option<&'data BTreeMap<String, String>>,
    pub timeout: Option<Duration>,
    pub api_key: Option<&'data str>,
    pub api_base: Option<&'data str>,
    pub provider: &'static str,
    pub is_async: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HandlerFailure {
    pub class: String,
    pub status: Option<u16>,
    pub message: String,
}

// The text-to-speech handler betterlite-provider fills with B2452's speech_http handler.
pub trait TextToSpeechHandler: Send {
    type Output: Send;

    fn text_to_speech(
        &mut self,
        request: HandlerRequest<'_>,
    ) -> impl Future<Output = Result<Self::Output, HandlerFailure>> + Send;
}

#[must_use]
pub fn supported_openai_params() -> Vec<String> {
    SUPPORTED_OPENAI_PARAMS
        .iter()
        .map(|name| (*name).to_owned())
        .collect()
}

pub async fn dispatch_text_to_speech<H>(
    call: DispatchCall<'_>,
    handler: &mut H,
) -> Result<H::Output, AzureSpeechError>
where
    H: TextToSpeechHandler,
{
    let resolved = Resolved::resolve(&call)?;
    handler
        .text_to_speech(resolved.request(&call))
        .await
        .map_err(|failure| AzureSpeechError::Handler {
            class: failure.class,
            status: failure.status,
            message: failure.message,
        })
}

struct Resolved {
    api_base: Option<String>,
    api_key: Option<SecretString>,
    voice: Option<String>,
    litellm_params: IndexMap<String, Json>,
}

impl Resolved {
    fn resolve(call: &DispatchCall<'_>) -> Result<Self, AzureSpeechError> {
        let api_base = resolve_base(call)?;
        let api_key = resolve_key(call)?;
        let voice = resolve_voice(call.voice)?;
        let mut litellm_params = call.litellm_params.clone();
        litellm_params.insert(
            String::from("api_key"),
            api_key
                .as_ref()
                .map_or(Json::Null, |key| Json::Text(key.expose_secret().to_owned())),
        );
        litellm_params.insert(
            String::from("api_base"),
            api_base
                .as_ref()
                .map_or(Json::Null, |base| Json::Text(base.clone())),
        );
        Ok(Self {
            api_base,
            api_key,
            voice,
            litellm_params,
        })
    }

    fn request<'data>(&'data self, call: &'data DispatchCall<'_>) -> HandlerRequest<'data> {
        HandlerRequest {
            model: call.model,
            input: call.input,
            voice: self.voice.as_deref(),
            optional_params: call.optional_params,
            litellm_params: &self.litellm_params,
            extra_headers: call.extra_headers,
            timeout: call.timeout,
            api_key: self.api_key.as_ref().map(ExposeSecret::expose_secret),
            api_base: self.api_base.as_deref(),
            provider: "azure",
            is_async: call.aspeech,
        }
    }
}

// A string voice stays as it is, a dict voice contributes its name, and any other value becomes None.
fn resolve_voice(voice: Option<&Json>) -> Result<Option<String>, AzureSpeechError> {
    match voice {
        None | Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_)) => Ok(None),
        Some(Json::Text(name)) => Ok(Some(name.clone())),
        Some(Json::Object(fields)) => match fields.get("name") {
            Some(Json::Text(name)) => Ok(Some(name.clone())),
            None | Some(Json::Null) => Ok(None),
            Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
                Err(AzureSpeechError::Option {
                    option: String::from("voice"),
                    problem: "name is not a string",
                })
            }
        },
    }
}

// The explicit value comes first, then the call's litellm_params, the module global and AZURE_API_BASE.
fn resolve_base(call: &DispatchCall<'_>) -> Result<Option<String>, AzureSpeechError> {
    if let Some(base) = &call.api_base {
        return Ok(Some(base.clone()));
    }
    if let Some(base) = params_text(&call.litellm_params, "api_base")? {
        return Ok(Some(base.to_owned()));
    }
    if let Some(base) = &call.environment.globals.api_base {
        return Ok(Some(base.clone()));
    }
    Ok(environment_secret(call.environment, "AZURE_API_BASE"))
}

// The explicit key, the call's litellm_params, litellm.api_key, litellm.azure_key, then either Azure variable.
fn resolve_key(call: &DispatchCall<'_>) -> Result<Option<SecretString>, AzureSpeechError> {
    let globals = call.environment.globals;
    let from_params = params_text(&call.litellm_params, "api_key")?;
    Ok(call
        .api_key
        .clone()
        .or_else(|| from_params.map(SecretString::from))
        .or_else(|| globals.api_key.clone())
        .or_else(|| globals.azure_key.clone())
        .or_else(|| {
            environment_secret(call.environment, "AZURE_OPENAI_API_KEY").map(SecretString::from)
        })
        .or_else(|| environment_secret(call.environment, "AZURE_API_KEY").map(SecretString::from)))
}

fn params_text<'data>(
    params: &'data IndexMap<String, Json>,
    name: &str,
) -> Result<Option<&'data str>, AzureSpeechError> {
    match params.get(name) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(text)) => Ok(Some(text)),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(AzureSpeechError::Option {
                option: name.to_owned(),
                problem: "is not a string",
            })
        }
    }
}

fn environment_secret(environment: &SpeechEnvironment<'_>, name: &str) -> Option<String> {
    environment
        .environment
        .lookup(name)
        .filter(|value| !value.is_empty())
}
