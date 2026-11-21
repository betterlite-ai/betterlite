use betterlite_config::sections::settings::JsonValue;
use betterlite_error::proxy_model_not_found::{ProxyModelNotFoundError, route_endpoint};

use super::error::{LlmPassthroughError, ModelBlockedError};
use super::route::{PassthroughRoute, PassthroughRouteInput, plan_passthrough_route};
use crate::api::passthrough_call::{PassthroughCallArgs, PassthroughReply};
use crate::api::video_router_calls::route::VideoRouteView;
use crate::operation::llm_passthrough::LlmPassthroughRequest;

// The settings the router accepts as per-request kwargs, in LiteLLM's order
const OVERRIDE_KEYS: [&str; 8] = [
    "fallbacks",
    "context_window_fallbacks",
    "content_policy_fallbacks",
    "num_retries",
    "timeout",
    "model_group_retry_policy",
    "routing_strategy",
    "enable_tag_filtering",
];

// ROUTE_ENDPOINT_MAPPING has no entry for the passthrough route type, so the type is the name
const ROUTE_NAME: &str = "allm_passthrough_route";

pub async fn execute(
    request: LlmPassthroughRequest,
) -> Result<PassthroughReply, LlmPassthroughError> {
    let LlmPassthroughRequest {
        mut args,
        route: input,
        router,
        view,
        blocked,
        direct,
    } = request;
    let plan = plan_passthrough_route(&input, view.as_deref(), blocked.as_ref());
    merge_overrides(&mut args, &input);
    match plan {
        PassthroughRoute::UserConfig => Err(LlmPassthroughError::UserConfig),
        PassthroughRoute::Blocked => Err(LlmPassthroughError::Blocked(ModelBlockedError {
            model: input.model.clone(),
        })),
        PassthroughRoute::NotFound => {
            let name = route_endpoint(ROUTE_NAME).unwrap_or(ROUTE_NAME);
            let error = ProxyModelNotFoundError::new(name, &input.model, true);
            Err(LlmPassthroughError::NotFound(error))
        }
        PassthroughRoute::Router {
            specific_deployment,
        } => {
            let Some(port) = router else {
                return Err(LlmPassthroughError::RouterMissing);
            };
            router_args(&mut args, &input, view.as_deref(), specific_deployment);
            port.call(*args).await.map_err(LlmPassthroughError::Call)
        }
        PassthroughRoute::Direct => direct.call(*args).await.map_err(LlmPassthroughError::Call),
    }
}

// The router_settings_override branch never runs for a call that carries api_key, api_base or user_config
fn merge_overrides(args: &mut PassthroughCallArgs, input: &PassthroughRouteInput) {
    if input.has_api_key_or_base.0 || input.has_user_config.0 {
        return;
    }
    let Some(settings) = &input.router_settings_override else {
        return;
    };
    for key in OVERRIDE_KEYS {
        if let Some(value) = settings.get(key)
            && !args.extra.contains_key(key)
        {
            args.extra.insert(String::from(key), value.clone());
        }
    }
}

fn router_args(
    args: &mut PassthroughCallArgs,
    input: &PassthroughRouteInput,
    view: Option<&(dyn VideoRouteView + Send + Sync)>,
    specific_deployment: bool,
) {
    // The team mapping runs only where LiteLLM reaches the router, never on the api_key or override branches
    if input.router_settings_override.is_none()
        && !input.has_api_key_or_base.0
        && let Some(router_view) = view
        && let Some(team_id) = input.team_id.as_deref()
        && let Some(mapped) = router_view.map_team_model(&input.model, team_id)
    {
        args.model = mapped;
    }
    if specific_deployment {
        args.extra
            .insert(String::from("specific_deployment"), JsonValue::Bool(true));
    }
}
