use std::sync::Arc;

use axum::body::{Body, Bytes};
use axum::http::{Request, header};
use betterlite_auth::auth_request_policy::ip::RequestProvenance;
use betterlite_auth::auth_request_policy::policy::{PreDbRequest, pre_db_read_auth_checks};
use betterlite_auth::auth_route_patterns::canonical::{
    RequestRouteSource, ScopeFields, get_request_route,
};
use betterlite_auth::request_body_safety::RequestBodySafety;
use betterlite_error::proxy_exception::ProxyException;

use crate::auth::auth_identity::{AuthContext, StageFailure};
use crate::auth::request_body_safety::{CoreMetadata, request_model, request_object};
use crate::auth::route_match::decoded_route_path;
use crate::error_body::AuthenticationFailure;
use crate::state::ServerState;
use crate::state::auth_request_policy::Slot;

mod tests;

#[derive(Clone, Debug)]
pub struct AuthRequestPolicyStageInput {
    pub context: AuthContext,
    pub content_length: Option<String>,
    pub forwarded_for: Option<String>,
    pub client_host: Option<String>,
    pub route: String,
    pub content_type: Option<String>,
    pub body: Bytes,
    pub slot: Option<Slot>,
}

pub(crate) fn stage_input(
    request: &Request<Body>,
    authenticated: Option<&AuthContext>,
    body: Option<&Bytes>,
) -> Result<AuthRequestPolicyStageInput, StageFailure> {
    let context = authenticated
        .cloned()
        .ok_or(AuthenticationFailure::Missing)?;
    let content_length = request
        .headers()
        .get(header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let forwarded_for = request
        .headers()
        .get("x-forwarded-for")
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let content_type = request
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let decoded = decoded_route_path(request.uri().path());
    let path = match &decoded {
        Some(path) => path.as_str(),
        None => request.uri().path(),
    };
    let route = get_request_route(&RequestRouteSource {
        scope: Ok(Some(ScopeFields {
            path: Some(path),
            app_root_path: None,
            root_path: None,
        })),
        url_path: path,
    });
    Ok(AuthRequestPolicyStageInput {
        context,
        content_length,
        forwarded_for,
        // The connection layer keeps the peer address to itself, so the forwarded header is the only client source.
        client_host: None,
        route,
        content_type,
        body: body.cloned().unwrap_or_default(),
        slot: request
            .extensions()
            .get::<Arc<ServerState>>()
            .map(|state| state.auth_request_policy.clone()),
    })
}

pub(crate) fn run_stage(
    stage: &AuthRequestPolicyStageInput,
) -> Result<AuthContext, Box<ProxyException>> {
    let Some(slot) = &stage.slot else {
        return Ok(stage.context.clone());
    };
    let content_type = stage.content_type.as_deref().unwrap_or_default();
    let body = request_object(&stage.body, content_type);
    let model = body.as_ref().map_or("", request_model);
    let safety = body.as_ref().map(|object| RequestBodySafety {
        body: object,
        model,
        allow_client_side_credentials: slot.body_safety.settings.allow_client_side_credentials,
        allowed_hosts: &slot.body_safety.settings.allowed_hosts,
        grants: slot,
        metadata: &CoreMetadata,
        max_fallbacks: slot.body_safety.settings.max_fallbacks,
    });
    let request = PreDbRequest {
        content_length: stage.content_length.as_deref(),
        body: &stage.body,
        route: stage.route.as_str(),
        provenance: RequestProvenance {
            forwarded_for: stage.forwarded_for.as_deref(),
            client_host: stage.client_host.as_deref(),
        },
        body_safety: safety,
    };
    pre_db_read_auth_checks(request, &slot.settings)
        .map_err(|error| Box::new(error.into_proxy_exception()))?;
    Ok(stage.context.clone())
}
