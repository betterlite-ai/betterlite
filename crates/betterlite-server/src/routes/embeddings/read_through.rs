use axum::response::Response;
use betterlite_core::operation::embedding::EmbeddingRoute;
use betterlite_error::proxy_model_not_found::{ProxyModelNotFoundError, route_endpoint};
use betterlite_openai::chat::deployment::ChatDeployment;
use betterlite_openai::chat::provider_error::ProviderFailure;

use crate::route_read_through::route_with_read_through;
use crate::routing::simple_shuffle::pick_deployment;

use super::{EmbeddingState, render};

pub(super) struct NotFound {
    pub(super) message: String,
}

pub(super) async fn deployment_or_answer<'state>(
    state: &'state EmbeddingState,
    model: Option<&str>,
) -> Result<&'state ChatDeployment, NotFound> {
    let model_text = model.unwrap_or("None");
    let endpoint = route_endpoint("aembedding").unwrap_or("aembedding");
    let attempt = move || async move {
        pick_deployment(&state.executor, model_text)
            .map_err(|_error| ProxyModelNotFoundError::new(endpoint, model_text, true))
    };
    let read_through = state.read_through.as_deref();
    match route_with_read_through(model, read_through, attempt).await {
        Ok(deployment) => Ok(deployment),
        Err(error) => Err(NotFound {
            message: state
                .executor
                .redactor
                .redact_internal_details(&error.detail.error),
        }),
    }
}

pub(super) fn not_found_response(not_found: NotFound, call_id: &str) -> Response {
    let route = EmbeddingRoute {
        failure: ProviderFailure {
            status: ProxyModelNotFoundError::STATUS.as_u16(),
            error_type: String::from("invalid_request_error"),
            param: None,
        },
        message: not_found.message.clone(),
        upstream_headers: None,
    };
    render(route, call_id, None, Some(not_found.message))
}
