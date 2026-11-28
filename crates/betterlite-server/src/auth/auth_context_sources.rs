use std::collections::BTreeSet;
use std::sync::Arc;

use axum::body::{Body, Bytes};
use axum::http::Request;
use betterlite_auth::auth_context_sources::context::{
    ModelContext, context_failure, get_model_from_request_context,
};
use betterlite_auth::auth_request_models::catalog::{Collaborators, RequestModelResolver};
use betterlite_auth::auth_request_models::sources::DispatchContext;
use betterlite_core::api::request_body::RequestBodyInput;
use betterlite_error::proxy_exception::ProxyException;
use betterlite_models::model_registration::JsonObject;

use super::auth_identity::{AuthContext, StageFailure, StageInput};
use super::auth_key_lookup::key_headers::headers_json;
use super::auth_request_models::RequestModelStageInput;
use super::identity_store;
use super::query_json;
use crate::error_body::AuthenticationFailure;

pub mod alias;
pub mod rewrite;
mod selection;
mod tests;

// The rewrite step authenticates an LLM API request before buffering it, and the key lookup stage forwards this identity instead of reading the key again.
#[derive(Clone)]
pub(crate) struct PreAuthenticated(pub(crate) AuthContext);

pub(crate) fn key_lookup_stage_input(request: &Request<Body>) -> StageInput {
    request.extensions().get::<PreAuthenticated>().map_or_else(
        || identity_store::lookup_stage_input(request),
        |context| StageInput::Authenticated(Box::new(context.0.clone())),
    )
}

pub(crate) fn model_stage_input(
    request: &Request<Body>,
    context: AuthContext,
    resolver: &Arc<RequestModelResolver>,
) -> RequestModelStageInput {
    let route = request.uri().path().to_owned();
    let headers = Some(headers_json(request.headers()));
    let query = Some(query_json(request.uri().query()));
    let body = request
        .extensions()
        .get::<RequestBodyInput>()
        .and_then(|input| input.cached_body.as_ref())
        .and_then(|cached| serde_json::to_vec(&cached.visible()).ok())
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default();
    RequestModelStageInput {
        context,
        route,
        body,
        headers,
        query,
        endpoint_marker: None,
        path_params: BTreeSet::new(),
        team_id: None,
        resolver: Arc::clone(resolver),
    }
}

// The stage input builder: the request body joins the model context because the sources read it.
pub(crate) fn stage_input(
    request: &Request<Body>,
    authenticated: Option<&AuthContext>,
    resolver: &Arc<RequestModelResolver>,
    body: Option<&Bytes>,
) -> Result<StageInput, StageFailure> {
    let context = authenticated
        .cloned()
        .ok_or(AuthenticationFailure::Missing)?;
    let mut stage = model_stage_input(request, context, resolver);
    stage.body = body
        .and_then(|bytes| serde_json::from_slice::<JsonObject>(bytes).ok())
        .unwrap_or_default();
    Ok(StageInput::AuthContextSources(Box::new(stage)))
}

// The request's model context resolved at auth time; a catalog failure is the auth_error LiteLLM raises.
pub(crate) fn run_stage(
    stage: &RequestModelStageInput,
) -> Result<AuthContext, Box<ProxyException>> {
    let context = ModelContext {
        route: &stage.route,
        body: &stage.body,
        headers: stage.headers.as_ref(),
        query: stage.query.as_ref(),
        dispatch: DispatchContext {
            endpoint_marker: stage.endpoint_marker,
            path_params: &stage.path_params,
        },
        team_id: stage.team_id.as_deref(),
    };
    let collaborators = Collaborators {
        catalog: Some(stage.resolver.catalog.as_ref()),
        decoders: stage.resolver.decoders.as_ref(),
    };
    match get_model_from_request_context(&context, &collaborators) {
        Ok(_requested) => Ok(stage.context.clone()),
        Err(error) => {
            tracing::debug!(error = %error, "the auth context source stage failed");
            Err(Box::new(context_failure(&error)))
        }
    }
}
