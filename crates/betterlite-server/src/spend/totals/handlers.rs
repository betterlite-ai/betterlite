use axum::Json;
use axum::extract::{Extension, RawQuery, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse as _, Response};
use betterlite_db::postgres_store::errors::DbError;
use rust_decimal::Decimal;
use serde::Serialize;

use crate::auth::auth_identity::AuthContext;
use crate::tracing_routes::router::unauthenticated;

use super::access::{Caller, classify};
use super::prometheus::DailySpendSource;
use super::query::{LimitQuery, decode_spend_totals_query};
use super::responses;
use super::rows::TeamSpend;
use super::state::TotalsState;
use super::store::{TotalsStore, TotalsStoreError};
use super::teams::team_totals;

const DEFAULT_MODELS_LIMIT: i64 = 10;
// LiteLLM's key helper declares limit: int = 10 and the keys route never passes its own
const INTERNAL_KEYS_LIMIT: i64 = 10;
const NO_DATABASE: &str = "No db connected";
const NO_USER_ID: &str = "No user_id found";
const PRISMA_MISSING: &str = "Prisma Client is not initialized";
const LOGS_FAILED: &str = "/global/spend/logs Error";
const SPEND_FAILED: &str = "/global/spend Error";
const MISSING_USER: &str = "/global/spend/logs Error: User ID is None";

#[derive(Clone, Copy)]
pub(super) enum Report {
    Logs,
    Spend,
    Keys,
    Models,
    Teams,
}

#[derive(Serialize)]
struct GlobalSpendBody {
    #[serde(serialize_with = "rust_decimal::serde::arbitrary_precision_option::serialize")]
    spend: Option<Decimal>,
    #[serde(serialize_with = "rust_decimal::serde::arbitrary_precision::serialize")]
    max_budget: Decimal,
}

pub(super) async fn respond<S>(
    State(state): State<TotalsState<S>>,
    Extension(report): Extension<Report>,
    identity: Option<Extension<AuthContext>>,
    RawQuery(raw_query): RawQuery,
) -> Response
where
    S: DailySpendSource + 'static,
{
    let Some(Extension(context)) = identity else {
        return unauthenticated();
    };
    let query = decode_spend_totals_query(raw_query.as_deref().unwrap_or_default().as_bytes());
    let ranked = matches!(report, Report::Keys | Report::Models);
    if ranked && query.limit == LimitQuery::Invalid {
        return responses::invalid_limit_type();
    }
    let Some(store) = state.store.as_deref() else {
        return missing_store(report);
    };
    let caller = classify(&context.principal);
    match report {
        Report::Logs => logs(&state, store, caller, query.api_key.as_deref()).await,
        Report::Spend => spend(store, state.max_budget).await,
        Report::Keys => keys(store, caller, query.limit).await,
        Report::Models => models(store, caller, query.limit).await,
        Report::Teams => teams(store).await,
    }
}

fn missing_store(report: Report) -> Response {
    match report {
        Report::Logs => responses::wrapped_error(PRISMA_MISSING.to_owned()),
        Report::Spend => responses::wrapped_error(NO_DATABASE.to_owned()),
        Report::Keys | Report::Models | Report::Teams => {
            responses::detail_error(StatusCode::INTERNAL_SERVER_ERROR, NO_DATABASE)
        }
    }
}

async fn logs<S>(
    state: &TotalsState<S>,
    store: &dyn TotalsStore,
    caller: Caller,
    api_key: Option<&str>,
) -> Response
where
    S: DailySpendSource,
{
    match caller {
        Caller::Internal { user_id: None } => {
            return responses::wrapped_error(MISSING_USER.to_owned());
        }
        Caller::Internal {
            user_id: Some(user_id),
        } => {
            let rows = store.user_daily_spend(&user_id, api_key).await;
            return rows_response(rows, Some(LOGS_FAILED));
        }
        Caller::Global => {}
    }
    if let Some(source) = state.prometheus.as_deref() {
        return match source.daily_spend(api_key).await {
            Ok(rows) => Json(rows).into_response(),
            Err(failure) => {
                tracing::warn!(error = ?failure, "the Prometheus daily spend query failed");
                responses::wrapped_error(LOGS_FAILED.to_owned())
            }
        };
    }
    match api_key {
        Some(key) => rows_response(store.key_daily_spend(key).await, Some(LOGS_FAILED)),
        None => rows_response(store.global_daily_spend().await, Some(LOGS_FAILED)),
    }
}

async fn spend(store: &dyn TotalsStore, max_budget: Decimal) -> Response {
    let rows = match store.global_total_spend().await {
        Ok(rows) => rows,
        Err(error) => return wrapped_failure(SPEND_FAILED, &error),
    };
    let total = rows
        .first()
        .map_or(Some(Decimal::ZERO), |row| row.total_spend);
    Json(GlobalSpendBody {
        spend: total,
        max_budget,
    })
    .into_response()
}

fn invalid_limit(limit: i64) -> Response {
    responses::detail_error(
        StatusCode::UNPROCESSABLE_ENTITY,
        &format!("Invalid limit: {limit}, error: Limit must be greater than 0"),
    )
}

async fn keys(store: &dyn TotalsStore, caller: Caller, requested: LimitQuery) -> Response {
    match (caller, requested) {
        (Caller::Internal { user_id: None }, _) => {
            responses::detail_error(StatusCode::INTERNAL_SERVER_ERROR, NO_USER_ID)
        }
        (
            Caller::Internal {
                user_id: Some(user_id),
            },
            _,
        ) => rows_response(
            store
                .user_keys_by_spend(&user_id, INTERNAL_KEYS_LIMIT)
                .await,
            None,
        ),
        (Caller::Global, LimitQuery::Value(limit)) if limit < 1 => invalid_limit(limit),
        (Caller::Global, LimitQuery::Value(limit)) => {
            rows_response(store.keys_by_spend(Some(limit)).await, None)
        }
        (Caller::Global, LimitQuery::Absent) => {
            rows_response(store.keys_by_spend(None).await, None)
        }
        (Caller::Global, LimitQuery::Invalid) => responses::invalid_limit_type(),
    }
}

async fn models(store: &dyn TotalsStore, caller: Caller, requested: LimitQuery) -> Response {
    let limit = match requested {
        LimitQuery::Absent => DEFAULT_MODELS_LIMIT,
        LimitQuery::Value(value) => value,
        LimitQuery::Invalid => return responses::invalid_limit_type(),
    };
    match caller {
        Caller::Internal { user_id: None } => {
            responses::detail_error(StatusCode::INTERNAL_SERVER_ERROR, NO_USER_ID)
        }
        Caller::Internal {
            user_id: Some(user_id),
        } => {
            if limit < 0 {
                invalid_limit(limit)
            } else {
                rows_response(store.user_models_by_spend(&user_id, limit).await, None)
            }
        }
        Caller::Global => {
            if limit < 0 {
                invalid_limit(limit)
            } else {
                rows_response(store.models_by_spend(limit).await, None)
            }
        }
    }
}

async fn teams(store: &dyn TotalsStore) -> Response {
    match store.team_daily_spend().await {
        Ok(rows) => team_body(&rows),
        Err(error) => store_failure(&error),
    }
}

fn team_body(rows: &[TeamSpend]) -> Response {
    match team_totals(rows) {
        Ok(body) => Json(body).into_response(),
        Err(error) => {
            tracing::warn!(error = %error, "the team spend totals could not be built");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(responses::INTERNAL_ERROR),
            )
                .into_response()
        }
    }
}

fn rows_response<Item>(
    result: Result<Vec<Item>, TotalsStoreError>,
    wrapped_prefix: Option<&str>,
) -> Response
where
    Item: Serialize,
{
    match (result, wrapped_prefix) {
        (Ok(rows), _) => Json(rows).into_response(),
        (Err(error), Some(prefix)) => wrapped_failure(prefix, &error),
        (Err(error), None) => store_failure(&error),
    }
}

fn wrapped_failure(prefix: &str, error: &TotalsStoreError) -> Response {
    tracing::warn!(error = %error, "the spend totals query failed");
    responses::wrapped_error(format!("{prefix}: {error}"))
}

fn store_failure(error: &TotalsStoreError) -> Response {
    match error {
        TotalsStoreError::Database(DbError::NoDbConnected) => {
            responses::detail_error(StatusCode::INTERNAL_SERVER_ERROR, NO_DATABASE)
        }
        TotalsStoreError::Database(_)
        | TotalsStoreError::InvalidAmount { .. }
        | TotalsStoreError::UnreadableAmount { .. } => {
            tracing::warn!(error = %error, "the spend totals query failed");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(responses::INTERNAL_ERROR),
            )
                .into_response()
        }
    }
}
