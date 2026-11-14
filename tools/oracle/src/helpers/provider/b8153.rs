use std::collections::BTreeMap;
use std::sync::Mutex;

use async_trait::async_trait;
use betterlite_openai::openai_base_config::Object;
use betterlite_openai::passthrough::transformation::types::{
    PassthroughTarget, PassthroughUrlRequest,
};
use betterlite_prompts::arguments::json::Json;
use betterlite_prompts::builtin::Message;
use betterlite_prompts::factory::PromptFactory;
use betterlite_prompts::factory::hub::{Hub, builtin_entries};
use betterlite_provider::petals::config::{
    PetalsConfig, REQUEST_HEADERS, SUPPORTED_OPENAI_PARAMS, map_openai_params,
};
use betterlite_provider::petals::error::get_error_class as petals_error_class;
use betterlite_provider::petals::reply::{ReplyInput, TokenCounter, transform_response};
use betterlite_provider::petals::request::{CustomPrompt, RequestBody, RequestContext};
use betterlite_provider::vllm::error::VllmSetupError;
use betterlite_provider::vllm::model_info::VllmModelInfo;
use betterlite_provider::vllm::passthrough::VllmPassthroughConfig;
use chrono::DateTime;
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::{Helper, HelperError, HelperResult};

const FIXED_NOW: i64 = 1_700_000_000;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct InfoInput {
    calls: Vec<InfoCall>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InfoCall {
    model: String,
    #[serde(default)]
    api_base: Option<String>,
    #[serde(default)]
    env: BTreeMap<String, String>,
    #[serde(default)]
    api_key: Option<String>,
    #[serde(default)]
    headers: BTreeMap<String, String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PassInput {
    calls: Vec<PassCall>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PassCall {
    endpoint: String,
    #[serde(default)]
    api_base: Option<String>,
    #[serde(default)]
    env: BTreeMap<String, String>,
    #[serde(default)]
    query: Option<Object>,
    #[serde(default)]
    request_data: Object,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ParamsInput {
    cases: Vec<MapCase>,
    validate: Vec<ValidateCase>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MapCase {
    non_default: Object,
    #[serde(default)]
    optional: Object,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ValidateCase {
    #[serde(default)]
    headers: BTreeMap<String, String>,
    #[serde(default)]
    api_key: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RequestInput {
    #[serde(default)]
    custom_prompt_dict: BTreeMap<String, CustomPrompt>,
    requests: Vec<RequestCase>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RequestCase {
    model: String,
    messages: Vec<Message>,
    #[serde(default)]
    api_base: Option<String>,
    #[serde(default)]
    max_tokens: Option<Json>,
    #[serde(default)]
    temperature: Option<Json>,
    #[serde(default)]
    top_p: Option<Json>,
    #[serde(default)]
    stream: Option<Json>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReplyCases {
    replies: Vec<ReplyCase>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReplyCase {
    model: String,
    body: String,
    #[serde(default = "default_status")]
    status: u16,
    #[serde(default)]
    headers: BTreeMap<String, String>,
    #[serde(default)]
    messages: Vec<Message>,
}

const fn default_status() -> u16 {
    200
}

#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub(crate) enum Input {
    VllmModelInfo(InfoInput),
    VllmPassthrough(PassInput),
    PetalsParams(ParamsInput),
    PetalsRequest(RequestInput),
    PetalsReply(ReplyCases),
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::VllmModelInfo(info) => Ok(model_info_values(info)),
            Input::VllmPassthrough(pass) => Ok(passthrough_values(pass)),
            Input::PetalsParams(params) => Ok(params_value(params)),
            Input::PetalsRequest(request) => request_values(request),
            Input::PetalsReply(replies) => reply_values(replies),
        }
    }
}

fn setup_result(result: Result<impl serde::Serialize, VllmSetupError>) -> Value {
    match result {
        Ok(value) => json!({"ok": true, "value": value}),
        Err(error) => json!({"ok": false, "message": error.to_string()}),
    }
}

fn model_info_value(call: &InfoCall) -> Value {
    let info = VllmModelInfo {
        environment: &call.env,
    };
    let mut headers = call.headers.clone();
    let key = call
        .api_key
        .as_ref()
        .map(|value| SecretString::from(value.clone()));
    info.validate_environment(&mut headers, key.as_ref());
    let error = info.get_error_class("boom", 418, &BTreeMap::new());
    json!({
        "model": call.model,
        "api_base": setup_result(info.get_api_base(call.api_base.as_deref())),
        "models": setup_result(info.get_models(call.api_base.as_deref())),
        "headers": headers,
        "error": {"status": error.status(), "message": error.to_string()},
    })
}

fn model_info_values(input: &InfoInput) -> Value {
    let calls: Vec<Value> = input.calls.iter().map(model_info_value).collect();
    json!({"calls": calls})
}

fn passthrough_value(call: &PassCall) -> Value {
    let config = VllmPassthroughConfig {
        environment: &call.env,
    };
    let litellm_params = Object::new();
    let request = PassthroughUrlRequest {
        api_base: call.api_base.as_deref(),
        api_key: None,
        target: PassthroughTarget {
            model: "m",
            custom_llm_provider: "vllm",
            endpoint: &call.endpoint,
        },
        request_query_params: call.query.as_ref(),
        litellm_params: &litellm_params,
    };
    let result = match config.get_complete_url(&request) {
        Ok((url, base)) => json!({"ok": true, "url": url.as_str(), "base": base}),
        Err(error) => json!({"ok": false, "message": error.to_string()}),
    };
    json!({
        "streaming": config.is_streaming_request(&call.endpoint, &call.request_data),
        "result": result,
    })
}

fn passthrough_values(input: &PassInput) -> Value {
    let calls: Vec<Value> = input.calls.iter().map(passthrough_value).collect();
    json!({"calls": calls})
}

fn params_value(input: &ParamsInput) -> Value {
    let config = PetalsConfig::default();
    let mapped: Vec<Value> = input
        .cases
        .iter()
        .map(|case| {
            let mut options = case.optional.clone();
            map_openai_params(&case.non_default, &mut options);
            json!({"options": options})
        })
        .collect();
    let validated: Vec<Value> = input
        .validate
        .iter()
        .map(|case| {
            json!({
                "headers": REQUEST_HEADERS,
                "given": case.headers,
                "api_key": case.api_key,
            })
        })
        .collect();
    let error = petals_error_class("boom", 418, &BTreeMap::new());
    json!({
        "config": config.get_config(),
        "supported": SUPPORTED_OPENAI_PARAMS,
        "mapped": mapped,
        "validated": validated,
        "error": {"status": error.status_code, "message": error.message},
    })
}

fn factory() -> PromptFactory {
    PromptFactory {
        cache: Mutex::new(builtin_entries()),
        hub: Hub::Unreachable,
    }
}

fn runtime() -> Result<tokio::runtime::Runtime, Box<HelperError>> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|source| {
            Box::new(HelperError {
                class: String::from("Runtime"),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: source.to_string(),
            })
        })
}

fn request_values(input: &RequestInput) -> HelperResult {
    let runtime = runtime()?;
    let config = PetalsConfig::default();
    let prompts = factory();
    let context = RequestContext {
        config: &config,
        prompts: &prompts,
        custom_prompts: &input.custom_prompt_dict,
        now: DateTime::from_timestamp(FIXED_NOW, 0).unwrap_or_default(),
    };
    let mut bodies: Vec<Value> = Vec::new();
    for request in &input.requests {
        let entry = one_request(&runtime, &context, request)?;
        bodies.push(entry);
    }
    Ok(json!({"requests": bodies}))
}

fn one_request(
    runtime: &tokio::runtime::Runtime,
    context: &RequestContext<'_>,
    request: &RequestCase,
) -> HelperResult {
    let mut options = Object::new();
    map_openai_params(&openai_params(request), &mut options);
    match runtime.block_on(context.transform_request(
        &request.model,
        &request.messages,
        request.api_base.as_deref(),
        &options,
    )) {
        Ok(body) => Ok(json!({"ok": true, "form": form(&body)?})),
        Err(error) => Ok(json!({
            "ok": false,
            "status": error.status(),
            "message": error.to_string(),
        })),
    }
}

fn openai_params(request: &RequestCase) -> Object {
    let mut params = Object::new();
    let fields = [
        ("max_tokens", &request.max_tokens),
        ("temperature", &request.temperature),
        ("top_p", &request.top_p),
        ("stream", &request.stream),
    ];
    for (name, value) in fields {
        if let Some(value) = value {
            params.insert(String::from(name), value.clone());
        }
    }
    params
}

fn form(body: &RequestBody) -> Result<Object, Box<HelperError>> {
    let mut form = Object::new();
    form.insert(String::from("model"), Json::Text(body.model.clone()));
    form.insert(
        String::from("inputs"),
        Json::Text(body.inputs.clone().unwrap_or_default()),
    );
    for (name, value) in &body.parameters {
        let Some(text) = form_text(value) else {
            return Err(Box::new(HelperError {
                class: String::from("ValueError"),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: format!("the parameter {name} is not a form value"),
            }));
        };
        form.insert(name.clone(), Json::Text(text));
    }
    Ok(form)
}

fn form_text(value: &Json) -> Option<String> {
    match value {
        Json::Null => Some(String::new()),
        Json::Bool(flag) => Some(flag.to_string()),
        Json::Number(number) => Some(number.to_string()),
        Json::Text(text) => Some(text.clone()),
        Json::Array(_) | Json::Object(_) => None,
    }
}

struct StandInCounter;

#[async_trait]
impl TokenCounter for StandInCounter {
    async fn count_messages(&self, messages: &[Message]) -> Option<i64> {
        Some(i64::try_from(messages.len()).unwrap_or(i64::MAX))
    }

    async fn count_text(&self, text: &str) -> Option<i64> {
        Some(i64::try_from(text.chars().count()).unwrap_or(i64::MAX))
    }
}

fn reply_values(input: &ReplyCases) -> HelperResult {
    let runtime = runtime()?;
    let counter = StandInCounter;
    let replies: Vec<Value> = input
        .replies
        .iter()
        .map(|reply| one_reply(&runtime, &counter, reply))
        .collect();
    Ok(json!({"replies": replies}))
}

fn one_reply(
    runtime: &tokio::runtime::Runtime,
    counter: &StandInCounter,
    reply: &ReplyCase,
) -> Value {
    let input = ReplyInput {
        body: reply.body.as_bytes(),
        status: reply.status,
        headers: &reply.headers,
        messages: &reply.messages,
        model: &reply.model,
        created: FIXED_NOW,
    };
    match runtime.block_on(transform_response(&input, counter)) {
        Ok(response) => json!({
            "ok": true,
            "model": response.model,
            "created": response.created,
            "choices": response.choices.iter().map(|choice| json!({
                "index": choice.index,
                "content": choice.content,
                "finish_reason": choice.finish_reason,
            })).collect::<Vec<Value>>(),
            "usage": {
                "prompt_tokens": response.usage.prompt_tokens,
                "completion_tokens": response.usage.completion_tokens,
                "total_tokens": response.usage.total_tokens,
            },
        }),
        Err(error) => json!({
            "ok": false,
            "status": error.status_code,
            "message": error.message,
        }),
    }
}
