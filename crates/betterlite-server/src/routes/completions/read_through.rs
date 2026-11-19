use std::sync::Arc;

use betterlite_error::proxy_model_not_found::{ProxyModelNotFoundError, route_endpoint};
use betterlite_openai::chat::deployment::ChatDeployment;

use crate::route_read_through::route_with_read_through;
use crate::routing::simple_shuffle::pick_deployment;
use crate::state::ServerState;

use super::error::FailureAnswer;

pub(super) async fn deployment_or_answer(
    state: &ServerState,
    model: Option<&str>,
) -> Result<ChatDeployment, FailureAnswer> {
    let model_text = model.map(str::to_owned);
    let executor = Arc::clone(&state.executor);
    // the AsyncFn bound cannot prove Send for a captured reference, so the attempt owns its reads
    let attempt = move || {
        let shared_executor = Arc::clone(&executor);
        let shared_model = model_text.clone();
        async move {
            let text = shared_model.as_deref().unwrap_or("None");
            let route = route_endpoint("atext_completion").unwrap_or("atext_completion");
            let deployment = pick_deployment(&shared_executor, text)
                .map_err(|_error| ProxyModelNotFoundError::new(route, text, true))?;
            Ok::<ChatDeployment, ProxyModelNotFoundError>(deployment.clone())
        }
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
