use std::fmt::Display;
use std::net::SocketAddr;
use std::sync::Arc;

use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::Request;
use betterlite_auth::auth_errors::conversion::{AuthFailure, as_proxy_exception};
use betterlite_auth::auth_identity::records::{
    CredentialRef, NetworkContext, Principal, SourceKey, TeamIdentity,
};
use betterlite_auth::auth_identity::roles::{AuthMethod, PrincipalType, TeamRole};
use betterlite_auth::auth_key_lookup::{key_identity, key_roles};
use betterlite_auth::auxiliary_routes::INFO_ROUTES;
use betterlite_auth::oauth_tokens::check::check_oauth2_token;
use betterlite_auth::oauth_tokens::proxy_identity::handle_oauth2_proxy_request;
use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_auth::resolved_key::credential::safe_hash_api_key;
use betterlite_auth::route_match::classifier::is_llm_api_route;
use betterlite_auth::route_match::templates::RouteValue;
use betterlite_auth::trusted_network::request::NetworkRequest;
use betterlite_config::sections::oauth_tokens::Settings;
use secrecy::{ExposeSecret as _, SecretString};

use crate::auth::auth_identity::{AuthContext, StageFailure};
use crate::auth::auth_key_lookup::key_headers::extract_key;
use crate::auth::route_match::decoded_route_path;
use crate::error_body::AuthenticationFailure;
use crate::state::ServerState;

mod tests;

pub(crate) enum PreStage {
    Skipped,
    Authenticated(AuthContext),
    Rejected(StageFailure),
}

// _user_api_key_auth_builder runs both OAuth branches before it reads the master key.
pub(crate) async fn pre_stage(request: &Request<Body>) -> PreStage {
    let Some(state) = request.extensions().get::<Arc<ServerState>>() else {
        return PreStage::Skipped;
    };
    let settings = state.oauth_tokens.settings.as_ref();
    let route =
        decoded_route_path(request.uri().path()).unwrap_or_else(|| request.uri().path().to_owned());
    let llm_or_info =
        is_llm_api_route(&RouteValue::Text(&route)) || INFO_ROUTES.contains(&route.as_str());
    if settings.enable_oauth2_auth && llm_or_info {
        return introspection(state, settings, request).await;
    }
    if settings.enable_oauth2_proxy_auth {
        return proxy(state, settings, request);
    }
    PreStage::Skipped
}

async fn introspection(
    state: &ServerState,
    settings: &Settings,
    request: &Request<Body>,
) -> PreStage {
    // LiteLLM introspects an absent token as the literal text None; betterlite refuses it here
    let Some(token) = extract_key(request.headers()) else {
        return PreStage::Rejected(StageFailure::from(AuthenticationFailure::Missing));
    };
    let transport = state.oauth_tokens.transport.as_ref();
    let result = check_oauth2_token(settings, transport, &token).await;
    stage_from_result(result, AuthMethod::Oauth2Introspection, Some(&token))
}

fn proxy(state: &ServerState, settings: &Settings, request: &Request<Body>) -> PreStage {
    let peer = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|connect_info| connect_info.0.ip().to_string());
    // the trust check reads the direct peer; the forwarded-header fields stay for the B743 stage
    let network = NetworkRequest {
        peer: peer.as_deref(),
        x_forwarded_for: None,
        x_forwarded_proto: None,
        host: None,
        scheme: "",
    };
    let trusted = state.trusted_network.settings.as_ref();
    let mappings = settings.config_mappings.as_ref();
    let result = handle_oauth2_proxy_request(request.headers(), mappings, &network, trusted);
    stage_from_result(result, AuthMethod::Oidc, None)
}

fn stage_from_result<E>(
    result: Result<UserAPIKeyAuth, E>,
    method: AuthMethod,
    token: Option<&SecretString>,
) -> PreStage
where
    E: Display,
{
    let auth = match result {
        Ok(auth) => auth,
        Err(error) => return PreStage::Rejected(value_error(&error)),
    };
    let hashed = token.map(|value| safe_hash_api_key(value.expose_secret()));
    PreStage::Authenticated(context_for(auth, method, hashed))
}

// auth_exception_handler._as_proxy_exception: an unhandled ValueError is this public 401 auth_error
fn value_error<E>(error: &E) -> StageFailure
where
    E: Display,
{
    let failure = AuthFailure::Other {
        message: error.to_string(),
        param: None,
    };
    StageFailure::OauthTokens(Box::new(as_proxy_exception(&failure).exception().clone()))
}

fn context_for(
    auth: UserAPIKeyAuth,
    method: AuthMethod,
    hashed_token: Option<String>,
) -> AuthContext {
    let identity = key_identity(&auth);
    let subject = identity
        .user_id
        .clone()
        .or_else(|| identity.key_name.clone())
        .unwrap_or_else(|| hashed_token.clone().unwrap_or_default());
    let teams = identity.team_id.clone().map_or_else(Vec::new, |id| {
        vec![TeamIdentity {
            id,
            name: identity.team_alias.clone(),
            role: TeamRole::Member,
        }]
    });
    AuthContext {
        principal: Principal {
            principal_type: PrincipalType::Human,
            subject,
            issuer: None,
            audience: Vec::new(),
            user: None,
            organization: None,
            teams,
            project: None,
            end_user: None,
            roles: key_roles(identity.user_role),
            scopes: Vec::new(),
            auth_method: method,
            credential_ref: CredentialRef {
                key_id: identity.key_name,
                token_id: hashed_token,
            },
            network: NetworkContext::default(),
            source_key: SourceKey(Some(auth)),
        },
    }
}
