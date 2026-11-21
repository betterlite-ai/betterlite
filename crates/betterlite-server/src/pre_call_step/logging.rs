use std::time::Duration;

use betterlite_config::sections::settings::JsonValue;
use betterlite_logging::callback_logger::resolver::LoggerResolver;
use betterlite_logging::dynamic_callbacks::resolve::resolve_callback_string;
use betterlite_logging::logging_privacy::callbacks::TrustedCallbacks;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::types::{
    CallbackPolicy, DynamicCallbacks, Messages, RequestLoggingFlags,
};
use betterlite_logging::logging_state::updates::{CallUpdate, update_environment_variables};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::decisions::Json;
use betterlite_openai::openai_base_config::Object as JsonMap;
use betterlite_redact::secret::Redactor;
use chrono::{DateTime, FixedOffset};
use indexmap::IndexMap;
use uuid::Uuid;

use super::internal;
use super::json::{to_decision, to_request_data};
use crate::pre_call::failure::PreCallFailure;
use crate::pre_call::route_type::RouteType;

pub(super) struct LoggingRequest<'request> {
    pub(super) data: &'request mut JsonMap,
    pub(super) route_type: RouteType,
    pub(super) start_time: DateTime<FixedOffset>,
    pub(super) call_id: &'request str,
    pub(super) loggers: &'request dyn LoggerResolver,
    pub(super) redactor: &'request Redactor,
}

pub(super) fn build_state(request: LoggingRequest<'_>) -> Result<LoggingState, PreCallFailure> {
    let LoggingRequest {
        data,
        route_type,
        start_time,
        call_id,
        loggers,
        redactor,
    } = request;
    let dynamic = take_dynamic_callbacks(data, loggers)?;
    let (init, params) = build_init(data, route_type, start_time, call_id, dynamic);
    let mut context = CorrelationContext::default();
    let setup = CorrelationSetup {
        context: &mut context,
        settings: CorrelationSettings::default(),
    };
    let mut state = LoggingState::new(
        init,
        setup,
        redactor,
        || Uuid::new_v4().to_string(),
        || Duration::ZERO,
    )
    .map_err(|error| internal(error.to_string()))?;
    let update = CallUpdate {
        model: Some(state.model.clone()),
        user: Some(String::new()),
        optional_params: IndexMap::new(),
        additional_params: stream_options(data),
    };
    update_environment_variables(&mut state, &params, update)
        .map_err(|error| internal(error.to_string()))?;
    Ok(state)
}

fn build_init(
    data: &JsonMap,
    route_type: RouteType,
    start_time: DateTime<FixedOffset>,
    call_id: &str,
    dynamic: DynamicCallbacks,
) -> (LoggingInit, IndexMap<String, Json>) {
    let params = environment_params(data);
    let init = LoggingInit {
        model: text_of(data, "model").unwrap_or_default(),
        call_type: <&str>::from(route_type).to_owned(),
        start_time,
        litellm_call_id: call_id.to_owned(),
        function_id: text_of(data, "id").unwrap_or_default(),
        messages: messages_from(data),
        stream: Some(is_stream(data, route_type)),
        litellm_trace_id: text_of(data, "litellm_trace_id"),
        kwargs: to_request_data(data),
        litellm_params: params.clone(),
        callback_dynamic_params: IndexMap::new(),
        trusted_callback_vars: Vec::new(),
        standard_built_in_tools_params: IndexMap::new(),
        dynamic_callbacks: dynamic,
        applied_guardrails: None,
        request_logging: RequestLoggingFlags::default(),
        callback_policy: CallbackPolicy::default(),
    };
    (init, params)
}

fn text_of(data: &JsonMap, field: &str) -> Option<String> {
    data.get(field)
        .and_then(JsonValue::as_str)
        .map(str::to_owned)
}

fn messages_from(data: &JsonMap) -> Option<Messages> {
    match data.get("messages") {
        Some(JsonValue::Array(items)) => {
            Some(Messages::Values(items.iter().map(to_decision).collect()))
        }
        Some(JsonValue::String(text)) => Some(Messages::Text(text.clone())),
        Some(
            JsonValue::Null | JsonValue::Bool(_) | JsonValue::Number(_) | JsonValue::Object(_),
        )
        | None => None,
    }
}

fn is_stream(data: &JsonMap, route_type: RouteType) -> bool {
    matches!(data.get("stream"), Some(JsonValue::Bool(true)))
        || route_type == RouteType::GenerateContentStream
}

fn environment_params(data: &JsonMap) -> IndexMap<String, Json> {
    let mut params = IndexMap::new();
    params.insert(String::from("api_base"), Json::Text(String::new()));
    if let Some(metadata) = data.get("metadata") {
        params.insert(String::from("metadata"), to_decision(metadata));
    }
    let litellm_metadata = data
        .get("litellm_metadata")
        .filter(|value| value.is_object())
        .map(to_decision);
    if let Some(litellm_metadata) = litellm_metadata {
        params.insert(String::from("litellm_metadata"), litellm_metadata.clone());
        if matches!(params.get("metadata"), None | Some(Json::Null)) {
            params.insert(String::from("metadata"), litellm_metadata);
        }
    }
    params
}

fn stream_options(data: &JsonMap) -> IndexMap<String, Json> {
    let mut params = IndexMap::new();
    if let Some(options) = data.get("stream_options")
        && !options.is_null()
    {
        params.insert(String::from("stream_options"), to_decision(options));
    }
    params
}

fn take_dynamic_callbacks(
    data: &mut JsonMap,
    loggers: &dyn LoggerResolver,
) -> Result<DynamicCallbacks, PreCallFailure> {
    let mut dynamic = DynamicCallbacks::default();
    if let Some(listed) = take_names(data, "success_callback") {
        let (success, asynchronous) = split_success(listed);
        dynamic.success = Some(success);
        if !asynchronous.is_empty() {
            dynamic.async_success = Some(asynchronous);
        }
    }
    if let Some(listed) = take_names(data, "failure_callback") {
        dynamic.failure = Some(listed);
    }
    // function_setup registers a per-request callback for every event, so each resolved name joins every list
    for name in take_named(data, "callbacks") {
        let resolved = resolve_callback_string(&name, &TrustedCallbacks::default(), loggers)
            .map_err(|error| internal(error.to_string()))?;
        for logger in resolved {
            let resolved_name = logger.callback_name().to_owned();
            add_name(&mut dynamic.input, &resolved_name);
            add_name(&mut dynamic.success, &resolved_name);
            add_name(&mut dynamic.async_success, &resolved_name);
            add_name(&mut dynamic.failure, &resolved_name);
            add_name(&mut dynamic.async_failure, &resolved_name);
        }
    }
    Ok(dynamic)
}

fn add_name(slot: &mut Option<Vec<String>>, name: &str) {
    let names = slot.get_or_insert_with(Vec::new);
    if !names.iter().any(|existing| existing.as_str() == name) {
        names.push(name.to_owned());
    }
}

fn split_success(listed: Vec<String>) -> (Vec<String>, Vec<String>) {
    let mut success = Vec::new();
    let mut asynchronous = Vec::new();
    for name in listed {
        if matches!(name.as_str(), "dynamodb" | "s3") {
            asynchronous.push(name);
        } else {
            success.push(name);
        }
    }
    (success, asynchronous)
}

fn take_names(data: &mut JsonMap, field: &str) -> Option<Vec<String>> {
    if !matches!(data.get(field), Some(JsonValue::Array(_))) {
        return None;
    }
    Some(take_named(data, field))
}

fn take_named(data: &mut JsonMap, field: &str) -> Vec<String> {
    match data.remove(field) {
        Some(JsonValue::Array(items)) => names_of(&items),
        Some(
            JsonValue::Null
            | JsonValue::Bool(_)
            | JsonValue::Number(_)
            | JsonValue::String(_)
            | JsonValue::Object(_),
        )
        | None => Vec::new(),
    }
}

fn names_of(items: &[JsonValue]) -> Vec<String> {
    items
        .iter()
        .filter_map(JsonValue::as_str)
        .map(str::to_owned)
        .collect()
}
