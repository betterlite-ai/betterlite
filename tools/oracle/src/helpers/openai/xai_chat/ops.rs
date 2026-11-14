use std::collections::BTreeMap;
use std::sync::PoisonError;

use betterlite_models::price_fields::decimal_lexeme::parse_decimal_lexeme;
use betterlite_openai::chat::ChatMessage;
use betterlite_openai::chat_choices::wire::StreamReply;
use betterlite_openai::chat_content::{ChatContent, RefuseDownloads};
use betterlite_openai::openai_base_config::Headers;
use betterlite_openai::xai_chat::params::{map_openai_params, supported_openai_params};
use betterlite_openai::xai_chat::request::transform_request;
use betterlite_openai::xai_chat::response::{ReplyInput, transform_response};
use betterlite_openai::xai_chat::stream::parse_chunk;
use betterlite_openai::xai_chat::{
    Environment, PROVIDER, UrlInput, complete_url, provider_info, validate_environment,
};
use betterlite_prompts::arguments::json::{Json, Object};
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::{Value, json};
use tracing::subscriber::with_default;
use tracing_subscriber::layer::SubscriberExt as _;
use uuid::Uuid;

use super::logs::Warnings;
use super::oauth::{CapabilityLookup, identity, oauth};
use super::{CHAT_FAILURE, STREAM_FAILURE, Settings, failure};
use crate::helpers::HelperResult;

pub(super) struct Run<'request> {
    pub(super) settings: &'request Settings,
    pub(super) runtime: &'request tokio::runtime::Runtime,
}

impl Run<'_> {
    pub(super) fn supported(&self, model: &str, supports_reasoning: Option<bool>) -> Value {
        let mut lookup = CapabilityLookup {
            outcome: supports_reasoning,
        };
        let params = self
            .runtime
            .block_on(supported_openai_params(model, &mut lookup));
        json!({ "supported": params })
    }

    pub(super) fn mapped(
        &self,
        model: &str,
        supports_reasoning: bool,
        params: &[(String, Json)],
        optional_params: &Object,
    ) -> HelperResult {
        let mut optional = optional_params.clone();
        let requested = Object::from_iter(params.iter().cloned());
        map_openai_params(model, supports_reasoning, &requested, &mut optional)
            .map_err(|error| failure(CHAT_FAILURE, Some(error.status_code()), error))?;
        Ok(json!({ "mapped": optional }))
    }

    pub(super) fn requested(
        &self,
        model: &str,
        messages: &[ChatMessage],
        optional_params: &Object,
    ) -> HelperResult {
        let warnings = Warnings::default();
        let subscriber = tracing_subscriber::registry().with(warnings.clone());
        let environment = BTreeMap::<String, String>::new();
        let body = with_default(subscriber, || {
            transform_request(&ChatContent {
                model,
                messages,
                optional_params,
                provider: Some(PROVIDER),
                api_base: None,
                global_api_base: None,
                openai_system_messages_first: false,
                environment: &environment,
                downloads: &RefuseDownloads,
            })
        })
        .map_err(|error| failure(CHAT_FAILURE, Some(error.status_code()), error))?;
        let count = warnings
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len();
        Ok(json!({ "body": body, "warnings": count }))
    }

    pub(super) fn environment(
        &self,
        model: &str,
        headers: &Headers,
        api_key: Option<&SecretString>,
        use_xai_oauth: bool,
    ) -> HelperResult {
        let settings = identity(self.settings);
        let mut authenticator = oauth(self.settings, None);
        let resolved = self
            .runtime
            .block_on(validate_environment(Environment {
                headers: headers.clone(),
                api_key,
                settings: &settings,
                model,
                use_xai_oauth,
                oauth: &mut authenticator,
            }))
            .map_err(|error| failure(CHAT_FAILURE, Some(error.status_code()), error))?;
        Ok(json!({ "headers": resolved }))
    }

    pub(super) fn url(
        &self,
        api_base: Option<&str>,
        api_key: Option<&SecretString>,
        use_xai_oauth: bool,
    ) -> Value {
        let settings = identity(self.settings);
        let resolved = provider_info(api_base, api_key, &settings);
        json!({
            "url": complete_url(&UrlInput {
                api_base,
                api_key,
                settings: &settings,
                use_xai_oauth,
                oauth: &oauth(self.settings, None),
            }),
            "api_base": resolved.api_base,
            "api_key": resolved.api_key.map(|key| key.expose_secret().to_owned()),
        })
    }

    pub(super) fn reply(&self, body: &str) -> HelperResult {
        let converted = transform_response(&ReplyInput {
            body: body.as_bytes(),
        })
        .map_err(|error| failure(CHAT_FAILURE, Some(error.status_code()), error))?;
        let choices: Vec<Value> = converted
            .response
            .choices
            .iter()
            .map(|choice| {
                json!({
                    "finish_reason": choice.finish_reason,
                    "provider_specific_fields": choice.provider_specific_fields,
                    "tool_calls": choice.message.tool_calls.as_ref().map_or(0, Vec::len),
                })
            })
            .collect();
        let usage = &converted.usage;
        Ok(json!({
            "choices": choices,
            "usage": {
                "prompt_tokens": usage.tokens.prompt_tokens,
                "completion_tokens": usage.tokens.completion_tokens,
                "total_tokens": usage.tokens.total_tokens,
                "cost": usage.cost.map(|cost| cost.normalize().to_string()),
                "web_search_requests": usage.tokens.prompt_details.and_then(|details| details.web_search_requests),
            },
        }))
    }

    pub(super) fn chunk(&self, raw: &Value) -> HelperResult {
        let mut chunk = raw.clone();
        let reply = parse_chunk(&mut chunk, Uuid::nil, || 0)
            .map_err(|error| failure(STREAM_FAILURE, None, error))?;
        let choices: Vec<Value> = reply
            .choices
            .iter()
            .map(|choice| {
                json!({
                    "index": choice.index,
                    "finish_reason": choice.finish_reason,
                })
            })
            .collect();
        Ok(json!({ "choices": choices, "usage": stream_usage(&reply) }))
    }
}

fn decimal_text(value: Option<&Value>) -> Option<String> {
    let Value::Number(number) = value? else {
        return None;
    };
    let decimal = parse_decimal_lexeme(&number.to_string()).ok()?;
    Some(decimal.normalize().to_string())
}

fn stream_usage(reply: &StreamReply) -> Value {
    let Some(Value::Object(fields)) = reply.usage.as_ref() else {
        return Value::Null;
    };
    json!({
        "prompt_tokens": fields.get("prompt_tokens"),
        "completion_tokens": fields.get("completion_tokens"),
        "total_tokens": fields.get("total_tokens"),
        "cost": decimal_text(fields.get("cost")),
        "web_search_requests": fields
            .get("prompt_tokens_details")
            .and_then(|details| details.get("web_search_requests")),
    })
}
