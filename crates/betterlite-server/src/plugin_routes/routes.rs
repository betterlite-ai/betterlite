use std::sync::PoisonError;

use axum::Json;
use axum::extract::{Extension, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse as _, Response};
use betterlite_auth::auth_identity::records::Principal;
use betterlite_auth::auth_identity::roles::Role;
use betterlite_config::loader::startup::process_environment;
use serde::{Deserialize, Serialize};

use super::claim::{SALT_VARIABLE, issue_plugin_session_claim};
use super::proxy::PluginState;
use crate::auth::auth_identity::AuthContext;
use crate::tracing_routes::router::unauthenticated;

const DEFAULT_PLUGIN_NAME: &str = "litellm-platform-plugin";
const SALT_MISSING_DETAIL: &str =
    "LITELLM_SALT_KEY is not configured; plugin iframe auth unavailable.";
pub(crate) const PROXY_ADMIN_ROLE: &str = "proxy_admin";
const PROXY_ADMIN_VIEWER_ROLE: &str = "proxy_admin_viewer";
const ORG_ADMIN_ROLE: &str = "org_admin";
const INTERNAL_USER_ROLE: &str = "internal_user";

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Identity {
    pub user_id: Option<String>,
    pub user_role: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct KeyIdentityView {
    user_id: Option<String>,
    user_role: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct AuthTokenQuery {
    plugin_name: Option<String>,
}

#[derive(Serialize)]
struct PluginEntry {
    name: String,
    display_name: String,
    url: String,
}

#[derive(Serialize)]
struct DetailBody {
    detail: String,
}

#[derive(Serialize)]
struct SessionClaimBody {
    session_claim: String,
}

// the identity LiteLLM reads off user_api_key_dict, from the caller's key or its user record
pub(crate) fn caller_identity(principal: &Principal) -> Result<Identity, serde_json::Error> {
    let view = key_identity(principal)?;
    let key_user_id = view.as_ref().and_then(|key| key.user_id.clone());
    let user_id = key_user_id.or_else(|| principal.user.as_ref().map(|user| user.id.clone()));
    let user_role = view
        .as_ref()
        .and_then(|key| key.user_role.clone())
        .unwrap_or_else(|| fallback_role(&principal.roles).to_owned());
    Ok(Identity {
        user_id,
        user_role: Some(user_role),
    })
}

fn key_identity(principal: &Principal) -> Result<Option<KeyIdentityView>, serde_json::Error> {
    let Some(key) = principal.source_key.0.as_ref() else {
        return Ok(None);
    };
    serde_json::from_value(serde_json::to_value(key)?).map(Some)
}

fn fallback_role(roles: &[Role]) -> &'static str {
    if roles.contains(&Role::PlatformAdmin) {
        PROXY_ADMIN_ROLE
    } else if roles.contains(&Role::PlatformViewer) {
        PROXY_ADMIN_VIEWER_ROLE
    } else if roles.contains(&Role::OrgAdmin) {
        ORG_ADMIN_ROLE
    } else {
        INTERNAL_USER_ROLE
    }
}

pub(crate) async fn list_plugins(
    State(state): State<PluginState>,
    caller: Option<Extension<AuthContext>>,
) -> Response {
    if caller.is_none() {
        return unauthenticated();
    }
    let entries: Vec<PluginEntry> = {
        let plugins = state
            .registry
            .plugins
            .read()
            .unwrap_or_else(PoisonError::into_inner);
        plugins
            .values()
            .map(|plugin| PluginEntry {
                name: plugin.name.clone(),
                display_name: plugin
                    .display_name
                    .clone()
                    .unwrap_or_else(|| plugin.name.clone()),
                url: plugin.url.clone(),
            })
            .collect()
    };
    Json(entries).into_response()
}

pub(crate) async fn plugin_auth_token(
    State(state): State<PluginState>,
    caller: Option<Extension<AuthContext>>,
    Query(query): Query<AuthTokenQuery>,
) -> Response {
    let Some(authenticated) = caller else {
        return unauthenticated();
    };
    // the salt is checked before the plugin name, so an unset salt answers 503 even for an unknown plugin
    let missing_salt = process_environment()
        .get(SALT_VARIABLE)
        .is_none_or(String::is_empty);
    if missing_salt {
        return json_detail(
            StatusCode::SERVICE_UNAVAILABLE,
            SALT_MISSING_DETAIL.to_owned(),
        );
    }
    let plugin_name = query.plugin_name.as_deref().unwrap_or(DEFAULT_PLUGIN_NAME);
    let plugins = state
        .registry
        .plugins
        .read()
        .unwrap_or_else(PoisonError::into_inner);
    let registered = plugins.contains_key(plugin_name);
    drop(plugins);
    if !registered {
        return json_detail(
            StatusCode::NOT_FOUND,
            format!("Plugin '{plugin_name}' is not registered."),
        );
    }
    claim_response(&authenticated.principal, plugin_name)
}

fn claim_response(principal: &Principal, plugin_name: &str) -> Response {
    let identity = match caller_identity(principal) {
        Ok(identity) => identity,
        Err(error) => {
            return json_detail(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("The caller identity could not be read: {error}"),
            );
        }
    };
    match issue_plugin_session_claim(
        plugin_name,
        identity.user_id.as_deref(),
        identity.user_role.as_deref(),
    ) {
        Ok(session_claim) => {
            (StatusCode::OK, Json(SessionClaimBody { session_claim })).into_response()
        }
        Err(error) => json_detail(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("The plugin session claim could not be issued: {error}"),
        ),
    }
}

fn json_detail(status: StatusCode, detail: String) -> Response {
    let body = DetailBody { detail };
    let payload = Json(body);
    (status, payload).into_response()
}

mod tests;
