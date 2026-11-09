use std::backtrace::Backtrace;
use std::sync::Arc;

use betterlite_auth::auxiliary_routes;
use betterlite_auth::route_match::classifier;
use betterlite_auth::route_match::templates::RouteValue;
use betterlite_config::sections::settings::JsonValue;
use betterlite_core::routes::CallType;
use betterlite_error::proxy_error_types::ProxyErrorTypes;
use betterlite_openai::openai_base_config::Object as JsonMap;

use super::hook::{FailureHook, FailureHookInput};
use super::lift::deployment::stamp_deployment_attribution;
use super::lift::{PASS_THROUGH_CALL_TYPE, call_type_for_route, call_type_name};
use super::ports::{CallEnvironment, FailureInput, FailureLogging, RouteCallTypes};
use crate::guardrails::callback_caps::errors::ProxyHookError;

const LOGGED_LITELLM_PARAMS: [&str; 18] = [
    "force_timeout",
    "custom_llm_provider",
    "api_base",
    "litellm_call_id",
    "model_alias_map",
    "metadata",
    "litellm_metadata",
    "model_info",
    "proxy_server_request",
    "acompletion",
    "preset_cache_key",
    "no_log",
    "cost_per_second",
    "input_cost_per_second",
    "input_cost_per_token",
    "output_cost_per_token",
    "output_cost_per_second",
    "cooldown_time",
];

#[must_use]
pub fn is_proxy_only_error(
    error: &ProxyHookError,
    error_type: Option<ProxyErrorTypes>,
    route: Option<&str>,
) -> bool {
    let Some(path) = route else {
        return false;
    };
    if !classifier::is_llm_api_route(&RouteValue::Text(path))
        && !auxiliary_routes::INFO_ROUTES.contains(&path)
    {
        return false;
    }
    matches!(
        error,
        ProxyHookError::Http { .. }
            | ProxyHookError::Proxy(_)
            | ProxyHookError::GuardrailRaised { .. }
    ) || error_type == Some(ProxyErrorTypes::AuthError)
}

pub(crate) async fn handle_proxy_only_error(hook: &FailureHook, input: &mut FailureHookInput<'_>) {
    let logging = ensure_logging(hook, input);
    let (picked, call_type) = update_logging_request(hook, input, &logging);
    if call_type.as_deref() == Some(PASS_THROUGH_CALL_TYPE) {
        return;
    }
    logging.mark_no_upstream_llm_call();
    logging.pre_call(&picked, "").await;
    dispatch_failure_handlers(hook, &logging, input.error).await;
}

fn update_logging_request(
    hook: &FailureHook,
    input: &FailureHookInput<'_>,
    logging: &Arc<dyn FailureLogging>,
) -> (FailureInput, Option<String>) {
    let (mut litellm_params, optional_params) = split_params(input.request_data);
    let details = logging.details();
    let dispatched = details.first_api_call_start_time.is_some() || details.cache_hit;
    let attribution = stamp_deployment_attribution(
        &mut litellm_params,
        input.request_data.get("model").and_then(JsonValue::as_str),
        input.key.team_id.as_deref(),
        dispatched,
        hook.deployments.as_ref(),
    );
    let model = input
        .request_data
        .get("model")
        .and_then(JsonValue::as_str)
        .unwrap_or_default()
        .to_owned();
    let user = input
        .request_data
        .get("user")
        .and_then(JsonValue::as_str)
        .unwrap_or_default()
        .to_owned();
    logging.update_environment_variables(CallEnvironment {
        model,
        user,
        optional_params,
        litellm_params,
        custom_llm_provider: attribution.custom_llm_provider,
    });
    let (picked, body_call_type) = pick_input(input.request_data);
    logging.record_input(&picked);
    let call_type = resolve_call_type(
        input.route,
        hook.route_call_types.as_ref(),
        logging,
        body_call_type,
    );
    (picked, call_type)
}

fn resolve_call_type(
    route: Option<&str>,
    routes: &dyn RouteCallTypes,
    logging: &Arc<dyn FailureLogging>,
    body_call_type: Option<CallType>,
) -> Option<String> {
    let mut call_type = logging.call_type();
    if let Some(resolved) = call_type_for_route(route, routes).or(body_call_type)
        && call_type.as_deref() != Some(PASS_THROUGH_CALL_TYPE)
    {
        logging.set_call_type(resolved);
        call_type = call_type_name(resolved);
    }
    call_type
}

fn ensure_logging(hook: &FailureHook, input: &mut FailureHookInput<'_>) -> Arc<dyn FailureLogging> {
    if let Some(logging) = &input.logging {
        return Arc::clone(logging);
    }
    if !input.request_data.contains_key("litellm_call_id") {
        input.request_data.insert(
            "litellm_call_id".to_owned(),
            JsonValue::String((hook.new_call_id)()),
        );
    }
    let call_id = input
        .request_data
        .get("litellm_call_id")
        .and_then(JsonValue::as_str)
        .unwrap_or_default()
        .to_owned();
    let logging = hook.logging_factory.create_logging(
        input.route.unwrap_or("IGNORE_THIS"),
        &call_id,
        input.request_data,
    );
    apply_key_metadata(input.request_data, &input.key.logged_key_metadata);
    input.logging = Some(Arc::clone(&logging));
    logging
}

fn apply_key_metadata(request_data: &mut JsonMap, metadata: &JsonMap) {
    if request_data.get("metadata").is_none_or(JsonValue::is_null) {
        request_data.insert("metadata".to_owned(), JsonValue::Object(JsonMap::new()));
    }
    let Some(JsonValue::Object(existing)) = request_data.get_mut("metadata") else {
        return;
    };
    for (name, value) in metadata {
        existing.insert(name.clone(), value.clone());
    }
}

fn split_params(request_data: &JsonMap) -> (JsonMap, JsonMap) {
    let mut litellm_params = JsonMap::new();
    let mut optional_params = JsonMap::new();
    for (name, value) in request_data {
        if LOGGED_LITELLM_PARAMS.contains(&name.as_str()) {
            litellm_params.insert(name.clone(), value.clone());
            continue;
        }
        if matches!(name.as_str(), "model" | "user" | "litellm_logging_obj") {
            continue;
        }
        optional_params.insert(name.clone(), value.clone());
    }
    (litellm_params, optional_params)
}

fn pick_input(request_data: &JsonMap) -> (FailureInput, Option<CallType>) {
    if let Some(JsonValue::Array(messages)) = request_data.get("messages") {
        return (
            FailureInput::Messages(messages.clone()),
            Some(CallType::Acompletion),
        );
    }
    if let Some(JsonValue::String(prompt)) = request_data.get("prompt") {
        return (
            FailureInput::Prompt(prompt.clone()),
            Some(CallType::AtextCompletion),
        );
    }
    if let Some(JsonValue::Array(input)) = request_data.get("input") {
        return (
            FailureInput::Embedding(input.clone()),
            Some(CallType::Aembedding),
        );
    }
    (FailureInput::Empty, None)
}

// LiteLLM's expected-client-error rule, not the alert rule: a 4xx status or code, a guardrail block included
fn is_expected_client_error(error: &ProxyHookError) -> bool {
    match error {
        ProxyHookError::Http { status, .. } | ProxyHookError::GuardrailRaised { status, .. } => {
            (400..500).contains(status)
        }
        ProxyHookError::Proxy(exception) => exception
            .code
            .parse::<u16>()
            .is_ok_and(|code| (400..500).contains(&code)),
        ProxyHookError::SensitiveDataRoute(_)
        | ProxyHookError::ModifyResponse(_)
        | ProxyHookError::RejectedRequest { .. }
        | ProxyHookError::Other(_) => false,
    }
}

async fn dispatch_failure_handlers(
    hook: &FailureHook,
    logging: &Arc<dyn FailureLogging>,
    error: &ProxyHookError,
) {
    let include_traceback = hook.log_client_error_tracebacks || !is_expected_client_error(error);
    let traceback = if include_traceback {
        format!("{:?}", Backtrace::capture())
    } else {
        String::new()
    };
    logging.async_failure_handler(error, &traceback).await;
    let owned_logging = Arc::clone(logging);
    let owned_error = error.clone();
    let _task = hook
        .alerts
        .tasks
        .spawn_blocking(move || owned_logging.failure_handler(&owned_error, &traceback));
}
