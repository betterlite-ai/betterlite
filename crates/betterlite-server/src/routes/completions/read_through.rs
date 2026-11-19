use betterlite_error::proxy_model_not_found::{ProxyModelNotFoundError, route_endpoint};
use betterlite_openai::chat::deployment::ChatDeployment;

use crate::route_read_through::route_with_read_through;
use crate::routing::simple_shuffle::pick_deployment;
use crate::state::ServerState;

use super::error::FailureAnswer;

pub(super) async fn deployment_or_answer<'state>(
    state: &'state ServerState,
    model: Option<&str>,
) -> Result<&'state ChatDeployment, FailureAnswer> {
    let model_text = model.unwrap_or("None");
    let route = route_endpoint("atext_completion").unwrap_or("atext_completion");
    let attempt = move || async move {
        pick_deployment(&state.executor, model_text)
            .map_err(|_error| ProxyModelNotFoundError::new(route, model_text, true))
    };
    let read_through = state.model_read_through.read_through.as_deref();
    match route_with_read_through(model, read_through, attempt).await {
        Ok(deployment) => Ok(deployment),
        Err(error) => {
            let message = state
                .executor
                .redactor
                .redact_internal_details(&error.detail.error);
            Err(FailureAnswer {
                status: ProxyModelNotFoundError::STATUS.as_u16(),
                error_type: Some(String::from("invalid_request_error")),
                param: None,
                message: message.clone(),
                provider_specific_fields: Some(message),
            })
        }
    }
}
