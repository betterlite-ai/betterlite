use betterlite_bedrock::agentcore_stream::UuidV4;
use betterlite_bedrock::converse_response::request::OptionalParams;
use betterlite_bedrock::image_call::plan::LogContext;
use betterlite_bedrock::invoke_primitives::config::Params;
use betterlite_bedrock::invoke_response::{InvokeReplyCall, RawInvokeReply};
use betterlite_bedrock::qwen_deepseek::error::{DeepseekReplyError, QwenReplyError};
use betterlite_bedrock::qwen_deepseek::options::{
    SUPPORTED_OPENAI_PARAMS, map_openai_params, transform_request,
};
use betterlite_bedrock::qwen_deepseek::prompt::PromptError;
use betterlite_bedrock::qwen_deepseek::reply::Reply;
use betterlite_bedrock::qwen_deepseek::{
    QwenProjection, decode_deepseek_reply, transform_qwen_response,
};
use betterlite_models::decisions::Json;
use betterlite_prompts::arguments::json::Json as PromptJson;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use betterlite_tokenizer::count::request::Message;
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::{Value, from_str, json};
use tokio::runtime::Builder;

use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

mod counter;
mod fixture;
mod tests;
mod view;

use counter::{NoReasoning, ZeroCounter};
use fixture::{Silent, build_reply, logging};
use view::reply_view;

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    qwen_reply: Vec<QwenReplyCall>,
    qwen_request: Vec<QwenRequestCall>,
    deepseek_reply: Vec<DeepseekReplyCall>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct QwenReplyCall {
    projection: Projection,
    model: String,
    messages: Json,
    body: String,
    response: Reply,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct QwenRequestCall {
    model: String,
    messages: Vec<PromptJson>,
    non_default_params: Params,
    optional_params: Params,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DeepseekReplyCall {
    model: String,
    status: u16,
    headers: Vec<(String, String)>,
    request_data: IndexMap<String, Json>,
    body: String,
    response: Reply,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Projection {
    Qwen2,
    Qwen3,
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let mut answers = Vec::new();
        for call in &input.qwen_reply {
            answers.push(qwen_reply(call)?);
        }
        for call in &input.qwen_request {
            answers.push(qwen_request(call)?);
        }
        for call in &input.deepseek_reply {
            answers.push(deepseek_reply(call)?);
        }
        Ok(json!({ "results": answers }))
    }
}

fn qwen_reply(call: &QwenReplyCall) -> Result<Value, Box<HelperError>> {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).or_value_error()?;
    let sink = Silent;
    let (mut state, env) = logging(&redactor, &sink)?;
    let empty: IndexMap<String, Json> = IndexMap::new();
    let optional = OptionalParams::default();
    let mappings: IndexMap<String, String> = IndexMap::new();
    let invoke = InvokeReplyCall {
        model: &call.model,
        messages: &call.messages,
        counted_messages: &[],
        request_data: &empty,
        optional_params: &empty,
        optional: &optional,
        litellm_params: &empty,
        tool_name_mappings: &mappings,
        json_mode: None,
        disable_add_prefix_to_prompt: false,
        now: 0.0_f64,
        ids: &UuidV4,
        log: LogContext {
            state: &mut state,
            env: &env,
        },
    };
    let raw = RawInvokeReply {
        status: 200,
        headers: &[],
        body: call.body.as_bytes(),
    };
    let outcome = transform_qwen_response(
        projection(call.projection),
        invoke,
        &raw,
        build_reply(&call.response),
    );
    let logged = state.model_call_details.contains_key("log_event_type");
    Ok(match outcome {
        Ok(reply) => json!({ "ok": reply_view(&reply), "logged": logged }),
        Err(error) => json!({
            "error": { "class": qwen_class(&error), "status": Value::Null },
            "logged": logged,
        }),
    })
}

fn qwen_request(call: &QwenRequestCall) -> Result<Value, Box<HelperError>> {
    let mapped = map_openai_params(&call.non_default_params, call.optional_params.clone());
    Ok(match transform_request(&call.messages, &mapped) {
        Ok(body) => json!({
            "ok": {
                "supported": SUPPORTED_OPENAI_PARAMS,
                "mapped": params_json(&mapped)?,
                "body": params_json(&body)?,
            }
        }),
        Err(error) => json!({
            "error": { "class": prompt_class(&error), "status": Value::Null },
        }),
    })
}

fn deepseek_reply(call: &DeepseekReplyCall) -> Result<Value, Box<HelperError>> {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).or_value_error()?;
    let sink = Silent;
    let (mut state, env) = logging(&redactor, &sink)?;
    let optional = OptionalParams::default();
    let mappings: IndexMap<String, String> = IndexMap::new();
    let counted: Vec<Message> = Vec::new();
    let messages = Json::List(Vec::new());
    let empty: IndexMap<String, Json> = IndexMap::new();
    let invoke = InvokeReplyCall {
        model: &call.model,
        messages: &messages,
        counted_messages: &counted,
        request_data: &call.request_data,
        optional_params: &empty,
        optional: &optional,
        litellm_params: &empty,
        tool_name_mappings: &mappings,
        json_mode: None,
        disable_add_prefix_to_prompt: false,
        now: 0.0_f64,
        ids: &UuidV4,
        log: LogContext {
            state: &mut state,
            env: &env,
        },
    };
    let raw = RawInvokeReply {
        status: call.status,
        headers: &call.headers,
        body: call.body.as_bytes(),
    };
    let runtime = Builder::new_current_thread().build().or_value_error()?;
    let outcome = runtime.block_on(decode_deepseek_reply(
        invoke,
        &raw,
        build_reply(&call.response),
        &ZeroCounter,
        &NoReasoning,
    ));
    Ok(match outcome {
        Ok(reply) => json!({ "ok": reply_view(&reply) }),
        Err(error) => json!({ "error": deepseek_error(&error) }),
    })
}

// The Python class each Rust reply error stands for, since LiteLLM re-raises whatever it caught.
fn qwen_class(error: &QwenReplyError) -> &'static str {
    match error {
        QwenReplyError::Decode(_) => "JSONDecodeError",
        QwenReplyError::NotObject
        | QwenReplyError::NotText { .. }
        | QwenReplyError::UsageNotObject => "AttributeError",
        QwenReplyError::UsageCount { .. } => "ValidationError",
    }
}

fn prompt_class(error: &PromptError) -> &'static str {
    match error {
        PromptError::ContentItemText { .. } => "TypeError",
        PromptError::MessageNotObject { .. }
        | PromptError::ContentItemNotObject { .. }
        | PromptError::ToolCallNotObject { .. }
        | PromptError::FunctionNotObject { .. } => "AttributeError",
        PromptError::Format(_) | PromptError::Encode(_) => "ValueError",
    }
}

fn deepseek_error(error: &DeepseekReplyError) -> Value {
    let (class, status) = match error {
        DeepseekReplyError::Base(base) => match base.bedrock() {
            Some(bedrock) => ("BedrockError", Some(bedrock.error.status_code)),
            None => ("ValueError", None),
        },
        DeepseekReplyError::NotChat | DeepseekReplyError::UsageRange { .. } => ("ValueError", None),
        DeepseekReplyError::PromptNotText => ("AttributeError", None),
    };
    json!({ "class": class, "status": status })
}

fn projection(value: Projection) -> QwenProjection {
    match value {
        Projection::Qwen2 => QwenProjection::Qwen2,
        Projection::Qwen3 => QwenProjection::Qwen3,
    }
}

fn params_json(source: &Params) -> Result<Value, Box<HelperError>> {
    let mut fields: IndexMap<String, Value> = IndexMap::new();
    for (name, value) in source {
        fields.insert(name.clone(), from_str(value.get())?);
    }
    Ok(json!(fields))
}
