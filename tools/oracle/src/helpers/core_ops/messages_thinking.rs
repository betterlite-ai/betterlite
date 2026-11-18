mod metadata;
mod tests;

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use betterlite_core::api::api_boundaries::{ApiLimitError, ApiLimits};
use betterlite_core::api::messages_thinking::bridge::{BridgeKwargs, local_model_name};
use betterlite_core::api::messages_thinking::effort::normalize_reasoning_effort;
use betterlite_core::api::messages_thinking::response::{
    ResponseJsonError, decode_messages_response,
};
use betterlite_core::api::messages_thinking::session::{SessionKeyError, decode_messages_thinking};
use betterlite_core::api::messages_thinking::summary::auto_summary_enabled;
use betterlite_logging::passthrough_stream_logging::object::{
    StreamLoggingObject, StreamSuccessCall, StreamSuccessDispatch,
};
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::runtime::Builder;

use betterlite_oracle_helper_api::error::{ErrorCode, HelperError};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

use metadata::Metadata;

// The cases carry a few hundred bytes, so the crate's own tests own the decoder bounds.
const LIMITS: ApiLimits = ApiLimits {
    max_json_bytes: 16 * 1024 * 1024,
    max_event_bytes: 1024 * 1024,
    max_items: 64 * 1024,
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    model_cost: IndexMap<String, Value>,
    calls: Vec<Call>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Call {
    Response {
        model: String,
        status: u16,
        body: String,
    },
    SessionCacheKey {
        user_id: Json,
    },
    LocalModelName {
        model: String,
        custom_llm_provider: Json,
    },
    LoggingObject {
        litellm_logging_obj: LoggingKind,
    },
    AutoSummary {
        #[serde(default)]
        global_value: Option<bool>,
        #[serde(default)]
        environment: Option<String>,
    },
    NormalizeEffort {
        effort: String,
        model: String,
        #[serde(default)]
        custom_llm_provider: Option<String>,
    },
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum LoggingKind {
    Supplied,
    Null,
    Absent,
    Lookalike,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let mut calls = Vec::with_capacity(input.calls.len());
        for call in &input.calls {
            calls.push(answer(call, &input.model_cost)?);
        }
        Ok(json!({ "calls": calls }))
    }
}

fn answer(call: &Call, model_cost: &IndexMap<String, Value>) -> HelperResult {
    match call {
        Call::Response {
            model,
            status,
            body,
        } => response(model, *status, body),
        Call::SessionCacheKey { user_id } => session_cache_key(user_id),
        Call::LocalModelName {
            model,
            custom_llm_provider,
        } => Ok(json!({ "value": local_model_name(model, text(custom_llm_provider)) })),
        Call::LoggingObject {
            litellm_logging_obj,
        } => Ok(logging_answer(litellm_logging_obj)),
        Call::AutoSummary {
            global_value,
            environment,
        } => Ok(json!({
            "enabled": auto_summary_enabled(
                global_value.unwrap_or(false),
                environment.as_deref(),
            ),
        })),
        Call::NormalizeEffort {
            effort,
            model,
            custom_llm_provider,
        } => normalize_effort(effort, model, custom_llm_provider.as_deref(), model_cost),
    }
}

fn response(model: &str, status: u16, body: &str) -> HelperResult {
    match decode_messages_response(body.as_bytes(), status, &LIMITS) {
        Ok(fields) => Ok(json!({ "response": fields, "model": model })),
        Err(ResponseJsonError::Anthropic(error)) => Err(helper_error(
            "AnthropicError",
            Some(error.status_code),
            error.message,
        )),
        // Python's AnthropicMessagesResponse(**json) raises TypeError for a list or a scalar
        Err(ResponseJsonError::NotAnObject) => Err(helper_error(
            "TypeError",
            None,
            "the response is not a mapping".to_owned(),
        )),
        Err(ResponseJsonError::Limit(error)) => Err(limit_error(error)),
    }
}

fn session_cache_key(user_id: &Json) -> HelperResult {
    let key = match user_id {
        Json::Text(text) => decode_messages_thinking(text.as_bytes(), &LIMITS)
            .map_err(|SessionKeyError::Limit(limit)| limit_error(limit))?,
        // prompt_cache_key_from_user_id returns None for a plain person or any non-string id
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => None,
    };
    Ok(json!({ "key": key }))
}

// litellm_logging_obj_from_kwargs returns the value only when it is B247's object; a lookalike mapping never enters the typed field.
fn logging_answer(kind: &LoggingKind) -> Value {
    let supplied = match kind {
        LoggingKind::Supplied => Some(logging_object()),
        LoggingKind::Null | LoggingKind::Absent | LoggingKind::Lookalike => None,
    };
    let kwargs = BridgeKwargs {
        litellm_logging_obj: supplied,
        request_body: IndexMap::new(),
    };
    let returned = if kwargs.litellm_logging_obj.is_some() {
        "supplied"
    } else {
        "none"
    };
    json!({ "returned": returned })
}

fn normalize_effort(
    effort: &str,
    model: &str,
    custom_llm_provider: Option<&str>,
    model_cost: &IndexMap<String, Value>,
) -> HelperResult {
    let metadata = Metadata::load(model_cost)?;
    let runtime = Builder::new_current_thread()
        .build()
        .map_err(|source| helper_error("InvalidInput", None, source.to_string()))?;
    let info = metadata.model_info(model, custom_llm_provider, &runtime);
    let normalized = normalize_reasoning_effort(effort, info.as_ref(), &metadata.map);
    Ok(json!({ "effort": normalized }))
}

fn limit_error(error: ApiLimitError) -> Box<HelperError> {
    let fields = error.fields();
    Box::new(HelperError {
        class: "ApiLimitError".to_owned(),
        status: Some(fields.status),
        error_type: Some(Some(fields.error_type.to_owned())),
        param: Some(Some(fields.param.to_owned())),
        code: Some(Some(ErrorCode::Text(fields.code.to_owned()))),
        message: error.to_string(),
    })
}

fn helper_error(class: &str, status: Option<u16>, message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status,
        error_type: Some(None),
        param: Some(None),
        code: Some(None),
        message,
    })
}

const fn text(value: &Json) -> Option<&str> {
    match value {
        Json::Text(text) => Some(text.as_str()),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => None,
    }
}

struct NoDispatch;

#[async_trait]
impl StreamSuccessDispatch for NoDispatch {
    async fn dispatch_success(&self, _call: StreamSuccessCall<'_>) {}
}

fn logging_object() -> Arc<StreamLoggingObject> {
    Arc::new(StreamLoggingObject {
        slot: Mutex::new(None),
        dispatch: Arc::new(NoDispatch),
    })
}
