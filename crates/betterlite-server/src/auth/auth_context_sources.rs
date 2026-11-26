use std::sync::Arc;

use axum::body::{Body, Bytes};
use axum::http::Request;
use betterlite_auth::auth_context_sources::context::{
    ModelContext, context_failure, get_model_from_request_context,
};
use betterlite_auth::auth_request_models::catalog::{
    Collaborators, EmptyCatalog, RequestModelResolver,
};
use betterlite_auth::auth_request_models::sources::DispatchContext;
use betterlite_error::proxy_exception::ProxyException;
use betterlite_models::model_registration::JsonObject;

use super::auth_identity::{AuthContext, StageFailure, StageInput};
use super::auth_request_models::{CoreDecoders, RequestModelStageInput};
use super::model_stage_input;
use crate::error_body::AuthenticationFailure;

mod tests;

// The empty catalog a request that carries no resolver of its own reads the model context through.
pub(crate) fn default_resolver() -> Arc<RequestModelResolver> {
    Arc::new(RequestModelResolver {
        catalog: Arc::new(EmptyCatalog),
        decoders: Arc::new(CoreDecoders),
    })
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
