#![cfg(test)]

mod postgres;

use std::collections::BTreeMap;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_auth::auth_identity::records::{
    CredentialRef, NetworkContext, Principal, SourceKey,
};
use betterlite_auth::auth_identity::roles::{AuthMethod, PrincipalType};
use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_db::autorouter_facts::session::bounded_session_id;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::managed_resource_rows::autorouter::AutoRouterSession;
use betterlite_db::managed_resource_rows::wire::RowTimestamp;
use chrono::NaiveDate;
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
use crate::benchmark_aggregates::row::SessionAggRow;
use crate::benchmark_aggregates::window::{BenchmarkWindow, WindowError};
use crate::plugin_routes::routes::Identity;
use crate::routing::auto_router_read_api::ConfiguredRouter;
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
        ..SessionAggRow::default()
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
        api_key: api_key.map(str::to_owned),
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

#[derive(Default)]
struct FakeStore {
    rows: Vec<SessionAggRow>,
    sessions: Vec<AutoRouterSession>,
    calls: Mutex<Vec<RecordedCall>>,
}

impl FakeStore {
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

struct GatedStore {
    gate: Notify,
}

#[async_trait]
impl AutoRouterReadStore for GatedStore {
    async fn agg_rows(
        &self,
        _window: &BenchmarkWindow,
        _api_key: Option<&str>,
        _user_id: Option<&str>,
    ) -> Result<Vec<SessionAggRow>, ReadApiError> {
        self.gate.notified().await;
        Ok(Vec::new())
    }

    async fn latest_session(
        &self,
        _api_key: &str,
        _session_id: &str,
    ) -> Result<Option<AutoRouterSession>, ReadApiError> {
        Ok(None)
    }
}

#[derive(Clone, Debug)]
enum Behavior {
    ComparedComplexity,
    ClassifierCostOnlyWhenCovered,
    NonComplexitySavingsZeroed,
    UnrecordedBaselineOutsideCohort,
    IdleRoutersFromTheRegistry,
    SessionRow,
    SessionWithoutEstimate,
}

#[rstest]
#[case::a_complexity_row_compares_its_savings(Behavior::ComparedComplexity)]
#[case::classifier_cost_only_when_every_turn_recorded(Behavior::ClassifierCostOnlyWhenCovered)]
#[case::a_non_complexity_row_reports_no_savings(Behavior::NonComplexitySavingsZeroed)]
#[case::an_unrecorded_baseline_stays_out_of_the_cohort(Behavior::UnrecordedBaselineOutsideCohort)]
#[case::idle_groups_come_from_the_registry(Behavior::IdleRoutersFromTheRegistry)]
#[case::a_session_reports_its_last_route(Behavior::SessionRow)]
#[case::a_session_without_an_estimate_reports_nulls(Behavior::SessionWithoutEstimate)]
#[tokio::test]
async fn auto_router_read_api_behaviors(#[case] behavior: Behavior) {
    match behavior {
        Behavior::ComparedComplexity => compared_complexity().await,
        Behavior::ClassifierCostOnlyWhenCovered => classifier_cost_only_when_covered().await,
        Behavior::NonComplexitySavingsZeroed => non_complexity_savings_zeroed().await,
        Behavior::UnrecordedBaselineOutsideCohort => unrecorded_baseline_outside_cohort().await,
        Behavior::IdleRoutersFromTheRegistry => idle_routers_from_the_registry().await,
        Behavior::SessionRow => session_row().await,
        Behavior::SessionWithoutEstimate => session_without_estimate().await,
    }
}

fn compared() -> SessionAggRow {
    SessionAggRow {
        sessions: 2,
        turns: 4,
        covered_turns: 4,
        cache_hits: 2,
        total_tokens: 1000,
        session_seconds: money("400"),
        spend: money("2.0"),
        saved_spend: money("2.0"),
        savings_estimated_turns: 1,
        savings_estimated_actual_spend: money("1.5"),
        savings_estimated_saved_spend: money("2.5"),
        savings_classifier_cost: Some(money("0.5")),
        classifier_cost: money("0.5"),
        classifier_cost_recorded_turns: 4,
        tier_turns: BTreeMap::from([("SIMPLE".to_owned(), 4)]),
        ..row("complexity-auto", "complexity")
    }
}

async fn compared_complexity() {
    let store = FakeStore {
        rows: vec![compared()],
        ..FakeStore::default()
    };
    let answer = benchmarks_for(
        &store,
        &query(Some("2024-01-01"), Some("2024-01-02"), None, None),
        &[],
        day(1, 5),
    )
    .await
    .unwrap();
    assert_eq!(answer.start_date, "2024-01-01");
    assert_eq!(answer.end_date, "2024-01-02");
    assert_eq!(answer.routers_in_scope, 1);
    let group = &answer.groups[0];
    // the compared row stands in for the estimate at read time: its own turns and spend compare
    assert_eq!(group.savings_estimated_turns, 4);
    assert_eq!(group.savings_estimated_actual_spend, money("2.0"));
    assert_eq!(group.savings_classifier_cost, Some(money("0.5")));
    assert_eq!(group.saved_spend, Some(money("2.0")));
    assert_eq!(group.baseline_spend, Some(money("4.0")));
    assert_eq!(group.saved_pct, Some(money("50")));
    assert_eq!(group.saved_per_session, Some(money("1")));
    assert_eq!(group.classifier_cost, Some(money("0.5")));
    assert_eq!(group.tier_turns.get("SIMPLE"), Some(&4));
    assert_eq!(answer.totals.sessions, 2);
    assert_eq!(answer.totals.avg_turns_per_session, money("2"));
    assert_eq!(answer.totals.avg_session_seconds, money("200"));
    assert_eq!(answer.totals.avg_tokens_per_session, money("500"));
    assert_eq!(answer.totals.baseline_spend, Some(money("4.0")));
    assert_eq!(
        store.calls(),
        [RecordedCall::AggRows {
            start: "2024-01-01T00:00:00".to_owned(),
            end: "2024-01-03T00:00:00".to_owned(),
            api_key: None,
            user_id: None,
        }]
    );
}

async fn classifier_cost_only_when_covered() {
    let partial = SessionAggRow {
        classifier_cost_recorded_turns: 3,
        ..compared()
    };
    let store = FakeStore {
        rows: vec![partial],
        ..FakeStore::default()
    };
    let answer = benchmarks_for(&store, &query(None, None, None, None), &[], day(1, 5))
        .await
        .unwrap();
    assert_eq!(answer.groups[0].classifier_cost, None);
    assert_eq!(answer.groups[0].savings_classifier_cost, None);
    assert_eq!(answer.totals.classifier_cost, None);
    assert_eq!(answer.totals.savings_classifier_cost, None);
}

async fn non_complexity_savings_zeroed() {
    let adaptive = SessionAggRow {
        router_type: "adaptive".to_owned(),
        ..compared()
    };
    let store = FakeStore {
        rows: vec![adaptive],
        ..FakeStore::default()
    };
    let answer = benchmarks_for(&store, &query(None, None, None, None), &[], day(1, 5))
        .await
        .unwrap();
    let group = &answer.groups[0];
    assert_eq!(group.router_type, "adaptive");
    assert_eq!(group.savings_estimated_turns, 0);
    assert_eq!(group.savings_estimated_actual_spend, Decimal::ZERO);
    assert_eq!(group.savings_classifier_cost, Some(Decimal::ZERO));
    assert_eq!(group.saved_spend, None);
    assert_eq!(group.baseline_spend, None);
    assert_eq!(group.saved_pct, None);
    assert_eq!(group.saved_per_session, None);
    assert_eq!(group.classifier_cost, Some(money("0.5")));
}

async fn unrecorded_baseline_outside_cohort() {
    let unrecorded = SessionAggRow {
        turns: 3,
        savings_estimated_turns: 0,
        spend: money("1.0"),
        saved_spend: Decimal::ZERO,
        savings_estimated_actual_spend: Decimal::ZERO,
        savings_estimated_saved_spend: Decimal::ZERO,
        classifier_cost_recorded_turns: 3,
        ..compared()
    };
    let store = FakeStore {
        rows: vec![unrecorded],
        ..FakeStore::default()
    };
    let answer = benchmarks_for(&store, &query(None, None, None, None), &[], day(1, 5))
        .await
        .unwrap();
    assert_eq!(answer.groups[0].savings_estimated_turns, 0);
    assert_eq!(answer.groups[0].savings_classifier_cost, Some(Decimal::ZERO));
    assert_eq!(answer.groups[0].saved_spend, None);
    assert_eq!(answer.totals.saved_pct, None);
}

async fn idle_routers_from_the_registry() {
    let deployments = vec![
        ConfiguredRouter {
            model_name: "complexity-auto".to_owned(),
            model: Some("auto_router/complexity_router".to_owned()),
        },
        ConfiguredRouter {
            model_name: "idle-quality".to_owned(),
            model: Some("auto_router/quality_router".to_owned()),
        },
        ConfiguredRouter {
            model_name: "idle-adaptive".to_owned(),
            model: Some("auto_router/adaptive_router".to_owned()),
        },
        ConfiguredRouter {
            model_name: "semantic-router".to_owned(),
            model: Some("auto_router/semantic_router".to_owned()),
        },
        ConfiguredRouter {
            model_name: "plain-model".to_owned(),
            model: Some("gpt-4o".to_owned()),
        },
        ConfiguredRouter {
            model_name: String::new(),
            model: Some("auto_router/adaptive_router".to_owned()),
        },
        ConfiguredRouter {
            model_name: "unnamed-model".to_owned(),
            model: None,
        },
    ];
    let store = FakeStore {
        rows: vec![compared()],
        ..FakeStore::default()
    };
    let answer = benchmarks_for(
        &store,
        &query(Some("2024-01-01"), Some("2024-01-01"), None, None),
        &deployments,
        day(1, 5),
    )
    .await
    .unwrap();
    let names: Vec<&str> = answer
        .groups
        .iter()
        .map(|group| group.router_name.as_str())
        .collect();
    assert_eq!(names, ["complexity-auto", "idle-adaptive", "idle-quality"]);
    assert_eq!(answer.groups[1].router_type, "adaptive");
    assert_eq!(answer.groups[2].router_type, "quality");
    assert_eq!(answer.groups[2].sessions, 0);
    assert_eq!(answer.routers_in_scope, 3);
}

async fn session_row() {
    let store = FakeStore {
        sessions: vec![session(HASH_A, "s1", "2024-01-02T00:00:00")],
        ..FakeStore::default()
    };
    let answer = session_for(&store, Some(HASH_A), &SessionQuery {
        session_id: "s1".to_owned(),
    })
    .await
    .unwrap();
    assert_eq!(answer.session_id, "s1");
    assert_eq!(answer.router_name, "complexity-auto");
    assert_eq!(answer.router_type, "complexity");
    assert_eq!(answer.turns, 4);
    assert_eq!(answer.last_model, "gpt-4o");
    assert_eq!(answer.spend, money("1.5"));
    assert_eq!(answer.savings_estimated_turns, 4);
    assert_eq!(answer.savings_estimated_actual_spend, money("1.0"));
    assert_eq!(answer.saved_spend, Some(money("0.5")));
    assert_eq!(answer.baseline_spend, Some(money("2.0")));
    assert_eq!(answer.savings_estimated_baseline_spend, Some(money("1.5")));
    assert_eq!(answer.baseline_model.as_deref(), Some("m1"));
    assert_eq!(answer.baseline_models, BTreeMap::from([
        ("m1".to_owned(), 3),
        ("m2".to_owned(), 1),
    ]));
    assert_eq!(
        store.calls(),
        [RecordedCall::LatestSession {
            api_key: HASH_A.to_owned(),
            session_id: "s1".to_owned(),
        }]
    );
}

async fn session_without_estimate() {
    let unrecorded = AutoRouterSession {
        turns: 2,
        estimated_turns: 0,
        saved_spend: Decimal::ZERO,
        estimated_actual_spend: Decimal::ZERO,
        estimated_saved_spend: Decimal::ZERO,
        baseline_models: BTreeMap::new(),
        ..session(HASH_A, "s2", "2024-01-02T00:00:00")
    };
    let store = FakeStore {
        sessions: vec![unrecorded],
        ..FakeStore::default()
    };
    let answer = session_for(&store, Some(HASH_A), &SessionQuery {
        session_id: "s2".to_owned(),
    })
    .await
    .unwrap();
    assert_eq!(answer.saved_spend, None);
    assert_eq!(answer.baseline_spend, None);
    assert_eq!(answer.savings_estimated_baseline_spend, None);
    assert_eq!(answer.baseline_model, None);
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
    let store = FakeStore::default();
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
    assert!(invalid_date, "expected the invalid-date error, got {error:?}");
    assert!(store.calls().is_empty());
}

async fn reversed_range() {
    let store = FakeStore::default();
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
    assert_eq!(error.to_string(), "the query parameter session_id is invalid");
    assert_eq!(ReadApiError::from(error).status_code(), 422);
}

fn repeated_and_unknown_parameters() {
    let decoded = decode_benchmarks_query(
        b"start_date=2024-01-01&extra=1&start_date=2024-02-02&api_key=",
    )
    .unwrap();
    assert_eq!(decoded.start_date.as_deref(), Some("2024-02-02"));
    assert_eq!(decoded.end_date, None);
    assert_eq!(decoded.api_key.as_deref(), Some(""));
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
    let store = FakeStore::default();
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

async fn filters() {
    let store = FakeStore::default();
    benchmarks_for(
        &store,
        &query(Some("2024-01-01"), Some("2024-01-01"), Some(HASH_A), Some("u1")),
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
    assert!(require_admin(&identity(Some("proxy_admin_viewer")), VIEW_ACTION, AdminRole::Viewer).is_ok());
    assert!(require_admin(&identity(Some("proxy_admin")), VIEW_ACTION, AdminRole::Viewer).is_ok());
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
    let store = FakeStore::default();
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
    let stored_exact = session(HASH_A, &exact, "2024-01-02T00:00:00");
    let exact_store = FakeStore {
        sessions: vec![stored_exact],
        ..FakeStore::default()
    };
    let exact_answer = session_for(&exact_store, Some(HASH_A), &SessionQuery {
        session_id: exact.clone(),
    })
    .await
    .unwrap();
    assert_eq!(exact_answer.session_id, exact);
    assert_eq!(
        exact_store.calls(),
        [RecordedCall::LatestSession {
            api_key: HASH_A.to_owned(),
            session_id: exact,
        }]
    );
    let oversized = "a".repeat(257);
    let bounded = bounded_session_id(&oversized);
    assert_eq!(bounded, sha256_id(&oversized));
    let stored_bounded = session(HASH_A, &bounded, "2024-01-02T00:00:00");
    let bounded_store = FakeStore {
        sessions: vec![stored_bounded],
        ..FakeStore::default()
    };
    let answer = session_for(&bounded_store, Some(HASH_A), &SessionQuery {
        session_id: oversized.clone(),
    })
    .await
    .unwrap();
    assert_eq!(answer.session_id, oversized);
    assert_eq!(
        bounded_store.calls(),
        [RecordedCall::LatestSession {
            api_key: HASH_A.to_owned(),
            session_id: bounded,
        }]
    );
}

async fn no_caller_key() {
    let store = FakeStore {
        sessions: vec![session("", "s1", "2024-01-02T00:00:00")],
        ..FakeStore::default()
    };
    let error = session_for(&store, None, &SessionQuery {
        session_id: "s1".to_owned(),
    })
    .await
    .unwrap_err();
    assert_eq!(error.status_code(), 404);
    assert!(store.calls().is_empty());
}

async fn cancellation() {
    let store = GatedStore {
        gate: Notify::new(),
    };
    let canceled = timeout(
        Duration::from_millis(1),
        benchmarks_for(&store, &query(None, None, None, None), &[], day(1, 5)),
    )
    .await;
    assert!(canceled.is_err());
    store.gate.notify_one();
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
    let store = FakeStore {
        sessions: vec![tied],
        ..FakeStore::default()
    };
    let answer = session_for(&store, Some(HASH_A), &SessionQuery {
        session_id: "s1".to_owned(),
    })
    .await
    .unwrap();
    assert_eq!(answer.baseline_model.as_deref(), Some("m2"));
}

async fn baseline_model_coverage() {
    let below = AutoRouterSession {
        estimated_turns: 3,
        ..session(HASH_A, "s1", "2024-01-02T00:00:00")
    };
    let store = FakeStore {
        sessions: vec![below],
        ..FakeStore::default()
    };
    let unnamed = session_for(&store, Some(HASH_A), &SessionQuery {
        session_id: "s1".to_owned(),
    })
    .await
    .unwrap();
    assert_eq!(unnamed.baseline_model, None);
    let covered = AutoRouterSession {
        baseline_models: BTreeMap::from([("m1".to_owned(), 2), ("m2".to_owned(), 2)]),
        ..session(HASH_A, "s1", "2024-01-02T00:00:00")
    };
    let store = FakeStore {
        sessions: vec![covered],
        ..FakeStore::default()
    };
    let named = session_for(&store, Some(HASH_A), &SessionQuery {
        session_id: "s1".to_owned(),
    })
    .await
    .unwrap();
    assert_eq!(named.baseline_model.as_deref(), Some("m2"));
}

fn principal(subject: &str, key: Option<&str>) -> Principal {
    let source_key = key.map(|raw| {
        let fields = BTreeMap::from([(
            "api_key".to_owned(),
            betterlite_db::budget_permission_rows::JsonData::String(raw.to_owned()),
        )]);
        betterlite_auth::auth_identity::records::SourceKey(Some(
            betterlite_auth::resolved_key::auth::UserAPIKeyAuth::construct(
                betterlite_auth::resolved_key::auth::AuthInput::Dict(fields),
            )
            .unwrap(),
        ))
    });
    Principal {
        principal_type: betterlite_auth::auth_identity::roles::PrincipalType::Human,
        subject: subject.to_owned(),
        source_key: source_key.unwrap_or(betterlite_auth::auth_identity::records::SourceKey(None)),
        ..Principal::default()
    }
}

#[test]
fn the_master_key_uses_the_stable_alias() {
    let master = principal("master_key", None);
    assert_eq!(caller_api_key(&master).unwrap().as_deref(), Some("litellm_proxy_master_key"));
}

#[test]
fn a_virtual_key_is_read_in_its_canonical_form() {
    let keyed = principal("key-owner", Some("sk-test"));
    let expected = format!("{}", hex::encode(Sha256::digest(b"sk-test")));
    assert_eq!(caller_api_key(&keyed).unwrap(), Some(expected));
}

#[test]
fn a_caller_without_a_key_has_none() {
    let bare = principal("jwt-user", None);
    assert_eq!(caller_api_key(&bare).unwrap(), None);
}
