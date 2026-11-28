use std::collections::BTreeMap;

use betterlite_core::api::responses_mcp_followup::error::McpFollowupError;
use betterlite_core::api::responses_mcp_results::McpToolResult;
use betterlite_core::api::responses_mcp_tool_selection::McpToolCall;
use betterlite_core::api::responses_provider_selection::ParameterMap;
use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_objects::ResponsesApiResponse;
use betterlite_openai::response_input::ResponsesItem;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::HelperResult;
use serde_json::{Value, json};

use super::input::{RequestScenario, RequestSpec};
use super::support::{
    Provider, Settings, auth_user_id, create_runtime, empty_input, helper_error, minimal_response,
    run_service, service, service_input, stream_events, streaming_auth,
};

pub(super) fn request_params(scenarios: &[RequestScenario]) -> HelperResult {
    let answers: Result<Vec<Value>, Box<HelperError>> =
        scenarios.iter().map(request_scenario).collect();
    answers.map(Value::Array)
}

fn request_scenario(scenario: &RequestScenario) -> Result<Value, Box<HelperError>> {
    let mut input = service_input(&scenario.request, &scenario.call_params);
    input.kwargs = scenario.kwargs.clone();
    input.auth = streaming_auth(&scenario.auth)?;
    input.mcp_tools_with_litellm_proxy = scenario.mcp_tools.clone();
    input.mcp_discovery_events = stream_events(&scenario.mcp_events);
    input.tool_server_map = scenario.tool_server_map.clone();
    let settings = Settings {
        should_auto_execute: false,
        original_stream: true,
    };
    let provider = Provider::answering(minimal_response()?, None);
    let result = run_service(&input, &provider, &[], settings)?;
    let Some(plan) = result.streaming else {
        return Err(helper_error("the streaming branch must build a plan"));
    };
    Ok(json!({
        "request_params": plan.request_params,
        "auth_user_id": auth_user_id(plan.auth.as_ref()),
        "mcp_events": plan.mcp_events,
        "mcp_tools": plan.mcp_tools_with_litellm_proxy,
        "tool_server_map": plan.tool_server_map,
    }))
}

pub(super) fn tool_execution_events(
    request: &RequestSpec,
    tool_calls: &[BTreeMap<String, Json>],
    tool_results: &[McpToolResult],
    ids: &[String],
) -> HelperResult {
    let mut input = service_input(request, &ParameterMap::new());
    input.tool_calls = tool_calls
        .iter()
        .map(|fields| McpToolCall::Fields(fields.clone()))
        .collect();
    input.tool_results = tool_results.to_vec();
    let settings = Settings {
        should_auto_execute: false,
        original_stream: true,
    };
    let provider = Provider::answering(minimal_response()?, None);
    let result = run_service(&input, &provider, ids, settings)?;
    Ok(json!({ "events": result.events }))
}

pub(super) fn call_parameter_streaming(
    request: &RequestSpec,
    call_params: &ParameterMap,
    settings: Settings,
    ids: &[String],
) -> HelperResult {
    let input = service_input(request, call_params);
    let provider = Provider::answering(minimal_response()?, None);
    let result = run_service(&input, &provider, ids, settings)?;
    Ok(json!({
        "initial_params": result.initial_params,
        "follow_up_params": result.follow_up_params,
    }))
}

pub(super) fn append_output(
    response: &ResponsesApiResponse,
    mcp_tools_fetched: &[Json],
    tool_results: &[McpToolResult],
    ids: &[String],
) -> HelperResult {
    let mut input = empty_input();
    input.mcp_tools_fetched = mcp_tools_fetched.to_vec();
    input.tool_results = tool_results.to_vec();
    let settings = Settings {
        should_auto_execute: false,
        original_stream: false,
    };
    let provider = Provider::answering(response.clone(), None);
    let result = run_service(&input, &provider, ids, settings)?;
    let composed = result
        .response
        .ok_or_else(|| helper_error("the completed branch must return the response"))?;
    serde_json::to_value(composed.output).map_err(|error| helper_error(&error.to_string()))
}

pub(super) fn provider_call_error(
    request: &RequestSpec,
    call_params: &ParameterMap,
    follow_up_input: &[ResponsesItem],
    status: u16,
    ids: &[String],
) -> HelperResult {
    let mut input = service_input(request, call_params);
    input.follow_up_input = follow_up_input.to_vec();
    input.follow_up_response_id = request.previous_response_id.clone();
    let settings = Settings {
        should_auto_execute: true,
        original_stream: false,
    };
    let provider = Provider::answering(minimal_response()?, Some(status));
    let runtime = create_runtime()?;
    match runtime.block_on(service(&provider, ids, settings).run(&input)) {
        Err(McpFollowupError::Provider(failure)) => {
            let recorded = provider.calls()?;
            let Some(pending) = recorded.first() else {
                return Err(helper_error("the provider must receive the follow-up call"));
            };
            Ok(json!({
                "request": {
                    "input": pending.input,
                    "model": pending.model,
                    "tools": pending.tools,
                    "previous_response_id": pending.previous_response_id,
                    "call_params": pending.kwargs,
                },
                "error": { "status": failure.status },
            }))
        }
        Ok(_) => Err(helper_error("the follow-up call must fail")),
        Err(other) => Err(helper_error(&other.to_string())),
    }
}
