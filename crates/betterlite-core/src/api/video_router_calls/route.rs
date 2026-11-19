use super::types::VideoCallKind;

pub trait VideoRouteView {
    fn is_recognized_model(&self, model: &str) -> bool;

    fn map_team_model(&self, model: &str, team_id: &str) -> Option<String>;

    fn team_public_model_names_has(&self, model: &str) -> bool;

    fn model_names_has(&self, model: &str) -> bool;

    fn pass_through_all_models(&self) -> bool;

    fn has_default_or_pattern(&self) -> bool;

    fn deployment_names_has(&self, model: &str) -> bool;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VideoRouteInput<'request> {
    pub kind: VideoCallKind,
    pub model: Option<&'request str>,
    pub team_id: Option<&'request str>,
    pub is_proxy_admin_without_team: bool,
    pub has_api_key_or_base: bool,
    pub user_model: Option<&'request str>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VideoRoute {
    Direct,
    Router {
        model: String,
        specific_deployment: bool,
    },
    NotFound {
        model: String,
    },
}

// route_llm_request's video branches in LiteLLM's order: the api_key shortcut first, then the router checks
#[must_use]
pub fn plan_video_route(
    input: &VideoRouteInput<'_>,
    view: Option<&dyn VideoRouteView>,
) -> VideoRoute {
    let model = input.model.unwrap_or_default();
    if input.has_api_key_or_base {
        return if view.is_some() {
            VideoRoute::Router {
                model: model.to_owned(),
                specific_deployment: false,
            }
        } else {
            VideoRoute::Direct
        };
    }
    let Some(view) = view else {
        return if input.user_model.is_some() {
            VideoRoute::Direct
        } else {
            VideoRoute::NotFound {
                model: model.to_owned(),
            }
        };
    };
    plan_with_router(input, view)
}

fn plan_with_router(input: &VideoRouteInput<'_>, view: &dyn VideoRouteView) -> VideoRoute {
    if input.kind != VideoCallKind::Generation && input.model.is_none_or(str::is_empty) {
        return VideoRoute::Direct;
    }
    // LiteLLM indexes data["model"] from here on, so a missing name is a 500 there and a not-found here
    let Some(model) = input.model else {
        return VideoRoute::NotFound {
            model: String::new(),
        };
    };
    plan_named_model(input, model, view)
}

fn plan_named_model(
    input: &VideoRouteInput<'_>,
    model: &str,
    view: &dyn VideoRouteView,
) -> VideoRoute {
    if let Some(team_id) = input.team_id
        && let Some(mapped) = view.map_team_model(model, team_id)
    {
        return VideoRoute::Router {
            model: mapped,
            specific_deployment: false,
        };
    }
    if view.is_recognized_model(model)
        || (input.is_proxy_admin_without_team
            && !view.model_names_has(model)
            && view.team_public_model_names_has(model))
    {
        return VideoRoute::Router {
            model: model.to_owned(),
            specific_deployment: false,
        };
    }
    if !view.model_names_has(model) {
        return plan_unlisted_model(input.kind, model, view);
    }
    VideoRoute::NotFound {
        model: model.to_owned(),
    }
}

fn plan_unlisted_model(kind: VideoCallKind, model: &str, view: &dyn VideoRouteView) -> VideoRoute {
    if view.pass_through_all_models() {
        return VideoRoute::Direct;
    }
    if view.has_default_or_pattern() {
        return VideoRoute::Router {
            model: model.to_owned(),
            specific_deployment: false,
        };
    }
    if view.deployment_names_has(model) {
        return VideoRoute::Router {
            model: model.to_owned(),
            specific_deployment: true,
        };
    }
    // LiteLLM's try/except around this router call never catches: async_wrapper returns an unawaited coroutine
    if !matches!(kind, VideoCallKind::Generation | VideoCallKind::List) {
        return VideoRoute::Router {
            model: model.to_owned(),
            specific_deployment: false,
        };
    }
    VideoRoute::NotFound {
        model: model.to_owned(),
    }
}
