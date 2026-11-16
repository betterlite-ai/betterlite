use axum::Json;
use axum::extract::{Extension, RawQuery, State};
use axum::response::{IntoResponse as _, Response};
use betterlite_db::postgres_store::errors::DbError;
use rust_decimal::Decimal;
use serde::Serialize;

use crate::auth::auth_identity::AuthContext;

use super::access::{Caller, classify};
use super::internal::{
    InternalFailure, global_spend_for_internal_user, global_spend_key_internal_user,
    global_spend_models_internal,
};
use super::query::{LimitQuery, decode_spend_totals_query};
use super::responses;
use super::rows::TeamSpend;
use super::state::TotalsState;
use super::store::{TotalsStore, TotalsStoreError};
use super::teams::team_totals;

const DEFAULT_MODELS_LIMIT: i64 = 10;
const NO_DATABASE: &str = "No db connected";
const NO_USER_ID: &str = "No user_id found";
const PRISMA_MISSING: &str = "Prisma Client is not initialized";
const LOGS_FAILED: &str = "/global/spend/logs Error";
const SPEND_FAILED: &str = "/global/spend Error";
const MISSING_USER: &str = "/global/spend/logs Error: User ID is None";

#[derive(Serialize)]
struct GlobalSpendBody {
    #[serde(serialize_with = "rust_decimal::serde::arbitrary_precision_option::serialize")]
    spend: Option<Decimal>,
    #[serde(serialize_with = "rust_decimal::serde::arbitrary_precision::serialize")]
    max_budget: Decimal,
}

pub(super) async fn global_spend_logs(
    State(state): State<TotalsState>,
    caller: Option<Extension<AuthContext>>,
    RawQuery(raw_query): RawQuery,
) -> Response {
    let Some(Extension(context)) = caller else {
        return responses::unauthenticated();
    };
    let query = decode_spend_totals_query(raw_query.as_deref().unwrap_or_default().as_bytes());
    let Some(store) = state.store.as_deref() else {
        return responses::wrapped_error(PRISMA_MISSING.to_owned());
    };
    match classify(&context.principal) {
        Caller::Internal { user_id } => {
            internal_logs(store, user_id.as_deref(), query.api_key.as_deref()).await
        }
        Caller::Global => global_logs(&state, store, query.api_key.as_deref()).await,
    }
}

async fn internal_logs(
    store: &dyn TotalsStore,
    user_id: Option<&str>,
    api_key: Option<&str>,
) -> Response {
    match global_spend_for_internal_user(Some(store), user_id, api_key).await {
        Ok(rows) => Json(rows).into_response(),
        Err(failure) => logs_internal_failure(&failure),
    }
}

async fn global_logs(
    state: &TotalsState,
    store: &dyn TotalsStore,
    api_key: Option<&str>,
) -> Response {
    if let Some(source) = state.prometheus.as_deref() {
        return match source.daily_spend(api_key).await {
            Ok(rows) => Json(rows).into_response(),
            Err(_) => responses::wrapped_error(LOGS_FAILED.to_owned()),
        };
    }
    let rows = match api_key {
        Some(api_key) => store.key_daily_spend(api_key).await,
        None => store.global_daily_spend().await,
    };
    match rows {
        Ok(rows) => Json(rows).into_response(),
        Err(error) => wrapped_failure(LOGS_FAILED, &error),
    }
}

pub(super) async fn global_spend(
    State(state): State<TotalsState>,
    caller: Option<Extension<AuthContext>>,
) -> Response {
    if caller.is_none() {
        return responses::unauthenticated();
    }
    let Some(store) = state.store.as_deref() else {
        return responses::wrapped_error(NO_DATABASE.to_owned());
    };
    match store.global_total_spend().await {
        Ok(rows) => {
            let spend = match rows.first() {
                Some(row) => row.total_spend,
                None => Some(Decimal::ZERO),
            };
            Json(GlobalSpendBody {
                spend,
                max_budget: state.max_budget,
            })
            .into_response()
        }
        Err(error) => wrapped_failure(SPEND_FAILED, &error),
    }
}

pub(super) async fn global_spend_keys(
    State(state): State<TotalsState>,
    caller: Option<Extension<AuthContext>>,
    RawQuery(raw_query): RawQuery,
) -> Response {
    let Some(Extension(context)) = caller else {
        return responses::unauthenticated();
    };
    let query = decode_spend_totals_query(raw_query.as_deref().unwrap_or_default().as_bytes());
    if query.limit == LimitQuery::Invalid {
        return responses::invalid_limit_type();
    }
    match classify(&context.principal) {
        Caller::Internal { user_id } => {
            match global_spend_key_internal_user(state.store.as_deref(), user_id.as_deref()).await {
                Ok(rows) => Json(rows).into_response(),
                Err(failure) => detail_internal_failure(&failure),
            }
        }
        Caller::Global => global_keys(state.store.as_deref(), query.limit).await,
    }
}

async fn global_keys(store: Option<&dyn TotalsStore>, limit: LimitQuery) -> Response {
    let Some(store) = store else {
        return responses::detail_error(NO_DATABASE);
    };
    let limit = match limit {
        LimitQuery::Absent => None,
        LimitQuery::Value(limit) if limit < 1 => return responses::invalid_limit(limit),
        LimitQuery::Value(limit) => Some(limit),
        LimitQuery::Invalid => return responses::invalid_limit_type(),
    };
    match store.keys_by_spend(limit).await {
        Ok(rows) => Json(rows).into_response(),
        Err(error) => detail_store_failure(&error),
    }
}

pub(super) async fn global_spend_models(
    State(state): State<TotalsState>,
    caller: Option<Extension<AuthContext>>,
    RawQuery(raw_query): RawQuery,
) -> Response {
    let Some(Extension(context)) = caller else {
        return responses::unauthenticated();
    };
    let query = decode_spend_totals_query(raw_query.as_deref().unwrap_or_default().as_bytes());
    let limit = match query.limit {
        LimitQuery::Absent => DEFAULT_MODELS_LIMIT,
        LimitQuery::Value(limit) => limit,
        LimitQuery::Invalid => return responses::invalid_limit_type(),
    };
    match classify(&context.principal) {
        Caller::Internal { user_id } => {
            match global_spend_models_internal(state.store.as_deref(), user_id.as_deref(), limit)
                .await
            {
                Ok(rows) => Json(rows).into_response(),
                Err(failure) => detail_internal_failure(&failure),
            }
        }
        Caller::Global => global_models(state.store.as_deref(), limit).await,
    }
}

async fn global_models(store: Option<&dyn TotalsStore>, limit: i64) -> Response {
    let Some(store) = store else {
        return responses::detail_error(NO_DATABASE);
    };
    match store.models_by_spend(limit).await {
        Ok(rows) => Json(rows).into_response(),
        Err(error) => detail_store_failure(&error),
    }
}

pub(super) async fn global_spend_teams(
    State(state): State<TotalsState>,
    caller: Option<Extension<AuthContext>>,
) -> Response {
    if caller.is_none() {
        return responses::unauthenticated();
    }
    let Some(store) = state.store.as_deref() else {
        return responses::detail_error(NO_DATABASE);
    };
    match store.team_daily_spend().await {
        Ok(rows) => team_body(&rows),
        Err(error) => detail_store_failure(&error),
    }
}

fn team_body(rows: &[TeamSpend]) -> Response {
    match team_totals(rows) {
        Ok(body) => Json(body).into_response(),
        Err(error) => {
            tracing::warn!(error = %error, "the team spend totals could not be built");
            responses::internal_error()
        }
    }
}

fn wrapped_failure(prefix: &str, error: &TotalsStoreError) -> Response {
    tracing::warn!(error = %error, "the spend totals query failed");
    responses::wrapped_error(format!("{prefix}: {error}"))
}

fn detail_internal_failure(failure: &InternalFailure) -> Response {
    match failure {
        InternalFailure::NoDatabase => responses::detail_error(NO_DATABASE),
        InternalFailure::MissingUserId => responses::detail_error(NO_USER_ID),
        InternalFailure::Store(error) => detail_store_failure(error),
    }
}

fn detail_store_failure(error: &TotalsStoreError) -> Response {
    match error {
        TotalsStoreError::Database(DbError::NoDbConnected) => responses::detail_error(NO_DATABASE),
        TotalsStoreError::InvalidAmount { .. } | TotalsStoreError::Database(_) => {
            tracing::warn!(error = %error, "the spend totals query failed");
            responses::internal_error()
        }
    }
}

fn logs_internal_failure(failure: &InternalFailure) -> Response {
    match failure {
        InternalFailure::NoDatabase => responses::wrapped_error(PRISMA_MISSING.to_owned()),
        InternalFailure::MissingUserId => responses::wrapped_error(MISSING_USER.to_owned()),
        InternalFailure::Store(error) => wrapped_failure(LOGS_FAILED, error),
    }
}
