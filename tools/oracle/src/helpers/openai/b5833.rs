use std::collections::{BTreeMap, BTreeSet};

use betterlite_models::price_fields::nested::SearchContextPricing;
use betterlite_openai::chat::ChatMessage;
use betterlite_openai::chat_content::RefuseDownloads;
use betterlite_openai::compatible_wire::ReplyOptions;
use betterlite_openai::groq_chat::capabilities::{CapabilityError, GroqCapabilities};
use betterlite_openai::groq_chat::credentials::compatible_provider_info;
use betterlite_openai::groq_chat::error::GroqChatError;
use betterlite_openai::groq_chat::messages::{RequestTransform, transform_request};
use betterlite_openai::groq_chat::options::{
    MapParams, SupportedParams, get_supported_openai_params, map_openai_params,
};
use betterlite_openai::groq_chat::reply::transform_response;
use betterlite_openai::groq_chat::stream::chunk_parser;
use betterlite_openai::groq_cost::{ServerToolUse, web_search_cost};
use betterlite_openai::groq_stt::{MapParams as SttMapParams, map_openai_params_stt};
use betterlite_openai::openai_base_config::{
    CatalogCapabilities, ConfigFamily, Headers, Object, OpenAiConfig, OpenAiSettings,
};
use rust_decimal::Decimal;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::helpers::{HelperError, HelperResult};

pub(crate) struct Op;

#[derive(Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Answer {
    Yes,
    #[default]
    No,
    Fail,
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Answers {
    web_search: Answer,
    reasoning: Answer,
    response_schema: Answer,
}

fn lookup(answer: Answer, name: &'static str, model: &str) -> Result<bool, CapabilityError> {
    match answer {
        Answer::Yes => Ok(true),
        Answer::No => Ok(false),
        Answer::Fail => Err(CapabilityError {
            name,
            model: model.to_owned(),
            message: String::from("the capability lookup failed"),
        }),
    }
}

impl GroqCapabilities for Answers {
    fn supports_web_search(&self, model: &str, _provider: &str) -> Result<bool, CapabilityError> {
        lookup(self.web_search, "web search", model)
    }

    fn supports_reasoning(&self, model: &str, _provider: &str) -> Result<bool, CapabilityError> {
        lookup(self.reasoning, "reasoning", model)
    }

    fn supports_response_schema(
        &self,
        model: &str,
        _provider: &str,
    ) -> Result<bool, CapabilityError> {
        lookup(self.response_schema, "response schema", model)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CredentialCall {
    api_base: Option<String>,
    api_key: Option<String>,
    environment_base: Option<String>,
    environment_key: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SupportedCall {
    model: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MessageCall {
    model: String,
    messages: Vec<ChatMessage>,
    #[serde(default)]
    environment: BTreeMap<String, String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OptionCall {
    model: String,
    #[serde(default)]
    non_default: Object,
    #[serde(default)]
    optional_params: Object,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReplyCall {
    reply: Value,
    #[serde(default)]
    json_mode: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ChunkCall {
    chunk: Value,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SttCall {
    #[serde(default)]
    non_default: Object,
    #[serde(default)]
    optional_params: Object,
    #[serde(default)]
    drop_params: bool,
    #[serde(default)]
    global_drop_params: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CostCall {
    web_search_requests: Option<u64>,
    browser_open_requests: Option<u64>,
    medium_rate: Option<String>,
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Input {
    capabilities: Answers,
    credentials: Vec<CredentialCall>,
    supported: Vec<SupportedCall>,
    messages: Vec<MessageCall>,
    options: Vec<OptionCall>,
    replies: Vec<ReplyCall>,
    chunks: Vec<ChunkCall>,
    stt: Vec<SttCall>,
    costs: Vec<CostCall>,
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        Ok(json!({
            "credentials": credentials(&input.credentials),
            "supported": supported(&input.capabilities, &input.supported),
            "messages": messages(&input.messages),
            "options": options(&input.capabilities, &input.options),
            "replies": replies(&input.replies),
            "chunks": chunks(&input.chunks),
            "stt": stt(&input.stt),
            "costs": costs(&input.costs)?,
        }))
    }
}

fn credentials(calls: &[CredentialCall]) -> Vec<Value> {
    calls
        .iter()
        .map(|call| {
            let key = call.api_key.as_deref().map(SecretString::from);
            let environment_key = call.environment_key.as_deref().map(SecretString::from);
            let (api_base, api_key) = compatible_provider_info(
                call.api_base.as_deref(),
                key.as_ref(),
                call.environment_base.as_deref(),
                environment_key.as_ref(),
            );
            json!({
                "api_base": api_base,
                "api_key": api_key.map(|key| key.expose_secret().to_owned()),
            })
        })
        .collect()
}

fn config() -> OpenAiConfig {
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

// a capability or stream failure has no status the Python adapter can read, so both sides project null
fn status(error: &GroqChatError) -> Value {
    match error {
        GroqChatError::Capability(_) | GroqChatError::Stream(_) => json!(null),
        other => json!(other.status_code()),
    }
}

fn supported(answers: &Answers, calls: &[SupportedCall]) -> Vec<Value> {
    let config = config();
    let catalog = empty_catalog();
    calls
        .iter()
        .map(|call| {
            match get_supported_openai_params(&SupportedParams {
                model: &call.model,
                config: &config,
                catalog: &catalog,
                capabilities: answers,
            }) {
                Ok(params) => json!({"params": params}),
                Err(error) => json!({"error": {"status": status(&error)}}),
            }
        })
        .collect()
}

fn messages(calls: &[MessageCall]) -> Vec<Value> {
    let downloads = RefuseDownloads;
    calls
        .iter()
        .map(|call| {
            let empty = Object::new();
            let result = transform_request(&RequestTransform {
                model: &call.model,
                messages: &call.messages,
                optional_params: &empty,
                environment: &call.environment,
                downloads: &downloads,
            });
            match result {
                Ok(body) => json!({"messages": body.get("messages").cloned().unwrap_or(Value::Null)}),
                Err(error) => json!({"error": {"status": status(&error)}}),
            }
        })
        .collect()
}

fn options(answers: &Answers, calls: &[OptionCall]) -> Vec<Value> {
    let config = config();
    let catalog = empty_catalog();
    calls
        .iter()
        .map(|call| {
            match map_openai_params(&MapParams {
                model: &call.model,
                non_default: &call.non_default,
                optional_params: &call.optional_params,
                config: &config,
                catalog: &catalog,
                capabilities: answers,
                drop_params: false,
            }) {
                Ok(mapped) => json!({
                    "optional_params": mapped.optional_params,
                    "non_default": mapped.non_default,
                }),
                Err(error) => json!({"error": {"status": status(&error)}}),
            }
        })
        .collect()
}

fn replies(calls: &[ReplyCall]) -> Vec<Value> {
    calls
        .iter()
        .map(|call| {
            let options = ReplyOptions {
                json_mode: call.json_mode,
                provider: None,
                base_model: None,
                identifier: Uuid::nil,
                clock: i64::default,
            };
            match transform_response(call.reply.to_string().as_bytes(), &options) {
                Ok(converted) => {
                    let actions = converted.browser_actions;
                    json!({
                        "service_tier": converted.reply.response.service_tier,
                        "web_search_requests": actions.map(|actions| actions.web_search_requests),
                        "browser_open_requests": actions.map(|actions| actions.browser_open_requests),
                    })
                }
                Err(error) => json!({"error": {"status": status(&error)}}),
            }
        })
        .collect()
}

fn chunks(calls: &[ChunkCall]) -> Vec<Value> {
    calls
        .iter()
        .map(|call| match chunk_parser(&call.chunk, Uuid::nil, i64::default) {
            Ok(chunk) => {
                let choices: Vec<Value> = chunk
                    .choices
                    .iter()
                    .map(|choice| {
                        json!({
                            "index": choice.index,
                            "finish_reason": choice.finish_reason,
                            "content": choice.delta.content,
                            "reasoning_content": choice.delta.reasoning_content,
                        })
                    })
                    .collect();
                json!({
                    "id": chunk.id,
                    "object": chunk.object,
                    "model": chunk.model,
                    "choices": choices,
                })
            }
            Err(error) => json!({"error": {"status": status(&error)}}),
        })
        .collect()
}

fn stt(calls: &[SttCall]) -> Vec<Value> {
    calls
        .iter()
        .map(|call| {
            match map_openai_params_stt(&SttMapParams {
                non_default: &call.non_default,
                optional_params: &call.optional_params,
                drop_params: call.drop_params,
                global_drop_params: call.global_drop_params,
            }) {
                Ok(optional_params) => json!({"optional_params": optional_params}),
                Err(error) => json!({"error": {"status": error.status_code()}}),
            }
        })
        .collect()
}

fn costs(calls: &[CostCall]) -> Result<Vec<Value>, Box<HelperError>> {
    let mut values = Vec::with_capacity(calls.len());
    for call in calls {
        let rate = match call.medium_rate.as_deref() {
            None => None,
            Some(text) => Some(parse_rate(text)?),
        };
        let pricing = SearchContextPricing {
            search_context_size_low: None,
            search_context_size_medium: rate,
            search_context_size_high: None,
        };
        let usage = ServerToolUse {
            web_search_requests: call.web_search_requests,
            browser_open_requests: call.browser_open_requests,
        };
        let used_pricing = if call.medium_rate.is_some() {
            Some(&pricing)
        } else {
            None
        };
        let cost = web_search_cost(Some(&usage), used_pricing).map_err(|error| {
            Box::new(HelperError {
                class: String::from("CostFailure"),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: error.to_string(),
            })
        })?;
        values.push(json!({"cost": format!("{cost:.12}")}));
    }
    Ok(values)
}

fn parse_rate(text: &str) -> Result<Decimal, Box<HelperError>> {
    text.parse::<Decimal>().map_err(|_error| {
        Box::new(HelperError {
            class: String::from("InvalidRate"),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: format!("the medium rate is not a decimal: {text}"),
        })
    })
}

mod tests;
