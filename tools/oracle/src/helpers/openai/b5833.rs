use std::collections::{BTreeMap, BTreeSet};

use betterlite_models::price_fields::nested::SearchContextPricing;
use betterlite_openai::chat::ChatMessage;
use betterlite_openai::chat_content::RefuseDownloads;
use betterlite_openai::compatible_wire::ReplyOptions;
use betterlite_openai::groq_chat::capabilities::{CapabilityError, GroqCapabilities};
use betterlite_openai::groq_chat::credentials::compatible_provider_info;
use betterlite_openai::groq_chat::error::GroqChatError;
use betterlite_openai::groq_chat::messages::{MessageTransform, transform_messages};
use betterlite_openai::groq_chat::options::{
    MapParams, SupportedParams, get_supported_openai_params, map_openai_params,
};
use betterlite_openai::groq_chat::reply::transform_response;
use betterlite_openai::groq_chat::stream::chunk_parser;
use betterlite_openai::groq_cost::{ServerToolUse, web_search_cost};
use betterlite_openai::groq_stt::{GroqSttError, MapParams as SttMapParams, map_openai_params_stt};
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

// every section shares one call shape, and deny_unknown_fields still rejects a key no section reads
#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Call {
    model: Option<String>,
    messages: Vec<ChatMessage>,
    environment: BTreeMap<String, String>,
    non_default: Object,
    optional_params: Object,
    drop_params: bool,
    global_drop_params: bool,
    reply: Option<Value>,
    json_mode: bool,
    chunk: Option<Value>,
    api_base: Option<String>,
    api_key: Option<String>,
    environment_base: Option<String>,
    environment_key: Option<String>,
    web_search_requests: Option<u64>,
    browser_open_requests: Option<u64>,
    medium_rate: Option<String>,
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Input {
    capabilities: Answers,
    credentials: Vec<Call>,
    supported: Vec<Call>,
    messages: Vec<Call>,
    options: Vec<Call>,
    replies: Vec<Call>,
    chunks: Vec<Call>,
    stt: Vec<Call>,
    costs: Vec<Call>,
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let answers = &input.capabilities;
        Ok(json!({
            "credentials": projected(&input.credentials, credential),
            "supported": projected(&input.supported, |call| supported_one(answers, call)),
            "messages": projected(&input.messages, message_one),
            "options": projected(&input.options, |call| options_one(answers, call)),
            "replies": projected(&input.replies, reply_one),
            "chunks": projected(&input.chunks, chunk_one),
            "stt": projected(&input.stt, stt_one),
            "costs": costs(&input.costs)?,
        }))
    }
}

fn projected<Call, Projection>(calls: &[Call], projection: Projection) -> Vec<Value>
where
    Projection: Fn(&Call) -> Value,
{
    calls.iter().map(projection).collect()
}

// a capability or stream failure has no status the Python adapter can read, so both sides project null
fn status(error: &GroqChatError) -> Value {
    match error {
        GroqChatError::Capability(_) | GroqChatError::Stream(_) => json!(null),
        other => json!(other.status_code()),
    }
}

fn failed(error: &GroqChatError) -> Value {
    json!({"error": {"status": status(error)}})
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

fn credential(call: &Call) -> Value {
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
}

fn supported_one(answers: &Answers, call: &Call) -> Value {
    get_supported_openai_params(&SupportedParams {
        model: call.model.as_deref().unwrap_or(""),
        config: &config(),
        catalog: &empty_catalog(),
        capabilities: answers,
    })
    .map_or_else(|error| failed(&error), |params| json!({"params": params}))
}

fn message_one(call: &Call) -> Value {
    let downloads = RefuseDownloads;
    let transform = MessageTransform {
        model: call.model.as_deref().unwrap_or(""),
        messages: &call.messages,
        environment: &call.environment,
        downloads: &downloads,
    };
    match transform_messages(&transform) {
        Ok(messages) => json!({"messages": messages}),
        Err(error) => failed(&error),
    }
}

fn options_one(answers: &Answers, call: &Call) -> Value {
    let config = config();
    let catalog = empty_catalog();
    let mapped = match map_openai_params(&MapParams {
        model: call.model.as_deref().unwrap_or(""),
        non_default: &call.non_default,
        optional_params: &call.optional_params,
        config: &config,
        catalog: &catalog,
        capabilities: answers,
        drop_params: call.drop_params,
    }) {
        Ok(mapped) => mapped,
        Err(error) => return failed(&error),
    };
    json!({
        "optional_params": mapped.optional_params,
        "non_default": mapped.non_default,
    })
}

fn reply_one(call: &Call) -> Value {
    let options = ReplyOptions {
        json_mode: call.json_mode,
        provider: None,
        base_model: None,
        identifier: Uuid::nil,
        clock: i64::default,
    };
    let body = call.reply.clone().unwrap_or(Value::Null);
    transform_response(body.to_string().as_bytes(), &options).map_or_else(
        |error| failed(&error),
        |converted| {
            let actions = converted.browser_actions;
            json!({
                "service_tier": converted.reply.response.service_tier,
                "web_search_requests": actions.map(|actions| actions.web_search_requests),
                "browser_open_requests": actions.map(|actions| actions.browser_open_requests),
            })
        },
    )
}

fn chunk_one(call: &Call) -> Value {
    let chunk = call.chunk.clone().unwrap_or(Value::Null);
    let chunk = match chunk_parser(&chunk, Uuid::nil, i64::default) {
        Ok(chunk) => chunk,
        Err(error) => return failed(&error),
    };
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

fn stt_one(call: &Call) -> Value {
    let params = SttMapParams {
        non_default: &call.non_default,
        optional_params: &call.optional_params,
        drop_params: call.drop_params,
        global_drop_params: call.global_drop_params,
    };
    match map_openai_params_stt(&params) {
        Ok(optional_params) => json!({"optional_params": optional_params}),
        Err(error) => stt_failed(&error),
    }
}

fn stt_failed(error: &GroqSttError) -> Value {
    json!({"error": {"status": error.status_code()}})
}

fn costs(calls: &[Call]) -> Result<Vec<Value>, Box<HelperError>> {
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
        let used_pricing = call.medium_rate.is_some().then_some(&pricing);
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
