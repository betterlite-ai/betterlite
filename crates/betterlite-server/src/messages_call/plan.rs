use betterlite_config::sections::settings::JsonValue;
use betterlite_openai::openai_base_config::Object as JsonMap;

use super::outcome::MessagesRouting;

// A bool in the route input; the newtype keeps struct_excessive_bools off a record that declares three
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RouteFlag(pub bool);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessagesRouteInput {
    pub model: Option<String>,
    pub has_model: bool,
    pub team_id: Option<String>,
    pub is_proxy_admin: bool,
    pub has_api_key_or_base: RouteFlag,
    pub has_user_config: RouteFlag,
    pub has_override: RouteFlag,
    pub user_model: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessagesRoute {
    Router { specific_deployment: bool },
    Direct,
    UserConfig,
    Blocked,
    A2aAgent,
    MissingModel,
    NotFound,
}

const TEAM_ID_KEY: &str = "user_api_key_team_id";

// get_team_id_from_data: metadata wins when it holds the key, litellm_metadata otherwise
#[must_use]
pub fn team_id_from_data(data: &JsonMap) -> Option<String> {
    let metadata = data.get("metadata").and_then(JsonValue::as_object);
    let source = if metadata.is_some_and(|object| object.contains_key(TEAM_ID_KEY)) {
        metadata
    } else {
        data.get("litellm_metadata").and_then(JsonValue::as_object)
    };
    source?.get(TEAM_ID_KEY)?.as_str().map(str::to_owned)
}

// _route_request_single_attempt's anthropic_messages branches in LiteLLM's order
#[must_use]
pub fn plan_messages_route(
    input: &MessagesRouteInput,
    view: Option<&dyn MessagesRouting>,
) -> MessagesRoute {
    if input.has_api_key_or_base.0 {
        return callee(view);
    }
    if input.has_user_config.0 {
        return MessagesRoute::UserConfig;
    }
    if input.has_override.0 {
        return callee(view);
    }
    let Some(routing) = view else {
        return if input.user_model.is_some() {
            MessagesRoute::Direct
        } else {
            MessagesRoute::NotFound
        };
    };
    plan_with_router(input, routing)
}

const fn callee(view: Option<&dyn MessagesRouting>) -> MessagesRoute {
    if view.is_some() {
        return MessagesRoute::Router {
            specific_deployment: false,
        };
    }
    MessagesRoute::Direct
}

fn plan_with_router(input: &MessagesRouteInput, routing: &dyn MessagesRouting) -> MessagesRoute {
    // _raise_if_model_fully_blocked skips an empty and a non-string model
    if let Some(model) = input.model.as_deref()
        && !model.is_empty()
        && routing.all_blocked(model, input.team_id.as_deref())
    {
        return MessagesRoute::Blocked;
    }
    // LiteLLM reads data["model"] past here, which is a KeyError on a body without one
    if !input.has_model {
        return MessagesRoute::MissingModel;
    }
    if let Some(team_id) = input.team_id.as_deref()
        && routing
            .map_team_model(input.model.as_deref(), team_id)
            .is_some()
    {
        return MessagesRoute::Router {
            specific_deployment: false,
        };
    }
    if admin_public_or_recognized(input, routing) {
        return MessagesRoute::Router {
            specific_deployment: false,
        };
    }
    let Some(model) = input.model.as_deref() else {
        // A model that is not a string compares as absent from every list
        return plan_unlisted(routing, None);
    };
    if routing.model_names_has(model) {
        return MessagesRoute::NotFound;
    }
    plan_unlisted(routing, Some(model))
}

fn admin_public_or_recognized(input: &MessagesRouteInput, routing: &dyn MessagesRouting) -> bool {
    let Some(model) = input.model.as_deref() else {
        return false;
    };
    routing.is_recognized_model(model)
        || (input.is_proxy_admin
            && input.team_id.is_none()
            && !routing.model_names_has(model)
            && routing.team_public_model_names_has(model))
}

fn plan_unlisted(routing: &dyn MessagesRouting, model: Option<&str>) -> MessagesRoute {
    if routing.pass_through_all_models() {
        return MessagesRoute::Direct;
    }
    if routing.has_default_or_pattern() {
        return MessagesRoute::Router {
            specific_deployment: false,
        };
    }
    let Some(model) = model else {
        return MessagesRoute::NotFound;
    };
    if routing.is_deployment_name(model) {
        return MessagesRoute::Router {
            specific_deployment: true,
        };
    }
    if model.starts_with("a2a/") {
        return MessagesRoute::A2aAgent;
    }
    MessagesRoute::NotFound
}
