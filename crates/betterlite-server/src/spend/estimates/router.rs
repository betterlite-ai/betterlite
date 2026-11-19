use std::sync::Arc;
use std::time::SystemTime;

use axum::Json;
use axum::Router;
use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, State};
use axum::routing::{get, post};
use betterlite_core::cost::prepare::BillingTokenCounter;
use betterlite_provider::pricing::grounding::ProviderFormulas;
use rust_decimal::Decimal;
use serde::Serialize;

use super::budgets::{ProviderBudgetResponse, provider_budgets};
use super::decode::decode_spend_estimate;
use super::error::EstimateError;
use super::estimate::estimate;
use super::ports::{Clock, EstimateRouterPort};
use crate::cost_service::CostService;

// Every inbound body is bounded; /spend/calculate carries a whole chat request.
pub(crate) const MAX_REQUEST_BYTES: usize = 2 * 1024 * 1024;

// The two endpoints' dependencies, so a test can drive them with fake ports.
pub(crate) struct EstimatesState {
    pub(crate) cost: Arc<CostService<ProviderFormulas>>,
    pub(crate) counter: Arc<dyn BillingTokenCounter>,
    pub(crate) router: Option<Arc<dyn EstimateRouterPort>>,
    pub(crate) clock: Arc<dyn Clock>,
}

// calculate_spend's answer; LiteLLM returns completion_cost's float, kept as a Decimal here.
#[derive(Debug, Serialize)]
pub(crate) struct SpendCalculateResponse {
    #[serde(serialize_with = "rust_decimal::serde::arbitrary_precision::serialize")]
    pub(crate) cost: Decimal,
}

pub(crate) fn routes(state: Arc<EstimatesState>) -> Router {
    Router::new()
        .route("/spend/calculate", post(calculate))
        .route("/provider/budgets", get(read_budgets))
        .layer(DefaultBodyLimit::max(MAX_REQUEST_BYTES))
        .with_state(state)
}

async fn calculate(
    State(state): State<Arc<EstimatesState>>,
    body: Bytes,
) -> Result<Json<SpendCalculateResponse>, EstimateError> {
    let request = decode_spend_estimate(&body)?;
    let instant = SystemTime::from(state.clock.now());
    let cost = estimate(
        &state.cost,
        state.counter.as_ref(),
        state.router.as_deref(),
        &request,
        instant,
    )
    .await?;
    Ok(Json(SpendCalculateResponse { cost }))
}

async fn read_budgets(
    State(state): State<Arc<EstimatesState>>,
) -> Result<Json<ProviderBudgetResponse>, EstimateError> {
    let now = state.clock.now();
    let providers = provider_budgets(state.router.as_deref(), now).await?;
    Ok(Json(ProviderBudgetResponse { providers }))
}
