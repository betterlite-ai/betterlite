use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::extract::Extension;
use axum::extract::RawQuery;
use axum::extract::State;
use axum::response::IntoResponse as _;
use axum::response::Response;
use axum::routing::get;
use betterlite_db::gateway_request_tracking::GatewayRequestClock;

use crate::auth::auth_identity::AuthContext;

use self::auth::require_admin;
use self::error::ActivityError;
use self::fold::activity_response;
use self::query::ActivityQuery;

pub mod store;

mod auth;
mod error;
mod fold;
mod query;
mod tests;

#[derive(Clone)]
pub struct GatewayActivityState {
    pub store: Option<Arc<dyn store::GatewayActivityStore>>,
    pub clock: Arc<dyn GatewayRequestClock>,
}

pub fn router(state: GatewayActivityState) -> Router {
    Router::new()
        .route("/gateway/daily/activity", get(daily_activity))
        .with_state(state)
}

async fn daily_activity(
    State(state): State<GatewayActivityState>,
    caller: Option<Extension<AuthContext>>,
    RawQuery(raw_query): RawQuery,
) -> Response {
    let context = caller.as_ref().map(|extension| &extension.0);
    if let Err(error) = require_admin(context) {
        return error.into_response();
    }
    let Some(store) = state.store.as_deref() else {
        return ActivityError::DbNotConnected.into_response();
    };
    let requested = ActivityQuery::decode(raw_query.as_deref().unwrap_or_default());
    let (start_date, end_date) = requested.range(state.clock.as_ref());
    match store.aggregate(&start_date, &end_date).await {
        Ok(rows) => {
            tracing::debug!("/gateway/daily/activity - aggregated {} rows", rows.len());
            Json(activity_response(&rows)).into_response()
        }
        Err(error) => ActivityError::Store(error).into_response(),
    }
}
