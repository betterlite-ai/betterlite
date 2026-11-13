use std::collections::{BTreeMap, BTreeSet};
use std::sync::Mutex;

use betterlite_openai::chat::ChatMessage;
use betterlite_openai::openai_base_config::{CatalogCapabilities, Headers};
use betterlite_openai::sagemaker_chat::request as chat;
use betterlite_openai::sagemaker_chat::nova;
use betterlite_openai::sagemaker_completion::params::{Options, map_openai_params};
use betterlite_openai::sagemaker_completion::request::{
    CustomPromptEntry, RequestInput, transform_request as completion_request,
};
use betterlite_openai::sagemaker_completion::response::{
    ReplyInput, TextTokenCounter, transform_response,
};
use betterlite_prompts::arguments::json::Object as Body;
use betterlite_prompts::builtin::Message;
use betterlite_prompts::factory::PromptFactory;
use betterlite_prompts::factory::hub::{Hub, builtin_entries};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::{Helper, HelperError, HelperResult};

mod script;

use self::script::{FIXED_CREATED, FIXED_NOW, WordCounter, factory, runtime};

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum Step {
    Params(ParamsStep),
    CompletionRequest(CompletionRequestStep),
    CompletionResponse(CompletionResponseStep),
    ChatHeaders(ChatHeadersStep),
    ChatPolicy(ChatPolicyStep),
    ChatRequest(ChatRequestStep),
    NovaRequest(ChatRequestStep),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ParamsStep {
    #[serde(default)]
    pub(crate) non_default: Options,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CompletionRequestStep {
    pub(crate) model: String,
    #[serde(default)]
    pub(crate) messages: Vec<Message>,
    #[serde(default)]
    pub(crate) optional: Options,
    #[serde(default)]
    pub(crate) custom_prompt_dict: BTreeMap<String, CustomPromptEntry>,
    #[serde(default)]
    pub(crate) global_custom_prompt_dict: BTreeMap<String, CustomPromptEntry>,
    #[serde(default)]
    pub(crate) hf_model_name: Option<String>,
    #[serde(default = "default_now")]
    pub(crate) now: i64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CompletionResponseStep {
    pub(crate) reply: Value,
    pub(crate) model: String,
    #[serde(default)]
    pub(crate) prompt: String,
    #[serde(default = "default_created")]
    pub(crate) created: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ChatHeadersStep {
    #[serde(default)]
    pub(crate) headers: BTreeMap<String, String>,
    #[serde(default)]
    pub(crate) optional: Body,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ChatPolicyStep {
    pub(crate) model: String,
    #[serde(default)]
    pub(crate) chat_models: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ChatRequestStep {
    pub(crate) model: String,
    #[serde(default)]
    pub(crate) messages: Vec<ChatMessage>,
    #[serde(default)]
    pub(crate) optional: Body,
    #[serde(default)]
    pub(crate) hf_model_name: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(crate) steps: Vec<Step>,
}

const fn default_now() -> i64 {
    FIXED_NOW
}

const fn default_created() -> u64 {
    FIXED_CREATED
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let mut results = Vec::with_capacity(input.steps.len());
        for step in &input.steps {
            results.push(execute(step)?);
        }
        Ok(Value::Array(results))
    }
}

fn execute(step: &Step) -> HelperResult {
    match step {
        Step::Params(case) => Ok(params_step(case)),
        Step::CompletionRequest(case) => completion_request_step(case),
        Step::CompletionResponse(case) => completion_response_step(case),
        Step::ChatHeaders(case) => Ok(chat_headers_step(case)),
        Step::ChatPolicy(case) => Ok(chat_policy_step(case)),
        Step::ChatRequest(case) => chat_request_step(case, "chat_request", chat::PROVIDER),
        Step::NovaRequest(case) => chat_request_step(case, "nova_request", nova::PROVIDER),
    }
}

fn params_step(case: &ParamsStep) -> Value {
    let mut requested = case.non_default.clone();
    let mut optional = Options::new();
    map_openai_params(&mut requested, &mut optional);
    json!({"kind": "params", "optional": optional, "remaining": requested})
}

fn body_step(kind: &str, body: &Body) -> Value {
    let keys: Vec<&str> = body.keys().map(String::as_str).collect();
    json!({"kind": kind, "keys": keys, "body": body})
}

fn completion_request_step(case: &CompletionRequestStep) -> HelperResult {
    let prompts = factory();
    let now = DateTime::<Utc>::from_timestamp(case.now, 0).unwrap_or_default();
    let request = RequestInput {
        model: &case.model,
        messages: &case.messages,
        optional_params: &case.optional,
        custom_prompt_dict: &case.custom_prompt_dict,
        global_custom_prompt_dict: &case.global_custom_prompt_dict,
        hf_model_name: case.hf_model_name.as_deref(),
        prompts: &prompts,
        now,
    };
    let body = runtime()?.block_on(completion_request(&request)).map_err(|error| {
        Box::new(HelperError {
            class: String::from("RequestError"),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: error.to_string(),
        })
    })?;
    Ok(body_step("completion_request", &body))
}

fn completion_response_step(case: &CompletionResponseStep) -> HelperResult {
    let input = ReplyInput {
        reply: &case.reply,
        model: &case.model,
        prompt: &case.prompt,
        id: String::from("chatcmpl-default"),
        created: case.created,
    };
    let response = transform_response(&input, &WordCounter).map_err(|error| {
        Box::new(HelperError {
            class: String::from("SagemakerError"),
            status: Some(error.status_code),
            error_type: None,
            param: None,
            code: None,
            message: error.message,
        })
    })?;
    let choice = response
        .choices
        .first()
        .ok_or_else(|| script::failure("the response has no choice"))?;
    Ok(json!({
        "kind": "completion_response",
        "model": response.model,
        "created": response.created,
        "content": choice.message.content,
        "finish_reason": choice.finish_reason,
        "usage": {
            "prompt_tokens": response.usage.prompt_tokens,
            "completion_tokens": response.usage.completion_tokens,
            "total_tokens": response.usage.total_tokens,
        },
    }))
}

fn chat_headers_step(case: &ChatHeadersStep) -> Value {
    let headers: Headers = case.headers.clone();
    let merged = chat::validate_environment(headers, &case.optional);
    let error = chat::get_error_class("boom", script::COMPONENT_STATUS, Some(script::fixture_headers()));
    json!({
        "kind": "chat_headers",
        "headers": merged,
        "error": {"status_code": error.status_code, "headers": error.headers},
    })
}

fn chat_policy_step(case: &ChatPolicyStep) -> Value {
    let catalog = CatalogCapabilities {
        chat_models: case.chat_models.iter().cloned().collect::<BTreeSet<String>>(),
        text_models: BTreeSet::new(),
    };
    json!({
        "kind": "chat_policy",
        "chat_supported": chat::supported_openai_params(&case.model, &catalog),
        "nova_supported": nova::supported_openai_params(&case.model, &catalog),
        "chat_flags": [chat::HAS_CUSTOM_STREAM_WRAPPER, chat::SUPPORTS_STREAM_PARAM_IN_BODY],
        "nova_flags": [nova::HAS_CUSTOM_STREAM_WRAPPER, nova::SUPPORTS_STREAM_PARAM_IN_BODY],
    })
}

fn chat_request_step(case: &ChatRequestStep, kind: &str, provider: &'static str) -> HelperResult {
    let input = chat::TransformInput {
        model: &case.model,
        messages: &case.messages,
        optional_params: &case.optional,
        hf_model_name: case.hf_model_name.as_deref(),
    };
    let body = if provider == nova::PROVIDER {
        nova::transform_request(&input)
    } else {
        chat::transform_request(&input)
    }
    .map_err(|error| {
        Box::new(HelperError {
            class: String::from("ChatContentError"),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: error.to_string(),
        })
    })?;
    Ok(body_step(kind, &body))
}
