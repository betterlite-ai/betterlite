use betterlite_config::loader::EnvironmentLookup;
use betterlite_logging::python_json;
use betterlite_models::decisions::Json as ModelJson;
use betterlite_prompts::arguments::json::{Json, Object};
use serde_json::Number;

use crate::chat::ChatMessage;
use crate::chat_content::{ChatContent, ChatContentError, Mode, UrlDownloader};
use crate::reasoning_models::DropParams;

use super::error::DeepInfraError;

pub const PROVIDER: &str = "deepinfra";
// LiteLLM's constructor declares these settings, and every one starts absent unless a caller supplies it
pub const CAPTURED_SETTINGS: &str = "frequency_penalty function_call functions logit_bias max_tokens n presence_penalty stop temperature top_p response_format tools tool_choice";
// the advertised list, in the order the pinned method appends it
pub const BASE_OPENAI_PARAMS: &str = "stream frequency_penalty function_call functions logit_bias max_tokens max_completion_tokens n presence_penalty stop temperature top_p response_format tools tool_choice";
const MISTRAL: &str = "mistralai/Mistral-7B-Instruct-v0.1";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DeepInfraChatSettings {
    settings: Object,
}

impl DeepInfraChatSettings {
    // the constructor sets an attribute only for a non-None argument, so the captured map keeps exactly what a caller supplied
    pub fn with_settings(settings: Object) -> Result<Self, DeepInfraError> {
        let unknown = settings.keys().find(|name| {
            !CAPTURED_SETTINGS
                .split_whitespace()
                .any(|known| known == name.as_str())
        });
        match unknown {
            Some(name) => Err(DeepInfraError::UnknownSetting { name: name.clone() }),
            None => Ok(Self { settings }),
        }
    }

    #[must_use]
    pub fn get_config(&self) -> Object {
        self.settings
            .iter()
            .filter(|(_, value)| !matches!(value, Json::Null))
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect()
    }
}

#[must_use]
pub fn supported_openai_params(supports_reasoning: bool) -> Vec<&'static str> {
    let mut params: Vec<&'static str> = BASE_OPENAI_PARAMS.split_whitespace().collect();
    if supports_reasoning {
        params.push("reasoning_effort");
    }
    params
}

pub struct MapRequest<'request> {
    pub model: &'request str,
    pub non_default_params: &'request Object,
    pub optional_params: &'request mut Object,
    pub drop_params: DropParams,
    pub minimum_temperature: f64,
    pub supports_reasoning: bool,
}

pub fn map_openai_params(request: MapRequest<'_>) -> Result<(), DeepInfraError> {
    let MapRequest {
        model,
        non_default_params,
        optional_params,
        drop_params,
        minimum_temperature,
        supports_reasoning,
    } = request;
    let supported = supported_openai_params(supports_reasoning);
    for (name, given) in non_default_params {
        let value = if name == "temperature" && model == MISTRAL && is_zero(given) {
            Number::from_f64(minimum_temperature)
                .map(Json::Number)
                .ok_or(DeepInfraError::UnrepresentableMinimumTemperature)?
        } else {
            given.clone()
        };
        if name == "tool_choice" {
            let tolerated = matches!(&value, Json::Text(text) if text.as_str() == "auto" || text.as_str() == "none");
            if !tolerated && !(drop_params.global || drop_params.request) {
                return Err(DeepInfraError::UnsupportedToolChoice);
            }
            continue;
        }
        if name == "max_completion_tokens" {
            optional_params.insert(String::from("max_tokens"), value);
            continue;
        }
        if supported.contains(&name.as_str()) && !matches!(value, Json::Null) {
            optional_params.insert(name.clone(), value);
        }
    }
    Ok(())
}

// Python compares the value to 0, which is true for both zero signs and no other integer or float
fn is_zero(value: &Json) -> bool {
    matches!(value, Json::Number(number) if number.as_f64().is_some_and(|parsed| parsed.to_bits() & 0x7fff_ffff_ffff_ffff == 0))
}

pub struct ChatTransform<'request> {
    pub model: &'request str,
    pub messages: &'request [ChatMessage],
    pub optional_params: &'request Object,
    pub environment: &'request dyn EnvironmentLookup,
    pub downloads: &'request dyn UrlDownloader,
}

// the parent message conversion runs first, then DeepInfra rewrites a tool message's content list
pub fn transform_request(
    request: &ChatTransform<'_>,
    mode: Mode,
) -> Result<Object, ChatContentError> {
    let content = ChatContent {
        model: request.model,
        messages: request.messages,
        optional_params: request.optional_params,
        provider: Some(PROVIDER),
        api_base: None,
        global_api_base: None,
        openai_system_messages_first: false,
        environment: request.environment,
        downloads: request.downloads,
    };
    let body = content.transform(mode)?.to_body()?;
    let mut value = as_model_json(&body)?;
    if let ModelJson::Object(fields) = &mut value
        && let Some(ModelJson::List(messages)) = fields.get_mut("messages")
    {
        for message in messages {
            tool_message_content(message);
        }
    }
    as_prompt_object(&value)
}

fn as_model_json(body: &Object) -> Result<ModelJson, ChatContentError> {
    let plain =
        serde_json::to_value(body).map_err(|source| ChatContentError::MessageShape { source })?;
    serde_json::from_value(plain).map_err(|source| ChatContentError::MessageShape { source })
}

fn as_prompt_object(value: &ModelJson) -> Result<Object, ChatContentError> {
    let plain =
        serde_json::to_value(value).map_err(|source| ChatContentError::MessageShape { source })?;
    serde_json::from_value(plain).map_err(|source| ChatContentError::MessageShape { source })
}

fn tool_message_content(message: &mut ModelJson) {
    let ModelJson::Object(fields) = message else {
        return;
    };
    if fields.get("role") != Some(&ModelJson::Text(String::from("tool"))) {
        return;
    }
    let Some(ModelJson::List(content)) = fields.get("content") else {
        return;
    };
    let single = match content.as_slice() {
        [ModelJson::Object(part)]
            if part.get("type") == Some(&ModelJson::Text(String::from("text"))) =>
        {
            part.get("text").cloned()
        }
        _ => None,
    };
    let converted = single.map_or_else(
        || ModelJson::Text(python_json::dumps(&ModelJson::List(content.clone()))),
        |value| value,
    );
    fields.insert(String::from("content"), converted);
}
