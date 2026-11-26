use betterlite_auth::auth_context_sources::context::{
    ModelContext, context_failure, get_model_from_request_context,
};
use betterlite_auth::auth_request_models::catalog::Collaborators;
use betterlite_auth::auth_request_models::sources::DispatchContext;
use betterlite_error::proxy_exception::ProxyException;

use super::auth_identity::AuthContext;
use super::auth_request_models::RequestModelStageInput;

mod tests;

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
