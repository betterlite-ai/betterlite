use std::sync::Arc;
use std::time::SystemTime;

use axum::Json;
use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::routing::post;

use crate::state::ServerState;

use super::decode::decode_cost_estimate_request;
use super::deployments::DeploymentSet;
use super::error::EstimateError;
use super::estimate::estimate;
use crate::proxy_request_contracts::cost_estimate::CostEstimateResponse;

#[derive(Clone)]
struct RouteState {
    server: Arc<ServerState>,
}

pub(crate) fn routes(state: &Arc<ServerState>) -> Router {
    Router::new()
        .route("/cost/estimate", post(quote))
        .with_state(RouteState {
            server: Arc::clone(state),
        })
}

async fn quote(
    State(state): State<RouteState>,
    body: Bytes,
) -> Result<Json<CostEstimateResponse>, EstimateError> {
    let request = decode_cost_estimate_request(&body)?;
    let service = state
        .server
        .cost_service
        .service
        .as_ref()
        .map_err(|error| EstimateError::Calculation {
            message: format!(
                "Could not calculate cost for model '{}' (resolved to '{}'): the cost service is unavailable: {error}",
                request.model, request.model
            ),
        })?;
    let deployments = DeploymentSet::from_config(&state.server.config)
        .inspect_err(|error| {
            tracing::warn!(error = %error, "the cost estimate could not index the configured deployments");
        })
        .ok();
    let response = estimate(service, deployments.as_ref(), &request, SystemTime::now()).await?;
    Ok(Json(response))
}
