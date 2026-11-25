pub mod store;

pub(crate) mod date;
pub(crate) mod error;
pub mod model;
mod query;
mod sql;
mod tests;

use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::extract::Extension;
use axum::extract::RawQuery;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse as _;
use axum::response::Response;
use axum::routing::get;
use chrono::{DateTime, Utc};

use crate::auth::auth_identity::AuthContext;
use crate::error_body::json_error;
use crate::tracing_routes::router::unauthenticated;

use self::error::CacheActivityError;
use self::model::CacheActivityResponse;
use self::query::{CacheActivityQuery, decode};
use self::store::CacheActivityStore;

pub const CACHE_ACTIVITY_PATH: &str = "/global/activity/cache_hits";
// LiteLLM declares this route with include_in_schema=False, so no OpenAPI document may list it.
pub const DOCUMENTED_IN_OPENAPI: bool = false;

pub fn routes(store: Option<Arc<dyn CacheActivityStore>>) -> Router {
    Router::new()
        .route(CACHE_ACTIVITY_PATH, get(cache_activity))
        .method_not_allowed_fallback(|| async { error::method_not_allowed() })
        .with_state(store)
}

async fn cache_activity(
    State(store): State<Option<Arc<dyn CacheActivityStore>>>,
    caller: Option<Extension<AuthContext>>,
    RawQuery(raw_query): RawQuery,
) -> Response {
    // The auth layer refuses a missing or invalid key before this route; the guard keeps it safe on its own.
    if caller.is_none() {
        return unauthenticated();
    }
    let requested = match decode(raw_query.as_deref()) {
        Ok(requested) => requested,
        Err(parameters) => return CacheActivityError::MissingDates(parameters).into_response(),
    };
    let Some(database) = store.as_deref() else {
        return CacheActivityError::DatabaseMissing.into_response();
    };
    let (start_date, end_date) = match requested_dates(&requested) {
        Ok(dates) => dates,
        Err(error) => return error.into_response(),
    };
    respond(database, start_date, end_date, &requested).await
}

fn requested_dates(
    requested: &CacheActivityQuery,
) -> Result<(DateTime<Utc>, DateTime<Utc>), CacheActivityError> {
    let Some(start_date) = date::parse_date(&requested.start_date) else {
        return Err(invalid_date("start_date", &requested.start_date));
    };
    let Some(end_date) = date::parse_date(&requested.end_date) else {
        return Err(invalid_date("end_date", &requested.end_date));
    };
    Ok((start_date, end_date))
}

fn invalid_date(parameter: &'static str, value: &str) -> CacheActivityError {
    CacheActivityError::InvalidDate {
        parameter,
        value: value.to_owned(),
    }
}

async fn respond(
    store: &dyn CacheActivityStore,
    start_date: DateTime<Utc>,
    end_date: DateTime<Utc>,
    requested: &CacheActivityQuery,
) -> Response {
    let result = store
        .read(
            start_date,
            end_date,
            &requested.key_aliases,
            &requested.models,
        )
        .await;
    match result {
        Ok(rows) => Json(CacheActivityResponse::new(rows)).into_response(),
        Err(source) => {
            tracing::error!(error = ?source, "cache activity query failed");
            json_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Internal server error",
                "internal_server_error",
                "500",
            )
        }
    }
}
