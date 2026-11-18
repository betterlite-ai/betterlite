use std::error::Error;

use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::error::{SpeechBridgeError, SpeechTransformError};
use super::{GEMINI_TTS_CHAT_AUDIO_FORMAT, GEMINI_TTS_RESPONSE_FORMATS, is_gemini_tts_model};

// LiteLLM's OPENAI_CHAT_COMPLETION_PARAMS; it lists temperature twice, and the projection drops response_format
const OPENAI_CHAT_COMPLETION_PARAMS: &str = "functions function_call temperature top_p n stream stream_options stop max_completion_tokens modalities prediction audio max_tokens presence_penalty frequency_penalty logit_bias user request_timeout api_base api_version api_key deployment_id organization base_url default_headers timeout response_format seed tools tool_choice max_retries parallel_tool_calls logprobs top_logprobs reasoning_effort extra_headers thinking web_search_options include_server_side_tool_invocations service_tier prompt_cache_key prompt_cache_retention safety_identifier verbosity store";

// the caller's speech kwargs before validation: each one is absent or a JSON value, so a wrong type fails like Python's isinstance
pub struct SpeechBridgeArguments<'call> {
    pub model: Option<&'call Json>,
    pub input: Option<&'call Json>,
    pub voice: Option<&'call Json>,
    pub optional_params: Option<&'call Json>,
    pub litellm_params: Option<&'call Json>,
    pub headers: Option<&'call Json>,
    pub custom_llm_provider: Option<&'call Json>,
}

// the same arguments after validation; LiteLLM checks headers and logging_obj again after this point, with no extra behaviour
pub(crate) struct ValidatedArguments<'call> {
    pub(crate) model: &'call str,
    pub(crate) input: &'call str,
    pub(crate) voice: Option<&'call Json>,
    pub(crate) optional_params: &'call IndexMap<String, Json>,
    pub(crate) litellm_params: &'call IndexMap<String, Json>,
    pub(crate) headers: &'call IndexMap<String, Json>,
    pub(crate) custom_llm_provider: &'call str,
    pub(crate) logging: &'call mut LoggingState,
}

impl<'call> SpeechBridgeArguments<'call> {
    pub(crate) fn validate<Source>(
        self,
        logging: Option<&'call mut LoggingState>,
    ) -> Result<ValidatedArguments<'call>, SpeechBridgeError<Source>>
    where
        Source: Error + 'static,
    {
        let model = argument_text("model", self.model)?;
        let custom_llm_provider = argument_text("custom_llm_provider", self.custom_llm_provider)?;
        let input = argument_text("input", self.input)?;
        let optional_params = argument_object("optional_params", self.optional_params)?;
        let litellm_params = argument_object("litellm_params", self.litellm_params)?;
        let headers = argument_object("headers", self.headers)?;
        let logging = logging.ok_or(SpeechBridgeError::Required {
            name: "logging_obj",
        })?;
        Ok(ValidatedArguments {
            model,
            input,
            voice: self.voice,
            optional_params,
            litellm_params,
            headers,
            custom_llm_provider,
            logging,
        })
    }
}

const fn argument_text<'value, Source>(
    name: &'static str,
    value: Option<&'value Json>,
) -> Result<&'value str, SpeechBridgeError<Source>>
where
    Source: Error + 'static,
{
    match value {
        Some(Json::Text(text)) => Ok(text),
        None
        | Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(SpeechBridgeError::Required { name })
        }
    }
}

const fn argument_object<'value, Source>(
    name: &'static str,
    value: Option<&'value Json>,
) -> Result<&'value IndexMap<String, Json>, SpeechBridgeError<Source>>
where
    Source: Error + 'static,
{
    match value {
        Some(Json::Object(object)) => Ok(object),
        None
        | Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            Err(SpeechBridgeError::Required { name })
        }
    }
}

impl<'call> ValidatedArguments<'call> {
    // the format the caller asked for, read from the original options even after litellm_params overrides them
    pub(crate) fn response_format(&self) -> Option<&'call str> {
        match self.optional_params.get("response_format") {
            Some(Json::Text(format)) => Some(format),
            Some(
                Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_),
            )
            | None => None,
        }
    }

    pub(crate) fn project(&self) -> Result<IndexMap<String, Json>, SpeechTransformError> {
        validate_response_format(self.model, self.optional_params)?;
        let mut kwargs = IndexMap::new();
        kwargs.insert(String::from("model"), Json::Text(self.model.to_owned()));
        kwargs.insert(String::from("messages"), user_message(self.input));
        kwargs.insert(
            String::from("modalities"),
            Json::List(vec![Json::Text(String::from("audio"))]),
        );
        insert_chat_options(self.optional_params, &mut kwargs);
        // the bridge's own audio replaces an optional_params one, and LiteLLM drops the key when it is absent
        let audio =
            chat_audio_param(self.model, self.voice, self.optional_params).unwrap_or(Json::Null);
        kwargs.insert(String::from("audio"), audio);
        kwargs.extend(
            self.litellm_params
                .iter()
                .map(|(name, value)| (name.clone(), value.clone())),
        );
        kwargs.insert(String::from("headers"), Json::Object(self.headers.clone()));
        kwargs.insert(
            String::from("custom_llm_provider"),
            Json::Text(self.custom_llm_provider.to_owned()),
        );
        kwargs.retain(|name, value| name != "litellm_logging_obj" && !matches!(value, Json::Null));
        Ok(kwargs)
    }
}

fn user_message(input: &str) -> Json {
    Json::Object(IndexMap::from([
        (String::from("role"), Json::Text(String::from("user"))),
        (String::from("content"), Json::Text(input.to_owned())),
    ]))
}

fn insert_chat_options(
    optional_params: &IndexMap<String, Json>,
    kwargs: &mut IndexMap<String, Json>,
) {
    for (name, value) in optional_params {
        if name != "response_format"
            && OPENAI_CHAT_COMPLETION_PARAMS
                .split(' ')
                .any(|allowed| allowed == name)
        {
            kwargs.insert(name.clone(), value.clone());
        }
    }
}

fn chat_audio_param(
    model: &str,
    voice: Option<&Json>,
    optional_params: &IndexMap<String, Json>,
) -> Option<Json> {
    let Json::Text(voice) = voice? else {
        return None;
    };
    let mut audio = IndexMap::new();
    audio.insert(String::from("voice"), Json::Text(voice.clone()));
    if let Some(format) = chat_audio_format(model, optional_params) {
        audio.insert(String::from("format"), Json::Text(format.to_owned()));
    }
    Some(Json::Object(audio))
}

fn chat_audio_format<'params>(
    model: &str,
    optional_params: &'params IndexMap<String, Json>,
) -> Option<&'params str> {
    if is_gemini_tts_model(model) {
        return Some(GEMINI_TTS_CHAT_AUDIO_FORMAT);
    }
    match optional_params.get("response_format") {
        Some(Json::Text(format)) => Some(format),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_))
        | None => None,
    }
}

fn validate_response_format(
    model: &str,
    optional_params: &IndexMap<String, Json>,
) -> Result<(), SpeechTransformError> {
    if !is_gemini_tts_model(model) {
        return Ok(());
    }
    match optional_params.get("response_format") {
        Some(Json::Text(format)) if !GEMINI_TTS_RESPONSE_FORMATS.contains(&format.as_str()) => {
            Err(SpeechTransformError::UnsupportedResponseFormat {
                response_format: format.clone(),
            })
        }
        Some(Json::Text(_) | Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_))
        | Some(Json::Object(_))
        | None => Ok(()),
    }
}
