use betterlite_models::decisions::Json;

use super::alias::resolve_model_group_alias;
use super::router_settings::{TeamRouterSettings, hierarchical_router_settings};

pub const MODEL_GROUP_ALIAS_RESOLVED_MARKER: &str = "litellm.model_group_alias_resolved";
pub const CLIENT_REQUESTED_MODEL_SCOPE_KEY: &str = "litellm.client_requested_model";

// Where the request stands before the rewrite: an ineligible route still reports the marker it found.
#[derive(Clone, Copy, Debug)]
pub enum RouterAliasState {
    Ineligible { already_resolved: bool },
    Resolved,
    Fresh,
}

pub struct RouterAliasInput<'request> {
    pub state: RouterAliasState,
    pub passthrough: bool,
    pub requested_model: Option<&'request str>,
    pub raw_body_available: bool,
    pub key_router_settings: Option<&'request Json>,
    pub team_id: Option<&'request str>,
    pub client_requested_model: Option<&'request str>,
}

// What the caller applies to the request: the marker, the setdefault value and the rewrite target.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RouterAliasRewrite {
    pub marker_set: bool,
    pub client_requested_model: Option<String>,
    pub target: Option<String>,
}

// _resolve_router_settings_model_group_alias up to the request mutation: the marker names the request
// before any early return, so a retry never gets a second rewrite opportunity.
pub async fn resolve_router_settings_model_group_alias(
    input: &RouterAliasInput<'_>,
    teams: &dyn TeamRouterSettings,
) -> RouterAliasRewrite {
    match input.state {
        RouterAliasState::Ineligible { already_resolved } => {
            return RouterAliasRewrite {
                marker_set: already_resolved,
                client_requested_model: None,
                target: None,
            };
        }
        RouterAliasState::Resolved => {
            return RouterAliasRewrite {
                marker_set: true,
                client_requested_model: None,
                target: None,
            };
        }
        RouterAliasState::Fresh => {}
    }
    let Some((requested, target)) = rewrite_target(input, teams).await else {
        return RouterAliasRewrite {
            marker_set: true,
            client_requested_model: None,
            target: None,
        };
    };
    let client_model = input.client_requested_model.unwrap_or(&requested);
    RouterAliasRewrite {
        marker_set: true,
        client_requested_model: Some(client_model.to_owned()),
        target: Some(target),
    }
}

async fn rewrite_target(
    input: &RouterAliasInput<'_>,
    teams: &dyn TeamRouterSettings,
) -> Option<(String, String)> {
    if input.passthrough {
        return None;
    }
    let requested = input.requested_model?;
    if !input.raw_body_available {
        return None;
    }
    let settings =
        hierarchical_router_settings(input.key_router_settings, input.team_id, teams).await?;
    let target = resolve_model_group_alias(settings.get("model_group_alias"), requested)?;
    if target == requested {
        return None;
    }
    // the debug line drops CR and LF so a hostile model name cannot forge a log line
    let logged_requested = requested.replace(['\r', '\n'], "");
    let logged_target = target.replace(['\r', '\n'], "");
    tracing::debug!(
        "router_settings.model_group_alias resolved {} -> {} before auth",
        logged_requested,
        logged_target
    );
    Some((requested.to_owned(), target))
}
