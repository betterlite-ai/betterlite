use axum::Json;
use axum::extract::{Extension, RawQuery, State};
use axum::response::{IntoResponse as _, Response};
use betterlite_db::health_check_rows::HealthCheckRow;
use betterlite_models::decisions::Json as Decisions;
use betterlite_models::model_registration::JsonObject;
use serde_json::Number;

use crate::auth::auth_identity::AuthContext;
use crate::health_status_routes::convert::convert_health_check;
use crate::health_status_routes::error::HealthRouteError;
use crate::health_status_routes::prisma::check_prisma_client;
use crate::health_status_routes::query::{HistoryQuery, decode_history_query};
use crate::health_status_routes::state::HealthStatusState;

pub(super) async fn health_check_history_endpoint(
    State(state): State<HealthStatusState>,
    identity: Option<Extension<AuthContext>>,
    RawQuery(raw): RawQuery,
) -> Response {
    let Some(Extension(_context)) = identity else {
        return HealthRouteError::Unauthenticated.into_response();
    };
    let query = match decode_history_query(raw.as_deref()) {
        Ok(query) => query,
        Err(failure) => return failure.into_response(),
    };
    match history_page(&state, &query).await {
        Ok(page) => Json(page).into_response(),
        Err(error) => error.into_response(),
    }
}

async fn history_page(
    state: &HealthStatusState,
    query: &HistoryQuery,
) -> Result<JsonObject, HealthRouteError> {
    let rows = check_prisma_client(state.rows.as_deref())?;
    let records = rows
        .history(
            query.model.as_deref(),
            query.status_filter.as_deref(),
            query.limit,
            query.offset,
        )
        .await;
    Ok(history_body(&records, query))
}

fn history_body(records: &[HealthCheckRow], query: &HistoryQuery) -> JsonObject {
    let mut body = JsonObject::new();
    body.insert(
        "health_checks".to_owned(),
        Decisions::List(
            records
                .iter()
                .map(|row| Decisions::Object(convert_health_check(row)))
                .collect(),
        ),
    );
    body.insert(
        "total_records".to_owned(),
        Decisions::Number(Number::from(records.len())),
    );
    body.insert(
        "limit".to_owned(),
        Decisions::Number(Number::from(query.limit)),
    );
    body.insert(
        "offset".to_owned(),
        Decisions::Number(Number::from(query.offset)),
    );
    body
}
