pub mod breadcrumb;
pub mod messages;
mod registration;

use std::sync::Arc;
use std::time::Duration;

use betterlite_core::rules::RuleError;
use betterlite_core::rules::RuleEvaluator;
use betterlite_logging::dynamic_callbacks::error::DynamicCallbackError;
use betterlite_logging::dynamic_callbacks::params::standard_callback_params as callback_params;
use betterlite_logging::logging_privacy::PrivacyContext;
use betterlite_logging::logging_privacy::callbacks::TrustedCallbacks;
use betterlite_logging::logging_privacy::request::PrivacyRequest;
use betterlite_logging::logging_state::LoggingInit;
use betterlite_logging::logging_state::LoggingState;
use betterlite_logging::logging_state::LoggingStateError;
use betterlite_logging::logging_state::correlation::CorrelationContext;
use betterlite_logging::logging_state::correlation::CorrelationSettings;
use betterlite_logging::logging_state::correlation::CorrelationSetup;
use betterlite_logging::logging_state::types::CallbackPolicy;
use betterlite_logging::logging_state::types::Messages;
use betterlite_logging::logging_state::types::RequestLoggingFlags;
use betterlite_logging::logging_state::updates::CallUpdate;
use betterlite_logging::logging_state::updates::update_environment_variables;
use betterlite_models::decisions::Json;
use betterlite_redact::secret::Redactor;
use chrono::DateTime;
use chrono::FixedOffset;
use indexmap::IndexMap;
use secrecy::SecretString;
use thiserror::Error;

use self::breadcrumb::BreadcrumbSink;
use self::registration::DynamicInputs;
use self::registration::register_request_callbacks;
use self::registration::resolve_global_callbacks;
use self::registration::take_dynamic_callbacks;
use super::callbacks::CallbackLists;
use super::callbacks::CustomLoggers;
use super::callbacks::get_dynamic_callbacks;
use super::guardrails::GuardrailError;
use super::guardrails::get_applied_guardrails;
use super::hooks::DeploymentHook;
use super::providers::ProviderRegistry;
use super::providers::custom_llm_setup;
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
    let function_id = function_id(&request)?;
    let dynamic = take_dynamic_callbacks(&mut request)?;
    let all_callbacks = get_dynamic_callbacks(deps.callbacks, dynamic.callbacks.as_deref());
    register_request_callbacks(&all_callbacks, &deps.loggers, &mut *deps.lists);
    resolve_global_callbacks(&mut *deps.lists, &deps.loggers);
    breadcrumb::add_call_breadcrumb(&request, deps.breadcrumbs, deps.turn_off_message_logging)?;
    let model = required_text(&request, "model")?;
    let messages = messages::message_source(original_function, &mut request, &model, rules)?;
    let stream = is_streaming_request(&request, original_function);
    let input = LoggingInput {
        model,
        call_type: original_function.to_owned(),
        start_time,
        messages,
        stream,
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
        litellm_call_id: required_text(request, "litellm_call_id")?,
        function_id: input.function_id,
        messages: input.messages,
        stream: Some(input.stream),
        litellm_trace_id: trace_id(request)?,
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

fn apply_environment(state: &mut LoggingState, request: &RequestMap) -> Result<(), CallSetupError> {
    let update = CallUpdate {
        model: Some(state.model.clone()),
        user: Some(String::new()),
        optional_params: IndexMap::new(),
        additional_params: stream_options(request),
    };
    let params = call_litellm_params(request);
    update_environment_variables(state, &params, update)?;
    Ok(())
}

fn function_id(request: &RequestMap) -> Result<String, CallSetupError> {
    match request.get("id") {
        None | Some(Json::Null) => Ok(String::new()),
        Some(Json::Text(id)) => Ok(id.clone()),
        Some(_) => Err(CallSetupError::FieldType {
            field: "id",
            expected: "a string",
        }),
    }
}

fn required_text(request: &RequestMap, field: &'static str) -> Result<String, CallSetupError> {
    match request.get(field) {
        Some(Json::Text(text)) => Ok(text.clone()),
        None | Some(Json::Null) => Err(CallSetupError::MissingField { field }),
        Some(_) => Err(CallSetupError::FieldType {
            field,
            expected: "a string",
        }),
    }
}

fn trace_id(request: &RequestMap) -> Result<Option<String>, CallSetupError> {
    match request.get("litellm_trace_id") {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(id)) => Ok(Some(id.clone())),
        Some(_) => Err(CallSetupError::FieldType {
            field: "litellm_trace_id",
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
    let encoded = serde_json::to_vec(request)?;
    let privacy = serde_json::from_slice(&encoded)?;
    Ok(privacy)
}

fn call_litellm_params(request: &RequestMap) -> IndexMap<String, Json> {
    let mut params = IndexMap::new();
    params.insert(String::from("api_base"), Json::Text(String::new()));
    if let Some(metadata) = request.get("metadata") {
        params.insert(String::from("metadata"), metadata.clone());
    }
    if let Some(Json::Object(litellm_metadata)) = request.get("litellm_metadata") {
        params.insert(
            String::from("litellm_metadata"),
            Json::Object(litellm_metadata.clone()),
        );
        if matches!(params.get("metadata"), None | Some(Json::Null)) {
            params.insert(
                String::from("metadata"),
                Json::Object(litellm_metadata.clone()),
            );
        }
    }
    params
}

fn stream_options(request: &RequestMap) -> IndexMap<String, Json> {
    let mut params = IndexMap::new();
    if let Some(options) = request.get("stream_options")
        && !matches!(options, Json::Null)
    {
        params.insert(String::from("stream_options"), options.clone());
    }
    params
}
