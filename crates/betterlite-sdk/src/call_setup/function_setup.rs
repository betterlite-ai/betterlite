pub mod environment;
pub mod messages;
mod registration;

use std::sync::Arc;
use std::time::Duration;

use betterlite_core::rules::{RuleError, RuleEvaluator};
use betterlite_logging::dynamic_callbacks::error::DynamicCallbackError;
use betterlite_logging::dynamic_callbacks::params::standard_callback_params as callback_params;
use betterlite_logging::logging_privacy::PrivacyContext;
use betterlite_logging::logging_privacy::callbacks::TrustedCallbacks;
use betterlite_logging::logging_privacy::request::PrivacyRequest;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::types::{CallbackPolicy, Messages, RequestLoggingFlags};
use betterlite_logging::logging_state::{LoggingInit, LoggingState, LoggingStateError};
use betterlite_models::decisions::Json;
use betterlite_redact::secret::Redactor;
use chrono::{DateTime, FixedOffset};
use indexmap::IndexMap;
use secrecy::SecretString;
use thiserror::Error;

use self::environment::apply_environment;
use self::registration::{
    DynamicInputs, register_request_callbacks, resolve_global_callbacks, take_dynamic_callbacks,
};
use super::callbacks::{CallbackLists, CustomLoggers, get_dynamic_callbacks};
use super::guardrails::{GuardrailError, get_applied_guardrails};
use super::hooks::DeploymentHook;
use super::providers::{ProviderRegistry, custom_llm_setup};
use super::request_flags::is_streaming_request;
use crate::call_setup::RequestMap;

#[derive(Error, Debug)]
pub enum CallSetupError {
    #[error(transparent)]
    Rule(#[from] RuleError),
    #[error(transparent)]
    Logging(#[from] LoggingStateError),
    #[error(transparent)]
    Guardrail(#[from] GuardrailError),
    #[error(transparent)]
    Callback(#[from] DynamicCallbackError),
    #[error("the {field} value must be {expected}")]
    FieldType {
        field: &'static str,
        expected: &'static str,
    },
    #[error("the {field} value is required")]
    MissingField { field: &'static str },
    #[error("the request could not be read: {source}")]
    Json {
        #[from]
        source: serde_json::Error,
    },
}

pub trait BreadcrumbSink: Send + Sync {
    fn add_breadcrumb(&self, category: &str, message: &str, level: &str);
}

pub struct TraceSource<'call> {
    pub next_trace_id: &'call dyn Fn() -> String,
    pub clock: &'call mut dyn FnMut() -> Duration,
}

// Everything LiteLLM reads from module globals reaches function_setup through this struct
pub struct CallSetupDependencies<'call> {
    pub lists: &'call mut CallbackLists,
    pub providers: &'call mut ProviderRegistry,
    pub callbacks: &'call [Arc<dyn DeploymentHook>],
    pub loggers: CustomLoggers<'call>,
    pub correlation: &'call mut CorrelationContext,
    pub correlation_settings: CorrelationSettings,
    pub redactor: &'call Redactor,
    pub trace: TraceSource<'call>,
    pub breadcrumbs: Option<&'call dyn BreadcrumbSink>,
    pub litellm_params: IndexMap<String, Json>,
    pub turn_off_message_logging: bool,
    pub is_async_call: bool,
}

#[derive(Debug)]
pub struct CallSetup {
    pub logging: LoggingState,
    pub request: RequestMap,
}

pub fn function_setup<Evaluator>(
    original_function: &str,
    rules: &Evaluator,
    start_time: DateTime<FixedOffset>,
    mut request: RequestMap,
    deps: CallSetupDependencies<'_>,
) -> Result<CallSetup, CallSetupError>
where
    Evaluator: RuleEvaluator,
{
    custom_llm_setup(deps.providers);
    let applied_guardrails = get_applied_guardrails(&request, deps.callbacks)?;
    let function_id = optional_text(&request, "id")?.unwrap_or_default();
    let dynamic = take_dynamic_callbacks(&mut request)?;
    let all_callbacks = get_dynamic_callbacks(deps.callbacks, dynamic.callbacks.as_deref());
    register_request_callbacks(&all_callbacks, &deps.loggers, &mut *deps.lists);
    resolve_global_callbacks(&mut *deps.lists, &deps.loggers);
    add_call_breadcrumb(&request, deps.breadcrumbs, deps.turn_off_message_logging)?;
    let model =
        optional_text(&request, "model")?.ok_or(CallSetupError::MissingField { field: "model" })?;
    let messages = messages::message_source(original_function, &mut request, &model, rules)?;
    let input = LoggingInput {
        model,
        call_type: original_function.to_owned(),
        start_time,
        messages,
        stream: is_streaming_request(&request, original_function),
        dynamic,
        applied_guardrails,
        function_id,
    };
    let logging = build_logging_state(input, &request, deps)?;
    Ok(CallSetup { logging, request })
}

struct LoggingInput {
    model: String,
    call_type: String,
    start_time: DateTime<FixedOffset>,
    messages: Option<Messages>,
    stream: bool,
    dynamic: DynamicInputs,
    applied_guardrails: Vec<String>,
    function_id: String,
}

fn build_logging_state(
    input: LoggingInput,
    request: &RequestMap,
    deps: CallSetupDependencies<'_>,
) -> Result<LoggingState, CallSetupError> {
    let CallSetupDependencies {
        correlation,
        correlation_settings,
        redactor,
        trace,
        litellm_params,
        is_async_call,
        ..
    } = deps;
    let trusted = TrustedCallbacks {
        values: trusted_callback_vars(request)?,
    };
    let privacy = privacy_request(request)?;
    let callback_dynamic_params = callback_params(&privacy, &trusted, PrivacyContext::default())?;
    let init = logging_init(
        input,
        request,
        litellm_params,
        callback_dynamic_params,
        trusted,
    )?;
    let setup = CorrelationSetup {
        context: &mut *correlation,
        settings: CorrelationSettings {
            supports_correlation_logging: is_async_call,
            request_correlation_in_logs: correlation_settings.request_correlation_in_logs,
            secret_redaction: correlation_settings.secret_redaction,
        },
    };
    let mut state = LoggingState::new(init, setup, redactor, trace.next_trace_id, trace.clock)?;
    apply_environment(&mut state, request)?;
    Ok(state)
}

fn logging_init(
    input: LoggingInput,
    request: &RequestMap,
    litellm_params: IndexMap<String, Json>,
    callback_dynamic_params: IndexMap<String, Json>,
    trusted: TrustedCallbacks,
) -> Result<LoggingInit, CallSetupError> {
    Ok(LoggingInit {
        model: input.model,
        call_type: input.call_type,
        start_time: input.start_time,
        litellm_call_id: optional_text(request, "litellm_call_id")?.ok_or(
            CallSetupError::MissingField {
                field: "litellm_call_id",
            },
        )?,
        function_id: input.function_id,
        messages: input.messages,
        stream: Some(input.stream),
        litellm_trace_id: optional_text(request, "litellm_trace_id")?,
        kwargs: request.clone(),
        litellm_params,
        callback_dynamic_params,
        trusted_callback_vars: trusted.values.into_iter().collect(),
        standard_built_in_tools_params: IndexMap::new(),
        dynamic_callbacks: input.dynamic.logging(),
        applied_guardrails: Some(input.applied_guardrails),
        request_logging: RequestLoggingFlags::default(),
        callback_policy: CallbackPolicy::default(),
    })
}

fn optional_text(
    request: &RequestMap,
    field: &'static str,
) -> Result<Option<String>, CallSetupError> {
    match request.get(field) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(text)) => Ok(Some(text.clone())),
        Some(_) => Err(CallSetupError::FieldType {
            field,
            expected: "a string",
        }),
    }
}

fn trusted_callback_vars(
    request: &RequestMap,
) -> Result<IndexMap<String, SecretString>, CallSetupError> {
    let Some(Json::Object(values)) = request.get("litellm_trusted_callback_vars") else {
        return match request.get("litellm_trusted_callback_vars") {
            None | Some(Json::Null) => Ok(IndexMap::new()),
            _ => Err(CallSetupError::FieldType {
                field: "litellm_trusted_callback_vars",
                expected: "an object of strings",
            }),
        };
    };
    let mut trusted = IndexMap::new();
    for (name, value) in values {
        let Json::Text(secret) = value else {
            return Err(CallSetupError::FieldType {
                field: "litellm_trusted_callback_vars",
                expected: "an object of strings",
            });
        };
        trusted.insert(name.clone(), SecretString::from(secret.clone()));
    }
    Ok(trusted)
}

fn privacy_request(request: &RequestMap) -> Result<PrivacyRequest, CallSetupError> {
    // The SDK reads the trusted variables itself, so the privacy check must not see them as smuggled metadata
    let mut direct = request.clone();
    direct.shift_remove("litellm_trusted_callback_vars");
    let encoded = serde_json::to_vec(&direct)?;
    let privacy = serde_json::from_slice(&encoded)?;
    Ok(privacy)
}

fn add_call_breadcrumb(
    request: &RequestMap,
    sink: Option<&dyn BreadcrumbSink>,
    turn_off_message_logging: bool,
) -> Result<(), CallSetupError> {
    let Some(recorder) = sink else {
        return Ok(());
    };
    let mut details = request.clone();
    if turn_off_message_logging {
        for field in ["messages", "input", "prompt"] {
            details.shift_remove(field);
        }
    }
    let encoded = serde_json::to_string(&Json::Object(details))?;
    recorder.add_breadcrumb(
        "litellm.llm_call",
        &format!("Keyword Args: {encoded}"),
        "info",
    );
    Ok(())
}
