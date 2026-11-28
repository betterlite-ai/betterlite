#![cfg(test)]

use std::str::from_utf8;
use std::sync::Arc;
use std::time::SystemTime;

use betterlite_config::sections::settings::JsonValue;
use betterlite_models::resolution::ResolutionError;
use betterlite_openai::error::{ProviderError, ProviderErrorParts};
use betterlite_openai::openai_base_config::Object;

use self::support::{
    Calls, FakeView, call_args, empty_view, failing_router_request, override_settings, ports_with,
    request, route_input, sdk_router,
};
use super::entry;
use super::error::{LlmPassthroughError, ModelBlockedError};
use super::route::{PassthroughRouteInput, RouteFlag};
use crate::api::passthrough_call::config::ConfigError;
use crate::api::passthrough_call::{PassthroughBody, PassthroughCallError};
use crate::dispatch::Dispatch;
use crate::dispatch::NoRules;
use crate::dispatch::error::DispatchError;
use crate::dispatch::log::TracingCallLog;
use crate::dispatch::tests::support::{Trace, context};
use crate::operation::Operation;

mod plan;
mod support;

#[tokio::test]
async fn entry_router_plan_calls_router() {
    let trace = Trace::default();
    let calls = Calls::default();
    let view = FakeView {
        recognized: RouteFlag(true),
        ..empty_view()
    };
    let ports = ports_with(Some(view), Vec::new(), &calls, &trace);
    let request = request(
        call_args("gpt-upstream"),
        route_input("gpt-upstream"),
        &ports,
    );
    let reply = entry::execute(request).await.expect("router reply");
    assert_eq!(reply.status, 200);
    let PassthroughBody::Full(body) = reply.body else {
        panic!("the fake answers with a full body");
    };
    assert_eq!(from_utf8(&body).expect("utf-8"), "{}");
    assert_eq!(trace.steps(), ["blocked:gpt-upstream:", "router"]);
    assert_eq!(calls.recorded().len(), 1);
}

#[tokio::test]
async fn entry_direct_plan_calls_direct() {
    let trace = Trace::default();
    let calls = Calls::default();
    let ports = ports_with(None, Vec::new(), &calls, &trace);
    let request = request(
        call_args("gpt-upstream"),
        route_input("gpt-upstream"),
        &ports,
    );
    entry::execute(request).await.expect("direct reply");
    assert_eq!(trace.steps(), ["direct"]);
    assert_eq!(calls.recorded()[0].model, "gpt-upstream");
}

#[tokio::test]
async fn entry_override_copies_every_key() {
    let trace = Trace::default();
    let calls = Calls::default();
    let ports = ports_with(Some(FakeView::default()), Vec::new(), &calls, &trace);
    let input = PassthroughRouteInput {
        router_settings_override: Some(override_settings()),
        ..route_input("gpt-upstream")
    };
    let request = request(call_args("gpt-upstream"), input, &ports);
    entry::execute(request).await.expect("router reply");
    let recorded = calls.recorded();
    assert_eq!(recorded.len(), 1);
    let extra = &recorded[0].extra;
    assert_eq!(extra.len(), 8);
    assert!(!extra.contains_key("cooldown_time"));
    assert_eq!(extra.get("fallbacks"), Some(&JsonValue::Array(Vec::new())));
    assert_eq!(
        extra.get("context_window_fallbacks"),
        Some(&JsonValue::Array(Vec::new()))
    );
    assert_eq!(
        extra.get("content_policy_fallbacks"),
        Some(&JsonValue::Array(Vec::new()))
    );
    assert_eq!(extra.get("num_retries"), Some(&JsonValue::from(3_u32)));
    assert_eq!(extra.get("timeout"), Some(&JsonValue::from(30_u32)));
    assert_eq!(
        extra.get("model_group_retry_policy"),
        Some(&JsonValue::Object(Object::new()))
    );
    assert_eq!(
        extra.get("routing_strategy"),
        Some(&JsonValue::String(String::from("usage-based-routing")))
    );
    assert_eq!(
        extra.get("enable_tag_filtering"),
        Some(&JsonValue::Bool(true))
    );
}

#[tokio::test]
async fn entry_override_keeps_existing_key() {
    let trace = Trace::default();
    let calls = Calls::default();
    let ports = ports_with(Some(FakeView::default()), Vec::new(), &calls, &trace);
    let input = PassthroughRouteInput {
        router_settings_override: Some(override_settings()),
        ..route_input("gpt-upstream")
    };
    let mut args = call_args("gpt-upstream");
    args.extra.insert(
        String::from("fallbacks"),
        JsonValue::String(String::from("client")),
    );
    args.extra
        .insert(String::from("timeout"), JsonValue::from(5_u32));
    let request = request(args, input, &ports);
    entry::execute(request).await.expect("router reply");
    let extra = &calls.recorded()[0].extra;
    assert_eq!(
        extra.get("fallbacks"),
        Some(&JsonValue::String(String::from("client")))
    );
    assert_eq!(extra.get("timeout"), Some(&JsonValue::from(5_u32)));
    assert_eq!(extra.get("num_retries"), Some(&JsonValue::from(3_u32)));
}

#[tokio::test]
async fn entry_override_skipped_with_api_key() {
    let trace = Trace::default();
    let calls = Calls::default();
    let ports = ports_with(Some(FakeView::default()), Vec::new(), &calls, &trace);
    let input = PassthroughRouteInput {
        has_api_key_or_base: RouteFlag(true),
        router_settings_override: Some(override_settings()),
        ..route_input("gpt-upstream")
    };
    let request = request(call_args("gpt-upstream"), input, &ports);
    entry::execute(request).await.expect("router reply");
    assert_eq!(trace.steps(), ["router"]);
    assert!(calls.recorded()[0].extra.is_empty());
}

#[tokio::test]
async fn entry_override_reaches_direct() {
    let trace = Trace::default();
    let calls = Calls::default();
    let ports = ports_with(None, Vec::new(), &calls, &trace);
    let input = PassthroughRouteInput {
        router_settings_override: Some(override_settings()),
        ..route_input("gpt-upstream")
    };
    let request = request(call_args("gpt-upstream"), input, &ports);
    entry::execute(request).await.expect("direct reply");
    assert_eq!(trace.steps(), ["direct"]);
    assert_eq!(calls.recorded()[0].extra.len(), 8);
}

#[tokio::test]
async fn entry_team_model_written() {
    let trace = Trace::default();
    let calls = Calls::default();
    let view = FakeView {
        team_map: vec![("team-model", "team-1", "global-model")],
        ..empty_view()
    };
    let ports = ports_with(Some(view), Vec::new(), &calls, &trace);
    let input = PassthroughRouteInput {
        team_id: Some(String::from("team-1")),
        ..route_input("team-model")
    };
    let request = request(call_args("team-model"), input, &ports);
    entry::execute(request).await.expect("router reply");
    assert_eq!(calls.recorded()[0].model, "global-model");
}

#[tokio::test]
async fn entry_api_key_keeps_model() {
    let trace = Trace::default();
    let calls = Calls::default();
    let view = FakeView {
        team_map: vec![("team-model", "team-1", "global-model")],
        ..empty_view()
    };
    let ports = ports_with(Some(view), Vec::new(), &calls, &trace);
    let input = PassthroughRouteInput {
        team_id: Some(String::from("team-1")),
        has_api_key_or_base: RouteFlag(true),
        ..route_input("team-model")
    };
    let request = request(call_args("team-model"), input, &ports);
    entry::execute(request).await.expect("router reply");
    assert_eq!(calls.recorded()[0].model, "team-model");
}

#[tokio::test]
async fn entry_override_keeps_model() {
    let trace = Trace::default();
    let calls = Calls::default();
    let view = FakeView {
        team_map: vec![("team-model", "team-1", "global-model")],
        ..empty_view()
    };
    let ports = ports_with(Some(view), Vec::new(), &calls, &trace);
    let input = PassthroughRouteInput {
        team_id: Some(String::from("team-1")),
        router_settings_override: Some(override_settings()),
        ..route_input("team-model")
    };
    let request = request(call_args("team-model"), input, &ports);
    entry::execute(request).await.expect("router reply");
    assert_eq!(calls.recorded()[0].model, "team-model");
}

#[tokio::test]
async fn entry_specific_deployment_in_extra() {
    let trace = Trace::default();
    let calls = Calls::default();
    let view = FakeView {
        deployment_names: vec!["deployment-7"],
        ..empty_view()
    };
    let ports = ports_with(Some(view), Vec::new(), &calls, &trace);
    let request = request(
        call_args("deployment-7"),
        route_input("deployment-7"),
        &ports,
    );
    entry::execute(request).await.expect("router reply");
    assert_eq!(
        calls.recorded()[0].extra.get("specific_deployment"),
        Some(&JsonValue::Bool(true))
    );
}

#[tokio::test]
async fn entry_specific_deployment_overwrites() {
    let trace = Trace::default();
    let calls = Calls::default();
    let view = FakeView {
        deployment_names: vec!["deployment-7"],
        ..empty_view()
    };
    let ports = ports_with(Some(view), Vec::new(), &calls, &trace);
    let mut args = call_args("deployment-7");
    args.extra
        .insert(String::from("specific_deployment"), JsonValue::Bool(false));
    let request = request(args, route_input("deployment-7"), &ports);
    entry::execute(request).await.expect("router reply");
    assert_eq!(
        calls.recorded()[0].extra.get("specific_deployment"),
        Some(&JsonValue::Bool(true))
    );
}

#[tokio::test]
async fn entry_router_plan_keeps_specific_absent() {
    let trace = Trace::default();
    let calls = Calls::default();
    let view = FakeView {
        recognized: RouteFlag(true),
        ..empty_view()
    };
    let ports = ports_with(Some(view), Vec::new(), &calls, &trace);
    let request = request(
        call_args("gpt-upstream"),
        route_input("gpt-upstream"),
        &ports,
    );
    entry::execute(request).await.expect("router reply");
    assert_eq!(calls.recorded()[0].extra.get("specific_deployment"), None);
}

#[tokio::test]
async fn entry_blocked_is_403() {
    let trace = Trace::default();
    let calls = Calls::default();
    let view = FakeView::default();
    let ports = ports_with(Some(view), vec!["gpt-upstream"], &calls, &trace);
    let request = request(
        call_args("gpt-upstream"),
        route_input("gpt-upstream"),
        &ports,
    );
    let Err(error) = entry::execute(request).await else {
        panic!("a blocked model is refused");
    };
    let LlmPassthroughError::Blocked(blocked) = error else {
        panic!("a blocked model answers Blocked");
    };
    assert_eq!(blocked.model, "gpt-upstream");
    assert_eq!(blocked.to_string(), "Model is blocked");
    assert_eq!(ModelBlockedError::STATUS.as_u16(), 403);
    assert_eq!(trace.steps(), ["blocked:gpt-upstream:"]);
    assert!(calls.recorded().is_empty());
}

#[tokio::test]
async fn entry_not_found_names_route() {
    let trace = Trace::default();
    let calls = Calls::default();
    let ports = ports_with(Some(FakeView::default()), Vec::new(), &calls, &trace);
    let request = request(call_args("missing"), route_input("missing"), &ports);
    let Err(LlmPassthroughError::NotFound(error)) = entry::execute(request).await else {
        panic!("an unknown model is not found");
    };
    let message = &error.detail.error;
    assert!(
        message.contains("allm_passthrough_route: Invalid model name passed in model=missing.")
    );
    assert!(error.retryable_with_model_read_through);
    assert_eq!(trace.steps(), ["blocked:missing:"]);
    assert!(calls.recorded().is_empty());
}

#[tokio::test]
async fn entry_user_config_is_rejected() {
    let trace = Trace::default();
    let calls = Calls::default();
    let ports = ports_with(Some(FakeView::default()), Vec::new(), &calls, &trace);
    let input = PassthroughRouteInput {
        has_user_config: RouteFlag(true),
        ..route_input("gpt-upstream")
    };
    let request = request(call_args("gpt-upstream"), input, &ports);
    let Err(error) = entry::execute(request).await else {
        panic!("a user_config request is refused");
    };
    assert!(matches!(error, LlmPassthroughError::UserConfig));
    let DispatchError::Rejected {
        status,
        error_type,
        message,
        ..
    } = error.rejected()
    else {
        panic!("the dispatch answer is a rejected call");
    };
    assert_eq!(
        (status, error_type.as_str()),
        (400, "invalid_request_error")
    );
    assert_eq!(
        message,
        "user_config is not supported on pass-through routes"
    );
}

fn rejected_parts(error: &LlmPassthroughError) -> (u16, String, String) {
    let DispatchError::Rejected {
        status,
        error_type,
        code,
        ..
    } = error.rejected()
    else {
        panic!("the dispatch answer is a rejected call");
    };
    (status, error_type, code)
}

fn provider_failure(status: u16) -> LlmPassthroughError {
    LlmPassthroughError::Call(PassthroughCallError::Mapped(Box::new(ProviderError::from(
        ProviderErrorParts {
            status_code: status,
            message: String::from("upstream failed"),
            headers: None,
            request: None,
            response: None,
            body: None,
            status_code_is_synthesized: false,
        },
    ))))
}

async fn rejected_call(status: u16) -> (u16, String) {
    let trace = Trace::default();
    let calls = Calls::default();
    let request = failing_router_request(status, &trace, &calls);
    let Err(error) = entry::execute(request).await else {
        panic!("a failed call reaches the caller");
    };
    let (http_status, error_type, _code) = rejected_parts(&error);
    (http_status, error_type)
}

#[tokio::test]
async fn entry_call_error_wraps() {
    let trace = Trace::default();
    let calls = Calls::default();
    let request = failing_router_request(418, &trace, &calls);
    let Err(LlmPassthroughError::Call(error)) = entry::execute(request).await else {
        panic!("a failed call reaches the caller");
    };
    assert!(matches!(
        error,
        PassthroughCallError::UpstreamStatus { status: 418, .. }
    ));
    let (status, error_type, code) = rejected_parts(&LlmPassthroughError::Call(error));
    assert_eq!(
        (status, error_type.as_str(), code.as_str()),
        (418, "invalid_request_error", "418")
    );
}

#[tokio::test]
async fn entry_call_error_types_follow_status() {
    let authentication = rejected_call(401).await;
    let permission = rejected_call(403).await;
    let rate_limit = rejected_call(429).await;
    let server = rejected_call(503).await;
    assert_eq!(authentication, (401, String::from("authentication_error")));
    assert_eq!(permission, (403, String::from("permission_error")));
    assert_eq!(rate_limit, (429, String::from("rate_limit_error")));
    assert_eq!(server, (503, String::from("internal_server_error")));
}

#[test]
fn entry_mapped_call_error_keeps_status() {
    let (status, error_type, _code) = rejected_parts(&provider_failure(429));
    assert_eq!((status, error_type.as_str()), (429, "rate_limit_error"));
}

#[test]
fn entry_mapped_call_error_outside_range_is_500() {
    let (status, error_type, _code) = rejected_parts(&provider_failure(302));
    assert_eq!(
        (status, error_type.as_str()),
        (500, "internal_server_error")
    );
}

#[test]
fn entry_resolution_call_error_is_400() {
    let error = LlmPassthroughError::Call(PassthroughCallError::Resolution(
        ResolutionError::MissingProvider {
            model: String::from("foo"),
        },
    ));
    let (status, error_type, _code) = rejected_parts(&error);
    assert_eq!(
        (status, error_type.as_str()),
        (400, "invalid_request_error")
    );
}

#[test]
fn entry_mantle_endpoint_call_error_is_400() {
    let error = LlmPassthroughError::Call(PassthroughCallError::MantleEndpoint(
        ConfigError::ValueError {
            message: String::from("invalid AWS region format: Not_A_Region"),
        },
    ));
    let (status, error_type, _code) = rejected_parts(&error);
    assert_eq!(
        (status, error_type.as_str()),
        (400, "invalid_request_error")
    );
}

#[tokio::test]
async fn entry_router_missing() {
    let trace = Trace::default();
    let calls = Calls::default();
    let view = FakeView {
        recognized: RouteFlag(true),
        ..empty_view()
    };
    let mut ports = ports_with(Some(view), Vec::new(), &calls, &trace);
    ports.router = None;
    let request = request(
        call_args("gpt-upstream"),
        route_input("gpt-upstream"),
        &ports,
    );
    let Err(error) = entry::execute(request).await else {
        panic!("a router plan without a router fails");
    };
    assert!(matches!(error, LlmPassthroughError::RouterMissing));
    assert!(calls.recorded().is_empty());
}

#[tokio::test]
async fn entry_router_uses_b8690_trait() {
    let trace = Trace::default();
    let calls = Calls::default();
    let view = FakeView {
        recognized: RouteFlag(true),
        ..empty_view()
    };
    let mut ports = ports_with(Some(view), Vec::new(), &calls, &trace);
    ports.router = Some(sdk_router(&trace));
    let request = request(
        call_args("gpt-upstream"),
        route_input("gpt-upstream"),
        &ports,
    );
    let reply = entry::execute(request).await.expect("sdk reply");
    assert_eq!(reply.status, 200);
    assert_eq!(trace.steps(), ["blocked:gpt-upstream:", "sdk"]);
}

#[tokio::test]
async fn entry_dispatch_blocked_is_403() {
    let trace = Trace::default();
    let calls = Calls::default();
    let view = FakeView::default();
    let ports = ports_with(Some(view), vec!["gpt-upstream"], &calls, &trace);
    let request = request(
        call_args("gpt-upstream"),
        route_input("gpt-upstream"),
        &ports,
    );
    let dispatch = Dispatch::new(Vec::new(), Arc::new(NoRules), Arc::new(TracingCallLog));
    let context = context(None, SystemTime::UNIX_EPOCH);
    let operation = Operation::LlmPassthrough(request);
    let result = dispatch.execute(operation, &context).await;
    let Err(DispatchError::Rejected {
        status,
        error_type,
        param,
        code,
        message,
    }) = result
    else {
        panic!("a blocked model is a rejected call");
    };
    assert_eq!(status, 403);
    assert_eq!(error_type, "permission_error");
    assert_eq!(param, None);
    assert_eq!(code, "403");
    assert_eq!(message, "Model is blocked");
}

#[tokio::test]
async fn entry_dispatch_not_found_is_400() {
    let trace = Trace::default();
    let calls = Calls::default();
    let ports = ports_with(Some(FakeView::default()), Vec::new(), &calls, &trace);
    let request = request(call_args("missing"), route_input("missing"), &ports);
    let dispatch = Dispatch::new(Vec::new(), Arc::new(NoRules), Arc::new(TracingCallLog));
    let context = context(None, SystemTime::UNIX_EPOCH);
    let operation = Operation::LlmPassthrough(request);
    let result = dispatch.execute(operation, &context).await;
    let Err(DispatchError::Rejected {
        status,
        error_type,
        param,
        code,
        message,
    }) = result
    else {
        panic!("an unknown model is a rejected call");
    };
    assert_eq!(status, 400);
    assert_eq!(error_type, "invalid_request_error");
    assert_eq!(param, None);
    assert_eq!(code, "400");
    assert!(
        message.contains("allm_passthrough_route: Invalid model name passed in model=missing.")
    );
}
