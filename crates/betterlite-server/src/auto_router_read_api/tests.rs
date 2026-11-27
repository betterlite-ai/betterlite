#![cfg(test)]

mod behaviors;
mod corpus;
mod postgres;

use std::collections::BTreeMap;
use std::convert::Infallible;
use std::error::Error as _;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_auth::auth_identity::records::{
    CredentialRef, NetworkContext, Principal, SourceKey,
};
use betterlite_auth::auth_identity::roles::{AuthMethod, PrincipalType};
use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_config::types::{LiteLlmParams, ModelDeployment, ModelInfo};
use betterlite_db::autorouter_facts::session::bounded_session_id;
use betterlite_db::auxiliary_repository::repository::TableError;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::managed_resource_rows::autorouter::AutoRouterSession;
use betterlite_db::managed_resource_rows::wire::RowTimestamp;
use betterlite_db::postgres_store::errors::DbError;
use betterlite_logging::analytics::python::FormatError;
use chrono::{NaiveDate, NaiveDateTime};
use rstest::rstest;
use rust_decimal::Decimal;
use sha2::{Digest as _, Sha256};
use tokio::sync::Notify;
use tokio::time::timeout;

use super::VIEW_ACTION;
use super::access::caller_api_key;
use super::error::ReadApiError;
use super::query::{
    BenchmarksQuery, MAX_READ_QUERY_BYTES, ReadQueryError, SessionQuery, decode_benchmarks_query,
    decode_session_query,
};
use super::service::{benchmarks_for, session_for};
use super::store::AutoRouterReadStore;
use crate::benchmark_aggregates::decode::DecodeError;
use crate::benchmark_aggregates::row::SessionAggRow;
use crate::benchmark_aggregates::window::{BenchmarkWindow, WindowError};
use crate::plugin_routes::routes::Identity;
use crate::routing::auto_router_read_api::configured_deployments;
use crate::shadow_eval_lifecycle::access::require_admin;
use crate::shadow_eval_validation::access::AdminRole;

const HASH_A: &str = "hash-a";

fn money(text: &str) -> Decimal {
    text.parse().unwrap()
}

fn day(month: u32, day_of_month: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2024, month, day_of_month).unwrap()
}

fn timestamp(text: &str) -> RowTimestamp {
    serde_json::from_str(&format!("\"{text}\"")).unwrap()
}

fn row(router_name: &str, router_type: &str) -> SessionAggRow {
    SessionAggRow {
        router_name: router_name.to_owned(),
        router_type: router_type.to_owned(),
        tier_turns: BTreeMap::new(),
        sessions: 0,
        turns: 0,
        unordered_turns: 0,
        covered_turns: 0,
        cache_hits: 0,
        same_model_turns: 0,
        same_model_hits: 0,
        first_visit_turns: 0,
        first_visit_hits: 0,
        return_turns: 0,
        return_hits: 0,
        return_expired_misses: 0,
        return_within_ttl_misses: 0,
        ttl_5m_turns: 0,
        ttl_1h_turns: 0,
        total_tokens: 0,
        spend: Decimal::ZERO,
        saved_spend: Decimal::ZERO,
        savings_estimated_turns: 0,
        savings_estimated_actual_spend: Decimal::ZERO,
        savings_classifier_cost: None,
        savings_estimated_saved_spend: Decimal::ZERO,
        classifier_cost: Decimal::ZERO,
        classifier_cost_recorded_turns: 0,
        session_seconds: Decimal::ZERO,
    }
}

fn query(
    start_date: Option<&str>,
    end_date: Option<&str>,
    api_key: Option<&str>,
    user_id: Option<&str>,
) -> BenchmarksQuery {
    BenchmarksQuery {
        start_date: start_date.map(str::to_owned),
        end_date: end_date.map(str::to_owned),
        key_hash: api_key.map(str::to_owned),
        user_id: user_id.map(str::to_owned),
    }
}

fn session(api_key: &str, session_id: &str, last_turn_at: &str) -> AutoRouterSession {
    AutoRouterSession {
        hashed_api_key: api_key.to_owned(),
        session_id: session_id.to_owned(),
        router_name: "complexity-auto".to_owned(),
        router_type: "complexity".to_owned(),
        first_turn_at: timestamp("2024-01-01T00:00:00"),
        last_turn_at: timestamp(last_turn_at),
        last_model: "gpt-4o".to_owned(),
        turns: 4,
        spend: money("1.5"),
        saved_spend: money("0.5"),
        estimated_turns: 4,
        estimated_actual_spend: money("1.0"),
        estimated_saved_spend: money("0.5"),
        estimated_baseline_models: BTreeMap::new(),
        classifier_cost: money("0.25"),
        tier_turns: BTreeMap::new(),
        baseline_models: BTreeMap::from([("m1".to_owned(), 3), ("m2".to_owned(), 1)]),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum RecordedCall {
    AggRows {
        start: String,
        end: String,
        api_key: Option<String>,
        user_id: Option<String>,
    },
    LatestSession {
        api_key: String,
        session_id: String,
    },
}

struct FakeStore {
    rows: Vec<SessionAggRow>,
    sessions: Vec<AutoRouterSession>,
    calls: Mutex<Vec<RecordedCall>>,
    gate: Option<Arc<Notify>>,
}

impl FakeStore {
    fn empty() -> Self {
        Self {
            rows: Vec::new(),
            sessions: Vec::new(),
            calls: Mutex::new(Vec::new()),
            gate: None,
        }
    }

    fn with_rows(rows: Vec<SessionAggRow>) -> Self {
        Self {
            rows,
            ..Self::empty()
        }
    }

    fn with_sessions(sessions: Vec<AutoRouterSession>) -> Self {
        Self {
            sessions,
            ..Self::empty()
        }
    }

    fn gated() -> Self {
        Self {
            gate: Some(Arc::new(Notify::new())),
            ..Self::empty()
        }
    }

    fn release(&self) {
        if let Some(gate) = &self.gate {
            gate.notify_one();
        }
    }

    fn calls(&self) -> Vec<RecordedCall> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn record(&self, call: RecordedCall) {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(call);
    }
}

#[async_trait]
impl AutoRouterReadStore for FakeStore {
    async fn agg_rows(
        &self,
        window: &BenchmarkWindow,
        api_key: Option<&str>,
        user_id: Option<&str>,
    ) -> Result<Vec<SessionAggRow>, ReadApiError> {
        if let Some(gate) = &self.gate {
            gate.notified().await;
        }
        self.record(RecordedCall::AggRows {
            start: window.query_start.clone(),
            end: window.query_end.clone(),
            api_key: api_key.map(str::to_owned),
            user_id: user_id.map(str::to_owned),
        });
        Ok(self.rows.clone())
    }

    async fn latest_session(
        &self,
        api_key: &str,
        session_id: &str,
    ) -> Result<Option<AutoRouterSession>, ReadApiError> {
        self.record(RecordedCall::LatestSession {
            api_key: api_key.to_owned(),
            session_id: session_id.to_owned(),
        });
        Ok(self
            .sessions
            .iter()
            .find(|row| row.hashed_api_key == api_key && row.session_id == session_id)
            .cloned())
    }
}

#[derive(Clone, Debug)]
enum Boundary {
    InvalidDay(&'static str),
    ReversedRange,
    EmptyUserId,
    MissingSessionId,
    RepeatedAndUnknownParameters,
    QueryAtTheLimit,
    QueryPastTheLimit,
    WindowBounds,
    DefaultWindow,
    Filters,
    MemberForbidden,
    SessionNotFound(&'static str, &'static str),
    BoundedSession,
    NoCallerKey,
    Cancellation,
    BaselineModelTie,
    BaselineModelCoverage,
}

#[rstest]
#[case::an_unparsable_day_is_400(Boundary::InvalidDay("2024-13-01"))]
#[case::a_reversed_range_is_400(Boundary::ReversedRange)]
#[case::an_empty_user_filter_is_422(Boundary::EmptyUserId)]
#[case::a_missing_session_id_is_422(Boundary::MissingSessionId)]
#[case::repeated_parameters_keep_the_last(Boundary::RepeatedAndUnknownParameters)]
#[case::a_query_at_the_limit_decodes(Boundary::QueryAtTheLimit)]
#[case::a_query_past_the_limit_is_refused(Boundary::QueryPastTheLimit)]
#[case::the_window_end_is_the_next_midnight(Boundary::WindowBounds)]
#[case::absent_dates_default_to_today_and_thirty_days_back(Boundary::DefaultWindow)]
#[case::the_filters_reach_the_store_exactly(Boundary::Filters)]
#[case::a_member_is_forbidden(Boundary::MemberForbidden)]
#[case::session_404_quotes_the_raw_id(Boundary::SessionNotFound("s1", "'s1'"))]
#[case::session_404_switches_quotes(Boundary::SessionNotFound("it's", "\"it's\""))]
#[case::an_oversized_session_id_is_bounded(Boundary::BoundedSession)]
#[case::a_caller_without_a_key_reads_nothing(Boundary::NoCallerKey)]
#[case::a_cancelled_read_leaves_the_store_usable(Boundary::Cancellation)]
#[case::a_baseline_tie_picks_the_greater_name(Boundary::BaselineModelTie)]
#[case::a_baseline_below_full_coverage_is_unnamed(Boundary::BaselineModelCoverage)]
#[tokio::test]
async fn auto_router_read_api_boundaries(#[case] boundary: Boundary) {
    match boundary {
        Boundary::InvalidDay(value) => invalid_day(value).await,
        Boundary::ReversedRange => reversed_range().await,
        Boundary::EmptyUserId => empty_user_id(),
        Boundary::MissingSessionId => missing_session_id(),
        Boundary::RepeatedAndUnknownParameters => repeated_and_unknown_parameters(),
        Boundary::QueryAtTheLimit => query_at_the_limit(),
        Boundary::QueryPastTheLimit => query_past_the_limit(),
        Boundary::WindowBounds => window_bounds().await,
        Boundary::DefaultWindow => default_window().await,
        Boundary::Filters => filters().await,
        Boundary::MemberForbidden => member_forbidden(),
        Boundary::SessionNotFound(session_id, expected) => {
            session_not_found(session_id, expected).await;
        }
        Boundary::BoundedSession => bounded_session().await,
        Boundary::NoCallerKey => no_caller_key().await,
        Boundary::Cancellation => cancellation().await,
        Boundary::BaselineModelTie => baseline_model_tie().await,
        Boundary::BaselineModelCoverage => baseline_model_coverage().await,
    }
}

async fn invalid_day(value: &str) {
    let store = FakeStore::empty();
    let error = benchmarks_for(
        &store,
        &query(Some(value), None, None, None),
        &[],
        day(1, 5),
    )
    .await
    .unwrap_err();
    assert_eq!(error.status_code(), 400);
    assert_eq!(
        error.to_string(),
        format!("Invalid date format: {value}. Expected: 'YYYY-MM-DD'")
    );
    let invalid_date = matches!(
        &error,
        ReadApiError::Window(WindowError::InvalidDate { value: reported })
            if reported.as_str() == value
    );
    assert!(
        invalid_date,
        "expected the invalid-date error, got {error:?}"
    );
    assert!(store.calls().is_empty());
}

async fn reversed_range() {
    let store = FakeStore::empty();
    let error = benchmarks_for(
        &store,
        &query(Some("2024-03-01"), Some("2024-02-29"), None, None),
        &[],
        day(3, 1),
    )
    .await
    .unwrap_err();
    assert_eq!(error.status_code(), 400);
    assert_eq!(
        error.to_string(),
        "end_date must not be earlier than start_date"
    );
    assert!(store.calls().is_empty());
}

fn empty_user_id() {
    let error = decode_benchmarks_query(b"user_id=").unwrap_err();
    assert_eq!(
        error,
        ReadQueryError::Invalid {
            parameter: "user_id",
            kind: "string_too_short",
            msg: "String should have at least 1 character",
        }
    );
    assert_eq!(ReadApiError::from(error).status_code(), 422);
}

fn missing_session_id() {
    let error = decode_session_query(b"").unwrap_err();
    assert_eq!(
        error,
        ReadQueryError::Invalid {
            parameter: "session_id",
            kind: "missing",
            msg: "Field required",
        }
    );
    assert_eq!(
        error.to_string(),
        "the query parameter session_id is invalid"
    );
    assert_eq!(ReadApiError::from(error).status_code(), 422);
}

fn repeated_and_unknown_parameters() {
    let decoded =
        decode_benchmarks_query(b"start_date=2024-01-01&extra=1&start_date=2024-02-02&api_key=")
            .unwrap();
    assert_eq!(decoded.start_date.as_deref(), Some("2024-02-02"));
    assert_eq!(decoded.end_date, None);
    assert_eq!(decoded.key_hash.as_deref(), Some(""));
    assert_eq!(decoded.user_id, None);
    let session = decode_session_query(b"session_id=s1&session_id=s2").unwrap();
    assert_eq!(session.session_id, "s2");
}

fn padded_query(size: usize) -> Vec<u8> {
    let mut raw = b"start_date=2024-01-01&pad=".to_vec();
    raw.resize(size, b'x');
    raw
}

fn query_at_the_limit() {
    assert_eq!(MAX_READ_QUERY_BYTES, 1_usize << 20_u32);
    let decoded = decode_benchmarks_query(&padded_query(MAX_READ_QUERY_BYTES)).unwrap();
    assert_eq!(decoded.start_date.as_deref(), Some("2024-01-01"));
}

fn query_past_the_limit() {
    let past_limit = MAX_READ_QUERY_BYTES.saturating_add(1);
    let error = decode_benchmarks_query(&padded_query(past_limit)).unwrap_err();
    assert_eq!(
        error,
        ReadQueryError::TooLarge {
            bytes: past_limit,
            limit: MAX_READ_QUERY_BYTES,
        }
    );
    assert_eq!(ReadApiError::from(error).status_code(), 422);
}

async fn window_bounds() {
    let store = FakeStore::empty();
    benchmarks_for(
        &store,
        &query(Some("2024-02-29"), Some("2024-02-29"), None, None),
        &[],
        day(3, 15),
    )
    .await
    .unwrap();
    assert_eq!(
        store.calls(),
        [RecordedCall::AggRows {
            start: "2024-02-29T00:00:00".to_owned(),
            end: "2024-03-01T00:00:00".to_owned(),
            api_key: None,
            user_id: None,
        }]
    );
}

async fn default_window() {
    let store = FakeStore::empty();
    let absent = benchmarks_for(&store, &query(None, None, None, None), &[], day(1, 5))
        .await
        .unwrap();
    assert_eq!(absent.start_date, "2023-12-06");
    assert_eq!(absent.end_date, "2024-01-05");
    let end_only = benchmarks_for(
        &store,
        &query(None, Some("2024-01-10"), None, None),
        &[],
        day(1, 5),
    )
    .await
    .unwrap();
    assert_eq!(end_only.start_date, "2023-12-11");
    assert_eq!(end_only.end_date, "2024-01-10");
}

async fn filters() {
    let store = FakeStore::empty();
    benchmarks_for(
        &store,
        &query(
            Some("2024-01-01"),
            Some("2024-01-01"),
            Some(HASH_A),
            Some("u1"),
        ),
        &[],
        day(1, 5),
    )
    .await
    .unwrap();
    assert_eq!(
        store.calls(),
        [RecordedCall::AggRows {
            start: "2024-01-01T00:00:00".to_owned(),
            end: "2024-01-02T00:00:00".to_owned(),
            api_key: Some(HASH_A.to_owned()),
            user_id: Some("u1".to_owned()),
        }]
    );
}

fn identity(role: Option<&str>) -> Identity {
    Identity {
        user_id: Some("caller".to_owned()),
        user_role: role.map(str::to_owned),
    }
}

fn member_forbidden() {
    require_admin(
        &identity(Some("proxy_admin_viewer")),
        VIEW_ACTION,
        AdminRole::Viewer,
    )
    .unwrap();
    require_admin(
        &identity(Some("proxy_admin")),
        VIEW_ACTION,
        AdminRole::Viewer,
    )
    .unwrap();
    for role in [None, Some("internal_user"), Some("org_admin")] {
        let error = require_admin(&identity(role), VIEW_ACTION, AdminRole::Viewer).unwrap_err();
        let failure = ReadApiError::from(error);
        assert_eq!(failure.status_code(), 403);
        assert_eq!(
            failure.to_string(),
            "only proxy admin roles can view auto-router benchmarks across the deployment"
        );
    }
}

async fn session_not_found(session_id: &str, expected: &str) {
    let store = FakeStore::empty();
    let error = session_for(
        &store,
        Some(HASH_A),
        &SessionQuery {
            session_id: session_id.to_owned(),
        },
    )
    .await
    .unwrap_err();
    assert_eq!(error.status_code(), 404);
    assert_eq!(
        error.to_string(),
        format!("No auto-routed turns recorded for session {expected} under this key")
    );
    assert_eq!(store.calls().len(), 1);
}

fn sha256_id(value: &str) -> String {
    format!("sha256:{}", hex::encode(Sha256::digest(value.as_bytes())))
}

async fn bounded_session() {
    let exact = "a".repeat(256);
    assert_eq!(bounded_session_id(&exact), exact);
    let exact_store = store_under_its_bounded_id(&exact).await;
    assert_eq!(
        exact_store.calls(),
        [RecordedCall::LatestSession {
            api_key: HASH_A.to_owned(),
            session_id: exact,
        }]
    );
    let oversized = "a".repeat(257);
    let hashed = sha256_id(&oversized);
    assert_eq!(bounded_session_id(&oversized), hashed);
    let hashed_store = store_under_its_bounded_id(&oversized).await;
    assert_eq!(
        hashed_store.calls(),
        [RecordedCall::LatestSession {
            api_key: HASH_A.to_owned(),
            session_id: hashed,
        }]
    );
}

async fn store_under_its_bounded_id(raw: &str) -> FakeStore {
    let bounded = bounded_session_id(raw);
    let store = FakeStore::with_sessions(vec![session(HASH_A, &bounded, "2024-01-02T00:00:00")]);
    let answer = session_for(
        &store,
        Some(HASH_A),
        &SessionQuery {
            session_id: raw.to_owned(),
        },
    )
    .await
    .unwrap();
    assert_eq!(answer.session_id, raw);
    store
}

async fn no_caller_key() {
    let store = FakeStore::with_sessions(vec![session("", "s1", "2024-01-02T00:00:00")]);
    let error = session_for(
        &store,
        None,
        &SessionQuery {
            session_id: "s1".to_owned(),
        },
    )
    .await
    .unwrap_err();
    assert_eq!(error.status_code(), 404);
    assert!(store.calls().is_empty());
}

async fn cancellation() {
    let store = FakeStore::gated();
    let canceled = timeout(
        Duration::from_millis(1),
        benchmarks_for(&store, &query(None, None, None, None), &[], day(1, 5)),
    )
    .await;
    canceled.unwrap_err();
    store.release();
    let answer = benchmarks_for(&store, &query(None, None, None, None), &[], day(1, 5))
        .await
        .unwrap();
    assert_eq!(answer.routers_in_scope, 0);
}

async fn baseline_model_tie() {
    let tied = AutoRouterSession {
        baseline_models: BTreeMap::from([("m1".to_owned(), 2), ("m2".to_owned(), 2)]),
        ..session(HASH_A, "s1", "2024-01-02T00:00:00")
    };
    let store = FakeStore::with_sessions(vec![tied]);
    let answer = session_for(
        &store,
        Some(HASH_A),
        &SessionQuery {
            session_id: "s1".to_owned(),
        },
    )
    .await
    .unwrap();
    assert_eq!(answer.baseline_model.as_deref(), Some("m2"));
}

async fn baseline_model_coverage() {
    let below = AutoRouterSession {
        estimated_turns: 3,
        ..session(HASH_A, "s1", "2024-01-02T00:00:00")
    };
    let store = FakeStore::with_sessions(vec![below]);
    let unnamed = session_for(
        &store,
        Some(HASH_A),
        &SessionQuery {
            session_id: "s1".to_owned(),
        },
    )
    .await
    .unwrap();
    assert_eq!(unnamed.baseline_model, None);
    let covered = AutoRouterSession {
        baseline_models: BTreeMap::from([("m1".to_owned(), 2), ("m2".to_owned(), 2)]),
        ..session(HASH_A, "s1", "2024-01-02T00:00:00")
    };
    let covered_store = FakeStore::with_sessions(vec![covered]);
    let named = session_for(
        &covered_store,
        Some(HASH_A),
        &SessionQuery {
            session_id: "s1".to_owned(),
        },
    )
    .await
    .unwrap();
    assert_eq!(named.baseline_model.as_deref(), Some("m2"));
}

fn principal(subject: &str, source_key: SourceKey) -> Principal {
    Principal {
        principal_type: PrincipalType::Human,
        subject: subject.to_owned(),
        issuer: None,
        audience: Vec::new(),
        user: None,
        organization: None,
        teams: Vec::new(),
        project: None,
        end_user: None,
        roles: Vec::new(),
        scopes: Vec::new(),
        auth_method: AuthMethod::ApiKey,
        credential_ref: CredentialRef::default(),
        network: NetworkContext::default(),
        source_key,
    }
}

fn key_principal(subject: &str, raw: &str) -> Principal {
    let fields = BTreeMap::from([("api_key".to_owned(), JsonData::String(raw.to_owned()))]);
    let key = UserAPIKeyAuth::construct(AuthInput::Dict(fields)).unwrap();
    principal(subject, SourceKey(Some(key)))
}

fn configured(model_name: &str, model: Option<&str>) -> ModelDeployment {
    ModelDeployment {
        model_name: model_name.to_owned(),
        litellm_params: LiteLlmParams {
            ssl_verify: None,
            model: model.map(str::to_owned),
            api_base: None,
            api_key: None,
            api_version: None,
            timeout: None,
            stream_timeout: None,
            max_retries: None,
            rpm: None,
            tpm: None,
            weight: None,
        },
        model_info: ModelInfo { id: None },
    }
}

#[test]
fn the_master_key_uses_the_stable_alias() {
    let master = principal("master_key", SourceKey(None));
    assert_eq!(
        caller_api_key(&master).unwrap().as_deref(),
        Some("litellm_proxy_master_key")
    );
}

#[test]
fn a_virtual_key_is_read_in_its_canonical_form() {
    let keyed = key_principal("key-owner", "sk-test");
    let expected = hex::encode(Sha256::digest(b"sk-test"));
    assert_eq!(caller_api_key(&keyed).unwrap(), Some(expected));
}

#[test]
fn a_caller_without_a_key_has_none() {
    let bare = principal("jwt-user", SourceKey(None));
    assert_eq!(caller_api_key(&bare).unwrap(), None);
}

#[test]
fn the_registry_narrows_to_the_router_key() {
    let deployments = configured_deployments(&[
        configured("complexity-auto", Some("auto_router/complexity_router")),
        configured("plain", Some("gpt-4o")),
        configured("unnamed", None),
    ]);
    assert_eq!(deployments.len(), 3);
    assert_eq!(
        deployments[0].deployment().model_name,
        Some("complexity-auto")
    );
    assert_eq!(
        deployments[0].deployment().model,
        Some("auto_router/complexity_router")
    );
    assert_eq!(deployments[2].deployment().model, None);
}

// the 500 store failures keep their typed cause instead of a flattened string
#[test]
fn every_store_failure_answers_500_with_its_source() {
    let failures = [
        ReadApiError::from(DbError::<Infallible>::NoDbConnected),
        ReadApiError::from(TableError::MissingRow),
        ReadApiError::from(DecodeError::TooLarge { bytes: 2, limit: 1 }),
        ReadApiError::from("nope".parse::<NaiveDateTime>().unwrap_err()),
        ReadApiError::from(FormatError::Utf8 {
            source: String::from_utf8(vec![0xff]).unwrap_err(),
        }),
    ];
    assert!(failures.iter().all(|failure| failure.source().is_some()));
    assert!(
        failures
            .iter()
            .map(ReadApiError::status_code)
            .all(|code| code == 500)
    );
}
