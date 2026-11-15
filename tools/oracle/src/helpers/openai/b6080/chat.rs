use std::collections::BTreeSet;

use betterlite_openai::chat::ChatMessage;
use betterlite_openai::chat_choices::wire::{DeltaToolCall, StreamChoice};
use betterlite_openai::openai_base_config::{
    CatalogCapabilities, ConfigFamily, Headers, OpenAiConfig, OpenAiSettings,
};
use betterlite_openai::openrouter_chat::OpenrouterConfig;
use betterlite_openai::openrouter_chat::error::OpenRouterError;
use betterlite_openai::openrouter_chat::policy::{BaseRequest, ChatPolicy};
use betterlite_openai::openrouter_chat::stream::chunk_parser;
use betterlite_prompts::arguments::json::{Json, Object};
use serde_json::{Map, Value, json};

use crate::helpers::{HelperResult, OrHelperError as _};

use super::input::{ChatParamsCase, ChunkCase};

// B668, B503 and B504 supply these calls at composition; the helper keeps the supported list, the base request fields and the parsed body
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

    fn request_body(&self, request: BaseRequest<'_>) -> Result<Object, OpenRouterError> {
        let messages = serde_json::to_value(request.messages)
            .and_then(serde_json::from_value::<Json>)
            .map_err(|error| failure(&error))?;
        let mut body = Object::new();
        body.insert(String::from("model"), Json::Text(request.model.to_owned()));
        body.insert(String::from("messages"), messages);
        body.extend(request.optional_params.clone());
        Ok(body)
    }

    fn response_body(&self, raw: &[u8]) -> Result<Value, OpenRouterError> {
        serde_json::from_slice(raw).map_err(|error| failure(&error))
    }
}

fn failure(error: &serde_json::Error) -> OpenRouterError {
    OpenRouterError {
        message: error.to_string(),
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

pub(super) fn chat_params(cases: &[ChatParamsCase]) -> HelperResult {
    let config = config();
    let mut mapped = Vec::with_capacity(cases.len());
    for case in cases {
        let mut optional_params = Object::new();
        config.map_openai_params(&case.model, &case.requested, &mut optional_params);
        mapped.push(serde_json::to_value(&optional_params).or_value_error()?);
    }
    Ok(Value::Array(mapped))
}

pub(super) fn chunk(case: &ChunkCase) -> Value {
    let reply = match chunk_parser(&case.chunk) {
        Ok(reply) => reply,
        Err(error) => {
            return json!({"error": {"message": error.message, "status": error.status_code}});
        }
    };
    let choices: Vec<Value> = reply.choices.iter().map(choice_json).collect();
    json!({
        "id": reply.id,
        "object": reply.object,
        "created": reply.created,
        "model": reply.model,
        "choices": choices,
        "usage": reply.usage,
    })
}

fn choice_json(choice: &StreamChoice) -> Value {
    let mut delta = Map::new();
    delta.insert(String::from("content"), json!(choice.delta.content));
    if let Some(reasoning) = &choice.delta.reasoning_content {
        delta.insert(String::from("reasoning_content"), json!(reasoning));
    }
    if let Some(reasoning) = choice.delta.passthrough.get("reasoning") {
        delta.insert(String::from("reasoning"), reasoning.clone());
    }
    let calls: Option<Vec<Value>> = choice
        .delta
        .tool_calls
        .as_ref()
        .map(|calls| calls.iter().map(tool_call_json).collect());
    delta.insert(String::from("tool_calls"), json!(calls));
    json!({
        "index": choice.index,
        "finish_reason": choice.finish_reason,
        "delta": delta,
    })
}

fn tool_call_json(call: &DeltaToolCall) -> Value {
    json!({
        "index": call.index,
        "type": call.kind,
        "id": call.id.clone().flatten(),
        "function": call.function.as_ref().map(|function| json!({
            "name": function.name,
            "arguments": function.arguments,
        })),
    })
}
