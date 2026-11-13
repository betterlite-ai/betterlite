pub mod cache_control;
pub mod error;
pub mod policy;
pub mod stream;

mod tests;

use betterlite_models::call_helpers::headers::AdditionalHeaders;
use betterlite_models::decisions::Json as HiddenJson;
use betterlite_prompts::arguments::json::{Json, Object};
use serde_json::{Number, Value};

use crate::chat::ChatMessage;
use crate::openai_base_config::Headers;
use crate::openrouter_chat::cache_control::{
    move_cache_control_to_content, supports_cache_control,
};
use crate::openrouter_chat::error::OpenRouterError;
use crate::openrouter_chat::policy::{BaseRequest, ChatPolicy};

const OPENROUTER_ONLY_PARAMS: [&str; 2] = ["reasoning_effort", "thinking"];
const EXTRA_BODY_PARAMS: [&str; 3] = ["transforms", "models", "route"];
const COST_HEADER: &str = "llm_provider-x-litellm-response-cost";

pub struct OpenrouterConfig<P> {
    pub policy: P,
}

pub struct ChatRequest<'request> {
    pub model: &'request str,
    pub messages: &'request [ChatMessage],
    pub optional_params: &'request Object,
    pub litellm_params: &'request Object,
    pub headers: &'request Headers,
}

pub struct ChatReply {
    pub body: Value,
    pub additional_headers: AdditionalHeaders,
}

impl<P: ChatPolicy> OpenrouterConfig<P> {
    #[must_use]
    pub fn supported_params(&self, model: &str) -> Vec<&'static str> {
        let mut params = self.policy.supported_params(model);
        for extra in OPENROUTER_ONLY_PARAMS {
            if !params.contains(&extra) {
                params.push(extra);
            }
        }
        params
    }

    pub fn map_openai_params(&self, model: &str, requested: &Object, optional_params: &mut Object) {
        let mut requested = requested.clone();
        if requested.get("reasoning_effort").and_then(Json::as_str) == Some("max") {
            requested.insert(
                String::from("reasoning_effort"),
                Json::Text(String::from("xhigh")),
            );
        }
        self.policy.map_params(model, &requested, optional_params);
        let mut extra_body = Object::new();
        for name in EXTRA_BODY_PARAMS {
            let Some(value) = requested.shift_remove(name) else {
                continue;
            };
            if !matches!(value, Json::Null) {
                extra_body.insert(name.to_owned(), value);
            }
        }
        optional_params.insert(String::from("extra_body"), Json::Object(extra_body));
    }

    // LiteLLM's override skips the base removal for a model whose content keeps cache_control blocks
    pub fn remove_cache_control(
        &self,
        model: &str,
        messages: &mut Vec<ChatMessage>,
        optional_params: &mut Object,
    ) {
        if !supports_cache_control(model) {
            self.policy.remove_cache_control(messages, optional_params);
        }
    }

    pub fn transform_request(&self, request: &ChatRequest<'_>) -> Result<Object, OpenRouterError> {
        let mut messages = request.messages.to_vec();
        if supports_cache_control(request.model) {
            messages = move_cache_control_to_content(&messages);
        }
        let mut optional_params = request.optional_params.clone();
        self.remove_cache_control(request.model, &mut messages, &mut optional_params);
        let extra_body = optional_params.shift_remove("extra_body");
        let mut body = self.policy.request_body(BaseRequest {
            model: request.model,
            messages: &messages,
            optional_params: &optional_params,
            litellm_params: request.litellm_params,
            headers: request.headers,
        })?;
        match extra_body {
            None | Some(Json::Null) => {}
            Some(Json::Object(entries)) => body.extend(entries),
            Some(_) => {
                return Err(OpenRouterError {
                    message: String::from("extra_body must be a JSON object"),
                    status_code: 400,
                    headers: Headers::new(),
                });
            }
        }
        if !body.contains_key("usage") {
            let mut usage = Object::new();
            usage.insert(String::from("include"), Json::Bool(true));
            body.insert(String::from("usage"), Json::Object(usage));
        }
        Ok(body)
    }

    pub fn transform_response(
        &self,
        raw: &[u8],
        additional_headers: AdditionalHeaders,
    ) -> Result<ChatReply, OpenRouterError> {
        let body = self.policy.response_body(raw)?;
        let mut additional_headers = additional_headers;
        if let Some(cost) = reported_cost(raw) {
            additional_headers.insert(COST_HEADER.to_owned(), HiddenJson::Number(cost));
        }
        Ok(ChatReply {
            body,
            additional_headers,
        })
    }
}

// LiteLLM re-reads the raw body for usage.cost and swallows a body it cannot read
fn reported_cost(raw: &[u8]) -> Option<Number> {
    let Ok(Value::Object(body)) = serde_json::from_slice::<Value>(raw) else {
        return None;
    };
    let Value::Object(usage) = body.get("usage")? else {
        return None;
    };
    if usage.is_empty() {
        return None;
    }
    let cost = usage.get("cost")?;
    if !cost.is_number() {
        return None;
    }
    cost.as_f64().and_then(Number::from_f64)
}
