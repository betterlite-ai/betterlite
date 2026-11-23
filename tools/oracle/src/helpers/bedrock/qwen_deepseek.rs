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
    projection: QwenProjection,
    model: String,
    messages: Json,
    body: String,
    response: Reply,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct QwenRequestCall {
    // LiteLLM's request API takes the model; the Rust projection takes none.
    #[serde(rename = "model")]
    _model: String,
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

// Which Bedrock reply transform a wire feeds, and the input values the two reply kinds differ in.
#[derive(Clone, Copy)]
enum ReplyKind {
    Qwen(QwenProjection),
    Deepseek,
}

#[derive(Clone, Copy)]
struct ReplyWire<'wire> {
    kind: ReplyKind,
    model: &'wire str,
    messages: &'wire Json,
    request_data: &'wire IndexMap<String, Json>,
    response: &'wire Reply,
    body: &'wire [u8],
    status: u16,
    headers: &'wire [(String, String)],
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let mut answers = Vec::new();
        for call in &input.qwen_reply {
            answers.push(reply_case(ReplyWire {
                kind: ReplyKind::Qwen(call.projection),
                model: &call.model,
                messages: &call.messages,
                request_data: &IndexMap::new(),
                response: &call.response,
                body: call.body.as_bytes(),
                status: 200,
                headers: &[],
            })?);
        }
        for call in &input.qwen_request {
            answers.push(qwen_request(call)?);
        }
        for call in &input.deepseek_reply {
            answers.push(reply_case(ReplyWire {
                kind: ReplyKind::Deepseek,
                model: &call.model,
                messages: &Json::List(Vec::new()),
                request_data: &call.request_data,
                response: &call.response,
                body: call.body.as_bytes(),
                status: call.status,
                headers: &call.headers,
            })?);
        }
        Ok(json!({ "results": answers }))
    }
}

// Both reply projections share this invoke wiring; only the transform and its answer shape differ.
fn reply_case(wire: ReplyWire<'_>) -> Result<Value, Box<HelperError>> {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).or_value_error()?;
    let sink = Silent;
    let (mut state, env) = logging(&redactor, &sink)?;
    let empty: IndexMap<String, Json> = IndexMap::new();
    let optional = OptionalParams::default();
    let mappings: IndexMap<String, String> = IndexMap::new();
    let invoke = InvokeReplyCall {
        model: wire.model,
        messages: wire.messages,
        counted_messages: &[],
        request_data: wire.request_data,
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
        status: wire.status,
        headers: wire.headers,
        body: wire.body,
    };
    match wire.kind {
        ReplyKind::Qwen(projection) => {
            let outcome =
                transform_qwen_response(projection, invoke, &raw, build_reply(wire.response));
            let logged = state.model_call_details.contains_key("log_event_type");
            Ok(qwen_answer(outcome, logged))
        }
        ReplyKind::Deepseek => deepseek_case(invoke, &raw, wire.response),
    }
}

// The DeepSeek wire runs the base conversion on a current-thread runtime and answers in its own shape.
fn deepseek_case(
    invoke: InvokeReplyCall<'_, '_, '_>,
    raw: &RawInvokeReply<'_>,
    response: &Reply,
) -> Result<Value, Box<HelperError>> {
    let runtime = Builder::new_current_thread().build().or_value_error()?;
    let outcome = runtime.block_on(decode_deepseek_reply(
        invoke,
        raw,
        build_reply(response),
        &ZeroCounter,
        &NoReasoning,
    ));
    match outcome {
        Ok(reply) => Ok(json!({ "ok": reply_view(&reply) })),
        Err(error) => Ok(json!({ "error": deepseek_error(&error) })),
    }
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
        Err(error) => {
            let class = match error {
                PromptError::ContentItemText { .. } => "TypeError",
                PromptError::MessageNotObject { .. }
                | PromptError::ContentItemNotObject { .. }
                | PromptError::ToolCallNotObject { .. }
                | PromptError::FunctionNotObject { .. } => "AttributeError",
                PromptError::Format(_) | PromptError::Encode(_) => "ValueError",
            };
            json!({ "error": { "class": class, "status": Value::Null } })
        }
    })
}

// The Python class each Qwen reply error stands for, since LiteLLM re-raises whatever it caught.
fn qwen_answer(outcome: Result<Reply, QwenReplyError>, logged: bool) -> Value {
    match outcome {
        Ok(reply) => json!({ "ok": reply_view(&reply), "logged": logged }),
        Err(error) => {
            let class = match error {
                QwenReplyError::Decode(_) => "JSONDecodeError",
                QwenReplyError::UsageCount { .. } => "ValidationError",
                QwenReplyError::NotObject
                | QwenReplyError::NotText { .. }
                | QwenReplyError::UsageNotObject => "AttributeError",
            };
            json!({ "error": { "class": class, "status": Value::Null }, "logged": logged })
        }
    }
}

fn deepseek_error(error: &DeepseekReplyError) -> Value {
    let (class, status) = match error {
        DeepseekReplyError::Base(base) => base.bedrock().map_or(("ValueError", None), |bedrock| {
            ("BedrockError", Some(bedrock.error.status_code))
        }),
        DeepseekReplyError::NotChat | DeepseekReplyError::UsageRange { .. } => ("ValueError", None),
        DeepseekReplyError::PromptNotText => ("AttributeError", None),
    };
    json!({ "class": class, "status": status })
}

fn params_json(source: &Params) -> Result<Value, Box<HelperError>> {
    let mut fields: IndexMap<String, Value> = IndexMap::new();
    for (name, value) in source {
        fields.insert(name.clone(), from_str(value.get())?);
    }
    Ok(json!(fields))
}
