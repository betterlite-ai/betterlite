use std::collections::BTreeSet;

use betterlite_openai::chat::ChatMessage;
use betterlite_openai::openai_base_config::{
    CatalogCapabilities, ConfigFamily, Headers, OpenAiConfig, OpenAiSettings,
};
use betterlite_openai::openrouter_chat::OpenrouterConfig;
use betterlite_openai::openrouter_chat::error::{OpenRouterError, get_error_class};
use betterlite_openai::openrouter_chat::policy::{BaseRequest, ChatPolicy};
use betterlite_openai::openrouter_chat::stream::chunk_parser;
use betterlite_prompts::arguments::json::{Json, Object};
use serde_json::{Map, Value, json};

use crate::helpers::{HelperResult, OrHelperError as _};

use super::input::{ChatParamsCase, ChunkCase, ErrorCase};

// B668, B503 and B504 supply these calls at composition; the helper only needs the supported list and the copy
struct OpenAiPolicy;

impl ChatPolicy for OpenAiPolicy {
    fn supported_params(&self, model: &str) -> Vec<&'static str> {
        openai_config().supported_params(model, &empty_catalog())
    }

    fn map_params(&self, model: &str, requested: &Object, optional_params: &mut Object) {
        let supported = self.supported_params(model);
        for (name, value) in requested {
            if supported.contains(&name.as_str()) {
                optional_params.insert(name.clone(), value.clone());
            }
        }
    }

    fn remove_cache_control(
        &self,
        _messages: &mut Vec<ChatMessage>,
        _optional_params: &mut Object,
    ) {
    }

    fn request_body(&self, _request: BaseRequest<'_>) -> Result<Object, OpenRouterError> {
        Err(unavailable())
    }

    fn response_body(&self, _raw: &[u8]) -> Result<Value, OpenRouterError> {
        Err(unavailable())
    }
}

fn unavailable() -> OpenRouterError {
    OpenRouterError {
        message: String::from("this helper operation does not build a request"),
        status_code: 500,
        headers: Headers::new(),
    }
}

// ConfigFamily::Other keeps the OpenAIConfig-only unknown-model reasoning_effort out of the base list
fn openai_config() -> OpenAiConfig {
    OpenAiConfig {
        family: ConfigFamily::Other,
        api_key: None,
        headers: Headers::new(),
        settings: OpenAiSettings::default(),
    }
}

fn empty_catalog() -> CatalogCapabilities {
    CatalogCapabilities {
        chat_models: BTreeSet::new(),
        text_models: BTreeSet::new(),
    }
}

pub(super) fn config() -> OpenrouterConfig<impl ChatPolicy> {
    OpenrouterConfig {
        policy: OpenAiPolicy,
    }
}

pub(super) fn prompt_object(fields: &Map<String, Value>) -> Object {
    fields
        .iter()
        .map(|(name, value)| (name.clone(), prompt_json(value)))
        .collect()
}

fn prompt_json(value: &Value) -> Json {
    match value {
        Value::Null => Json::Null,
        Value::Bool(flag) => Json::Bool(*flag),
        Value::Number(number) => Json::Number(number.clone()),
        Value::String(text) => Json::Text(text.clone()),
        Value::Array(items) => Json::List(items.iter().map(prompt_json).collect()),
        Value::Object(fields) => Json::Object(prompt_object(fields)),
    }
}

pub(super) fn chat_params(cases: &[ChatParamsCase]) -> HelperResult {
    let config = config();
    let mut mapped = Vec::with_capacity(cases.len());
    for case in cases {
        let mut optional_params = Object::new();
        config.map_openai_params(
            &case.model,
            &prompt_object(&case.requested),
            &mut optional_params,
        );
        mapped.push(serde_json::to_value(&optional_params).or_value_error()?);
    }
    Ok(Value::Array(mapped))
}

pub(super) fn chunks(cases: &[ChunkCase]) -> Value {
    Value::Array(cases.iter().map(chunk).collect())
}

fn chunk(case: &ChunkCase) -> Value {
    let reply = match chunk_parser(&case.chunk) {
        Ok(reply) => reply,
        Err(error) => {
            return json!({"error": {"message": error.message, "status": error.status_code}});
        }
    };
    let choices: Vec<Value> = reply
        .choices
        .iter()
        .map(|choice| {
            let mut delta = Map::new();
            delta.insert(String::from("content"), json!(choice.delta.content));
            if let Some(reasoning) = &choice.delta.reasoning_content {
                delta.insert(String::from("reasoning_content"), json!(reasoning));
            }
            json!({"index": choice.index, "delta": delta})
        })
        .collect();
    json!({
        "id": reply.id,
        "object": reply.object,
        "created": reply.created,
        "model": reply.model,
        "choices": choices,
    })
}

pub(super) fn error_class(case: &ErrorCase) -> Value {
    let error = get_error_class(&case.message, case.status, case.headers.clone());
    json!({"message": error.message, "status": error.status_code})
}
