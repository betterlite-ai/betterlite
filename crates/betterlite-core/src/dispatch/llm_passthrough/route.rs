use betterlite_openai::openai_base_config::Object;

use super::BlockedModels;
use crate::api::video_router_calls::route::VideoRouteView;

// A bool in the route input; the newtype keeps struct_excessive_bools off a record that declares three
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RouteFlag(pub bool);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PassthroughRouteInput {
    pub model: String,
    pub team_id: Option<String>,
    pub is_proxy_admin_without_team: RouteFlag,
    pub has_api_key_or_base: RouteFlag,
    pub has_user_config: RouteFlag,
    pub router_settings_override: Option<Object>,
    pub user_model: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PassthroughRoute {
    Router { specific_deployment: bool },
    Direct,
    UserConfig,
    Blocked,
    NotFound,
}

// route_llm_request's passthrough branches in LiteLLM's order, up to the router choice
#[must_use]
pub fn plan_passthrough_route(
    input: &PassthroughRouteInput,
    view: Option<&(dyn VideoRouteView + Send + Sync)>,
    blocked: &dyn BlockedModels,
) -> PassthroughRoute {
    if input.has_api_key_or_base.0 {
        return callee(view);
    }
    if input.has_user_config.0 {
        return PassthroughRoute::UserConfig;
    }
    if input.router_settings_override.is_some() {
        return callee(view);
    }
    let Some(router_view) = view else {
        return PassthroughRoute::Direct;
    };
    plan_with_router(input, router_view, blocked)
}

const fn callee(view: Option<&(dyn VideoRouteView + Send + Sync)>) -> PassthroughRoute {
    if view.is_some() {
        return PassthroughRoute::Router {
            specific_deployment: false,
        };
    }
    PassthroughRoute::Direct
}

fn plan_with_router(
    input: &PassthroughRouteInput,
    view: &(dyn VideoRouteView + Send + Sync),
    blocked: &dyn BlockedModels,
) -> PassthroughRoute {
    // _raise_if_model_fully_blocked skips an empty model and runs before the team mapping
    if !input.model.is_empty() && blocked.all_blocked(&input.model, input.team_id.as_deref()) {
        return PassthroughRoute::Blocked;
    }
    if let Some(team_id) = input.team_id.as_deref()
        && view.map_team_model(&input.model, team_id).is_some()
    {
        return PassthroughRoute::Router {
            specific_deployment: false,
        };
    }
    if view.is_recognized_model(&input.model)
        || (input.is_proxy_admin_without_team.0
            && !view.model_names_has(&input.model)
            && view.team_public_model_names_has(&input.model))
    {
        return PassthroughRoute::Router {
            specific_deployment: false,
        };
    }
    if !view.model_names_has(&input.model) {
        return plan_unlisted(input, view);
    }
    PassthroughRoute::NotFound
}

fn plan_unlisted(
    input: &PassthroughRouteInput,
    view: &(dyn VideoRouteView + Send + Sync),
) -> PassthroughRoute {
    if view.pass_through_all_models() {
        return PassthroughRoute::Direct;
    }
    if view.has_default_or_pattern() {
        return PassthroughRoute::Router {
            specific_deployment: false,
        };
    }
    if view.deployment_names_has(&input.model) {
        return PassthroughRoute::Router {
            specific_deployment: true,
        };
    }
    PassthroughRoute::NotFound
}
