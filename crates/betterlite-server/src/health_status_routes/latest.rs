use axum::Json;
use axum::extract::{Extension, State};
use axum::response::{IntoResponse as _, Response};
use betterlite_db::health_check_rows::HealthCheckRow;
use betterlite_models::decisions::Json as Decisions;
use betterlite_models::model_registration::JsonObject;
use serde::Serialize;

use crate::auth::auth_identity::AuthContext;
use crate::health_status_routes::convert::convert_health_check;
use crate::health_status_routes::error::HealthRouteError;
use crate::health_status_routes::prisma::check_prisma_client;
use crate::health_status_routes::state::HealthStatusState;

// GET /health/latest's object, in LiteLLM's key order
#[derive(Serialize)]
struct LatestPage {
    latest_health_checks: JsonObject,
    total_models: usize,
}

pub(super) async fn latest_health_checks_endpoint(
    State(state): State<HealthStatusState>,
    identity: Option<Extension<AuthContext>>,
) -> Response {
    let Some(Extension(_context)) = identity else {
        return HealthRouteError::Unauthenticated.into_response();
    };
    match latest_page(&state).await {
        Ok(body) => Json(body).into_response(),
        Err(error) => error.into_response(),
    }
}

async fn latest_page(state: &HealthStatusState) -> Result<LatestPage, HealthRouteError> {
    let rows = check_prisma_client(state.rows.as_deref())?;
    // fetch_latest_health_checks logs a driver error and reads as no rows, as LiteLLM's wrapper does
    let records = rows.fetch_latest().await;
    Ok(latest_body(&records))
}

fn latest_body(records: &[HealthCheckRow]) -> LatestPage {
    let mut checks = JsonObject::new();
    for row in records {
        // a later row with the same key replaces the value where the earlier one sits
        checks.insert(
            lookup_key(row),
            Decisions::Object(convert_health_check(row)),
        );
    }
    let total_models = checks.len();
    LatestPage {
        latest_health_checks: checks,
        total_models,
    }
}

fn lookup_key(row: &HealthCheckRow) -> String {
    row.model_id
        .as_deref()
        .filter(|id| !id.is_empty())
        .map_or_else(|| row.model_name.clone(), str::to_owned)
}
