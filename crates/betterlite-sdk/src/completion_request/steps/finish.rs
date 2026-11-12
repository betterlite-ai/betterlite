use super::messages::{optional, text_of};
use crate::completion_request::Preparation;
use crate::completion_request::args::JsonObject;
use crate::completion_request::call::{MockCall, PreparedCompletion};
use crate::completion_request::error::RequestResult;
use crate::completion_request::ports::EnvironmentUpdate;
use betterlite_core::message_utilities::small_helpers::should_run_mock_completion;
use betterlite_models::controls::call_params::{
    DropParams, RequestCall, TrustedCall, call_params, with_control_options,
};
use betterlite_models::decisions::Json;
use betterlite_models::headers::affinity::SessionMetadata;
fn mcp_server_url(tool: &Json) -> Option<&str> {
    let Json::Object(fields) = tool else {
        return None;
    };
    if fields.get("type").and_then(text_of) != Some("mcp") {
        return None;
    }
    fields.get("server_url").and_then(text_of)
}
fn proxy_path_mcp_name(tool: &Json) -> Option<&str> {
    let url = mcp_server_url(tool)?;
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))?;
    let (host, name) = rest.rsplit_once("/mcp/")?;
    (!host.is_empty() && !name.is_empty() && !name.contains('/')).then_some(name)
}
pub(crate) fn should_use_mcp_gateway(tools: Option<&[Json]>) -> bool {
    tools.unwrap_or_default().iter().any(|tool| {
        mcp_server_url(tool).is_some_and(|url| url.starts_with("litellm_proxy"))
            || proxy_path_mcp_name(tool).is_some()
    })
}
fn admission_prompt_tokens(kwargs: &JsonObject) -> Option<i64> {
    // LiteLLM takes the first bucket that names input tokens, litellm_metadata before metadata
    ["litellm_metadata", "metadata"]
        .into_iter()
        .find_map(|name| {
            let Some(Json::Object(metadata)) = kwargs.get(name) else {
                return None;
            };
            let Some(Json::Object(reservation)) = metadata.get("user_api_key_budget_reservation")
            else {
                return None;
            };
            match reservation.get("input_tokens") {
                Some(Json::Number(raw)) => raw.as_i64(),
                None
                | Some(
                    Json::Null | Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_),
                ) => None,
            }
        })
}
fn should_run_mock(state: &Preparation<'_>) -> bool {
    let response = state
        .kwargs
        .get("mock_response")
        .cloned()
        .unwrap_or(Json::Null);
    let tool_calls = state
        .kwargs
        .get("mock_tool_calls")
        .cloned()
        .unwrap_or(Json::Null);
    let timeout = state
        .kwargs
        .get("mock_timeout")
        .cloned()
        .unwrap_or(Json::Null);
    should_run_mock_completion(&response, &tool_calls, &timeout)
}
pub(crate) fn litellm_params(state: &mut Preparation<'_>) -> RequestResult<()> {
    let request = RequestCall {
        session_id: optional::<String>(&state.kwargs, "session_id")?,
        litellm_session_id: optional::<String>(&state.kwargs, "litellm_session_id")?,
        litellm_trace_id: optional::<String>(&state.kwargs, "litellm_trace_id")?,
        metadata: optional::<SessionMetadata>(&state.kwargs, "metadata")?,
        litellm_metadata: optional::<SessionMetadata>(&state.kwargs, "litellm_metadata")?,
    };
    let trusted = TrustedCall {
        model: Some(state.call.model.clone()),
        api_key: state.call.api_key.clone(),
        api_base: state.call.api_base.clone(),
        custom_llm_provider: state.call.custom_llm_provider.clone(),
        control: state.control,
        credentials: None,
        drop_params: match state.kwargs.get("drop_params") {
            Some(Json::Bool(flag)) => Some(DropParams::Flag(*flag)),
            Some(Json::Text(text)) => Some(DropParams::Text(text.clone())),
            None | Some(Json::Null | Json::Number(_) | Json::List(_) | Json::Object(_)) => None,
        },
        no_log: optional::<bool>(&state.kwargs, "no-log")?,
        optional: state.settings.optional.clone(),
    };
    state.call.litellm_params = call_params(&trusted, &request);
    with_control_options(&mut state.call.litellm_params, state.control);
    Ok(())
}
pub(crate) fn environment(state: &Preparation<'_>) {
    state.ports.logging.update_environment(&EnvironmentUpdate {
        model: &state.call.model,
        user: state.args.user.as_deref(),
        optional_params: &state.call.non_default_params,
        litellm_params: &state.call.litellm_params,
        custom_llm_provider: state.call.custom_llm_provider.as_deref(),
    });
}
pub(crate) fn mock_or_bridge(
    state: &mut Preparation<'_>,
    provider: &str,
) -> RequestResult<PreparedCompletion> {
    if should_run_mock(state) {
        return Ok(PreparedCompletion::Mock(MockCall {
            call: state.call.clone(),
            acompletion: state.acompletion,
            mock_delay: state.kwargs.get("mock_delay").cloned(),
            prompt_tokens: admission_prompt_tokens(&state.kwargs),
        }));
    }
    if super::bridge::late(state, provider)? {
        return Ok(PreparedCompletion::ResponsesBridge(state.call.clone()));
    }
    Ok(PreparedCompletion::Dispatch(state.call.clone()))
}
