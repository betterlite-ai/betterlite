#![cfg(test)]

mod corpus;
mod postgres;
mod state;

use std::cmp::Reverse;
use std::collections::BTreeMap;
use std::sync::Mutex;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use rstest::rstest;
use rust_decimal::Decimal;

use super::access::require_viewer;
use super::error::ShadowResultsRouteError;
use super::query::{
    DEFAULT_SHADOW_LIMIT, MAX_SHADOW_QUERY_BYTES, ShadowQuery, ShadowQueryError,
    decode_shadow_query,
};
use super::service::{get_shadow_eval_job, list_shadow_eval_jobs};
use super::store::{ShadowLegReads, ShadowTargetFilter};
use crate::plugin_routes::routes::Identity;
use crate::shadow_eval_job_loading::jobs::ShadowEvalDirection;
use crate::shadow_eval_models::ShadowEvalStatus;
use crate::shadow_eval_models::ShadowEvalTargetType;
use crate::shadow_eval_models::ShadowEvalTargetType::{Key, Team, User};
use crate::shadow_eval_models::job::ShadowJobResponse;
use crate::shadow_result_aggregation::aggregation::{AttemptAggRow, GroupBy};
use crate::shadow_result_aggregation::error::ShadowResultsError;
use crate::shadow_result_aggregation::labels::{LabelKind, LabelRow};
use crate::shadow_result_aggregation::legs::LegRow;
use crate::shadow_result_aggregation::repository::{RepositoryResult, ShadowResultsRepository};
use crate::shadow_result_aggregation::responses::{
    AttemptTotals, ErrorAttempt, LegAttemptCount, latest_error,
};
use crate::shadow_result_aggregation::results::FunnelTotals;

fn at(seconds: i64) -> DateTime<Utc> {
    DateTime::from_timestamp(seconds, 0).unwrap()
}

fn money(text: &str) -> Decimal {
    text.parse().unwrap()
}

fn leg(id: &str, group: &str, target_type: ShadowEvalTargetType, target_id: &str) -> LegRow {
    LegRow {
        id: id.to_owned(),
        group_id: group.to_owned(),
        target_type,
        target_id: target_id.to_owned(),
        router_name: "router-a".to_owned(),
        router_names: vec!["router-a".to_owned()],
        models: vec!["gpt-5.2".to_owned()],
        direction: ShadowEvalDirection::Forward,
        baseline_model: None,
        judge_model: "anthropic/claude-sonnet-5".to_owned(),
        shadow_percentage: 10.0,
        max_turns: 10_000,
        max_budget: Some(money("5")),
        created_at: at(0),
        ends_at: at(10_000),
        stopped_at: None,
        stopped_by: None,
    }
}

fn count(job_id: &str, attempt_count: i64, spend: &str) -> LegAttemptCount {
    LegAttemptCount {
        job_id: job_id.to_owned(),
        attempt_count,
        spend: money(spend),
    }
}

fn agg(group: &str, turns: i64) -> AttemptAggRow {
    AttemptAggRow {
        group: group.to_owned(),
        turn_count: turns,
        real_wins: turns,
        shadow_wins: 0,
        ties: 0,
        avg_confidence: Some(0.9_f64),
        real_spend: money("0.1"),
        shadow_spend: money("0.2"),
        cache_hit_turns: 0,
    }
}

fn label(target_id: &str, alias: Option<&str>, key_name: Option<&str>) -> LabelRow {
    LabelRow {
        target_id: target_id.to_owned(),
        target_alias: alias.map(str::to_owned),
        key_name: key_name.map(str::to_owned),
    }
}

fn error_attempt(job_id: &str, seconds: i64, error: Option<&str>) -> (String, ErrorAttempt) {
    (
        job_id.to_owned(),
        ErrorAttempt {
            error: error.map(str::to_owned),
            created_at: at(seconds),
        },
    )
}

#[derive(Default)]
struct FakeStore {
    legs: Vec<LegRow>,
    counts: Vec<LegAttemptCount>,
    totals: Option<AttemptTotals>,
    errors: Vec<(String, ErrorAttempt)>,
    by_tier: Vec<AttemptAggRow>,
    by_model: Vec<AttemptAggRow>,
    by_leg: Vec<AttemptAggRow>,
    by_router: Vec<AttemptAggRow>,
    funnel: Option<FunnelTotals>,
    keys: Vec<LabelRow>,
    teams: Vec<LabelRow>,
    users: Vec<LabelRow>,
    list_calls: Mutex<Vec<(i64, Option<ShadowTargetFilter>)>>,
}

impl FakeStore {
    fn recording_list_calls(&self) -> Vec<(i64, Option<ShadowTargetFilter>)> {
        self.list_calls.lock().unwrap().clone()
    }
}

#[async_trait]
impl ShadowLegReads for FakeStore {
    async fn list_legs(
        &self,
        limit: i64,
        target: Option<ShadowTargetFilter>,
    ) -> RepositoryResult<Vec<LegRow>> {
        self.list_calls
            .lock()
            .unwrap()
            .push((limit, target.clone()));
        let mut selected: BTreeMap<String, Vec<&LegRow>> = BTreeMap::new();
        for leg in &self.legs {
            let matches = target.as_ref().is_none_or(|filter| {
                leg.target_type == filter.target_type && leg.target_id == filter.target_id
            });
            if matches {
                selected.entry(leg.group_id.clone()).or_default().push(leg);
            }
        }
        let mut groups: Vec<(String, Vec<&LegRow>)> = selected.into_iter().collect();
        groups.sort_by_key(|(_group, legs)| Reverse(legs.iter().map(|leg| leg.created_at).max()));
        let keep = usize::try_from(limit).unwrap();
        let wanted: Vec<&str> = groups
            .iter()
            .take(keep)
            .map(|(group, _legs)| group.as_str())
            .collect();
        Ok(self
            .legs
            .iter()
            .filter(|leg| wanted.contains(&leg.group_id.as_str()))
            .cloned()
            .collect())
    }

    async fn legs_of_group(&self, job_id: &str) -> RepositoryResult<Vec<LegRow>> {
        Ok(self
            .legs
            .iter()
            .filter(|leg| leg.group_id == job_id)
            .cloned()
            .collect())
    }
}

#[async_trait]
impl ShadowResultsRepository for FakeStore {
    async fn attempt_aggregates(
        &self,
        _leg_ids: &[String],
        group_by: GroupBy,
    ) -> RepositoryResult<Vec<AttemptAggRow>> {
        let rows = match group_by {
            GroupBy::Tier => &self.by_tier,
            GroupBy::Model => &self.by_model,
            GroupBy::Leg => &self.by_leg,
            GroupBy::Router => &self.by_router,
        };
        Ok(rows.clone())
    }

    async fn attempt_totals(&self, _leg_ids: &[String]) -> RepositoryResult<AttemptTotals> {
        self.totals.ok_or(ShadowResultsError::NoTotalsRow)
    }

    async fn attempt_count_rows(
        &self,
        leg_ids: &[String],
    ) -> RepositoryResult<Vec<LegAttemptCount>> {
        let found: Vec<LegAttemptCount> = leg_ids
            .iter()
            .flat_map(|leg_id| self.counts.iter().filter(move |row| row.job_id == *leg_id))
            .cloned()
            .collect();
        Ok(found)
    }

    async fn funnel_totals(&self, _leg_ids: &[String]) -> RepositoryResult<Option<FunnelTotals>> {
        Ok(self.funnel)
    }

    async fn latest_error(&self, leg_ids: &[String]) -> RepositoryResult<Option<String>> {
        let attempts: Vec<ErrorAttempt> = self
            .errors
            .iter()
            .filter(|(job_id, _attempt)| leg_ids.contains(job_id))
            .map(|(_job_id, attempt)| attempt.clone())
            .collect();
        Ok(latest_error(&attempts))
    }

    async fn label_rows(
        &self,
        kind: LabelKind,
        target_ids: &[String],
    ) -> RepositoryResult<Vec<LabelRow>> {
        let rows = match kind {
            LabelKind::Key => &self.keys,
            LabelKind::Team => &self.teams,
            LabelKind::User => &self.users,
        };
        Ok(rows
            .iter()
            .filter(|row| target_ids.contains(&row.target_id))
            .cloned()
            .collect())
    }
}

struct ListCase {
    store: FakeStore,
    query: ShadowQuery,
    now: DateTime<Utc>,
    expected_jobs: Vec<&'static str>,
    expected_targets: Vec<(&'static str, Vec<(ShadowEvalTargetType, &'static str)>)>,
    expected_attempts: Vec<(&'static str, i64, Option<Decimal>)>,
    expected_labels: Vec<(&'static str, Option<&'static str>, Option<&'static str>)>,
    expected_statuses: Vec<(&'static str, ShadowEvalStatus)>,
}

struct StatusCase {
    store: FakeStore,
    now: DateTime<Utc>,
    expected: Vec<(&'static str, ShadowEvalStatus)>,
}

struct DetailCase {
    store: FakeStore,
    job_id: &'static str,
    expected: DetailExpectation,
}

struct DetailExpectation {
    judged_count: i64,
    error_count: i64,
    judge_spend: Decimal,
    last_error: Option<&'static str>,
    status: ShadowEvalStatus,
    targets: Vec<(
        ShadowEvalTargetType,
        &'static str,
        i64,
        Decimal,
        Option<&'static str>,
    )>,
    overall_shadow_win: Option<f64>,
}

enum Behavior {
    List(ListCase),
    Statuses(StatusCase),
    Detail(DetailCase),
    Viewer {
        user_role: Option<&'static str>,
        store: bool,
        expected_status: Option<u16>,
    },
}

fn plain_query(limit: i64) -> ShadowQuery {
    ShadowQuery {
        target_type: None,
        target_id: None,
        limit,
    }
}

fn list_store() -> FakeStore {
    let mut older = leg("leg-1", "job-old", Team, "t1");
    older.created_at = at(100);
    let mut newer = leg("leg-2", "job-new", Key, "k1");
    newer.created_at = at(300);
    let mut sibling = leg("leg-3", "job-new", User, "u1");
    sibling.created_at = at(200);
    FakeStore {
        legs: vec![older, newer, sibling],
        counts: vec![count("leg-1", 2, "0.5"), count("leg-2", 1, "0.25")],
        totals: None,
        errors: Vec::new(),
        by_tier: Vec::new(),
        by_model: Vec::new(),
        by_leg: Vec::new(),
        by_router: Vec::new(),
        funnel: None,
        keys: vec![label("k1", Some("key alias"), Some("sk-...abcd"))],
        teams: vec![label("t1", Some("team alias"), None)],
        users: Vec::new(),
        list_calls: Mutex::new(Vec::new()),
    }
}

#[rstest]
#[case::list_orders_newest_first_and_keeps_every_leg(Behavior::List(ListCase {
    store: list_store(),
    query: plain_query(50),
    now: at(500),
    expected_jobs: vec!["job-new", "job-old"],
    expected_targets: vec![
        ("job-new", vec![(Key, "k1"), (User, "u1")]),
        ("job-old", vec![(Team, "t1")]),
    ],
    expected_attempts: vec![
        ("k1", 1, Some(money("0.25"))),
        ("u1", 0, Some(Decimal::ZERO)),
        ("t1", 2, Some(money("0.5"))),
    ],
    expected_labels: vec![
        ("k1", Some("key alias"), Some("sk-...abcd")),
        ("u1", None, None),
        ("t1", Some("team alias"), None),
    ],
    expected_statuses: vec![
        ("job-new", ShadowEvalStatus::Running),
        ("job-old", ShadowEvalStatus::Running),
    ],
}))]
#[case::list_applies_the_target_filter_to_whole_jobs(Behavior::List(ListCase {
    store: list_store(),
    query: ShadowQuery {
        target_type: Some(Key),
        target_id: Some("k1".to_owned()),
        limit: 50,
    },
    now: at(500),
    expected_jobs: vec!["job-new"],
    expected_targets: vec![("job-new", vec![(Key, "k1"), (User, "u1")])],
    expected_attempts: vec![("k1", 1, Some(money("0.25"))), ("u1", 0, Some(Decimal::ZERO))],
    expected_labels: vec![("k1", Some("key alias"), Some("sk-...abcd")), ("u1", None, None)],
    expected_statuses: vec![("job-new", ShadowEvalStatus::Running)],
}))]
#[case::list_keeps_only_the_newest_limit_jobs(Behavior::List(ListCase {
    store: list_store(),
    query: plain_query(1),
    now: at(500),
    expected_jobs: vec!["job-new"],
    expected_targets: vec![("job-new", vec![(Key, "k1"), (User, "u1")])],
    expected_attempts: vec![("k1", 1, Some(money("0.25"))), ("u1", 0, Some(Decimal::ZERO))],
    expected_labels: vec![("k1", Some("key alias"), Some("sk-...abcd")), ("u1", None, None)],
    expected_statuses: vec![("job-new", ShadowEvalStatus::Running)],
}))]
#[case::list_derives_stopped_completed_and_budget_statuses(Behavior::Statuses(StatusCase {
    store: {
        let mut by_stop = leg("leg-1", "job-stop", Key, "k1");
        by_stop.stopped_by = Some("alice".to_owned());
        let mut by_window = leg("leg-2", "job-window", Team, "t1");
        by_window.ends_at = at(400);
        let mut by_budget = leg("leg-3", "job-budget", User, "u1");
        by_budget.max_budget = Some(money("0.5"));
        let mut running = leg("leg-4", "job-running", Team, "t2");
        running.max_budget = None;
        FakeStore {
            legs: vec![by_stop, by_window, by_budget, running],
            counts: vec![count("leg-3", 4, "0.5")],
            ..FakeStore::default()
        }
    },
    now: at(500),
    expected: vec![
        ("job-budget", ShadowEvalStatus::Completed),
        ("job-running", ShadowEvalStatus::Running),
        ("job-stop", ShadowEvalStatus::Stopped),
        ("job-window", ShadowEvalStatus::Completed),
    ],
}))]
#[case::detail_carries_totals_error_results_and_verdicts(Behavior::Detail(DetailCase {
    store: FakeStore {
        legs: vec![leg("leg-1", "job-1", Key, "k1"), leg("leg-2", "job-1", User, "u1")],
        counts: vec![count("leg-1", 2, "0.5")],
        totals: Some(AttemptTotals {
            judged_count: 7,
            error_count: 2,
            judge_spend: money("0.1234567"),
        }),
        errors: vec![error_attempt("leg-1", 5, Some("old")), error_attempt("leg-2", 9, Some("new"))],
        by_tier: vec![agg("simple", 3)],
        by_leg: vec![agg("leg-1", 3)],
        ..FakeStore::default()
    },
    job_id: "job-1",
    expected: DetailExpectation {
        judged_count: 7,
        error_count: 2,
        judge_spend: money("0.123457"),
        last_error: Some("new"),
        status: ShadowEvalStatus::Running,
        targets: vec![(Key, "k1", 2, money("0.5"), Some("k1")), (User, "u1", 0, Decimal::ZERO, None)],
        overall_shadow_win: Some(0.0_f64),
    },
}))]
#[case::detail_defaults_totals_when_the_read_has_no_row(Behavior::Detail(DetailCase {
    store: FakeStore {
        legs: vec![leg("leg-1", "job-1", Key, "k1")],
        ..FakeStore::default()
    },
    job_id: "job-1",
    expected: DetailExpectation {
        judged_count: 0,
        error_count: 0,
        judge_spend: Decimal::ZERO,
        last_error: None,
        status: ShadowEvalStatus::Running,
        targets: vec![(Key, "k1", 0, Decimal::ZERO, None)],
        overall_shadow_win: None,
    },
}))]
#[case::viewer_accepts_both_admin_roles(Behavior::Viewer {
    user_role: Some("proxy_admin"),
    store: true,
    expected_status: None,
})]
#[case::viewer_accepts_the_view_only_admin(Behavior::Viewer {
    user_role: Some("proxy_admin_viewer"),
    store: true,
    expected_status: None,
})]
#[case::viewer_refuses_an_internal_user(Behavior::Viewer {
    user_role: Some("internal_user"),
    store: true,
    expected_status: Some(403),
})]
#[case::viewer_refuses_a_missing_role(Behavior::Viewer {
    user_role: None,
    store: true,
    expected_status: Some(403),
})]
#[case::viewer_needs_a_store_after_the_role(Behavior::Viewer {
    user_role: Some("proxy_admin"),
    store: false,
    expected_status: Some(500),
})]
#[tokio::test]
async fn shadow_eval_results_behaviors(#[case] behavior: Behavior) {
    match behavior {
        Behavior::List(case) => assert_list(case).await,
        Behavior::Statuses(case) => assert_statuses(case).await,
        Behavior::Detail(case) => assert_detail(case).await,
        Behavior::Viewer {
            user_role,
            store,
            expected_status,
        } => assert_viewer(user_role, store, expected_status),
    }
}

async fn assert_list(case: ListCase) {
    let store = &case.store;
    let jobs = list_shadow_eval_jobs(store, &case.query, case.now)
        .await
        .unwrap();
    let seen: Vec<&str> = jobs.iter().map(|job| job.job.job_id.as_str()).collect();
    assert_eq!(seen, case.expected_jobs);
    assert_targets(&jobs, &case.expected_targets);
    assert_attempts(&jobs, &case.expected_attempts);
    assert_labels(&jobs, &case.expected_labels);
    let statuses: Vec<(&str, ShadowEvalStatus)> = jobs
        .iter()
        .map(|job| (job.job.job_id.as_str(), job.status))
        .collect();
    assert_eq!(statuses, case.expected_statuses);
    let calls = store.recording_list_calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls.first().unwrap().0, case.query.limit);
}

fn assert_targets(
    jobs: &[ShadowJobResponse],
    expected: &[(&str, Vec<(ShadowEvalTargetType, &str)>)],
) {
    let seen: Vec<(&str, Vec<(ShadowEvalTargetType, &str)>)> = jobs
        .iter()
        .map(|job| {
            let targets = job
                .job
                .targets
                .iter()
                .map(|target| (target.target_type, target.target_id.as_str()))
                .collect();
            (job.job.job_id.as_str(), targets)
        })
        .collect();
    assert_eq!(seen, expected);
}

fn assert_attempts(jobs: &[ShadowJobResponse], expected: &[(&str, i64, Option<Decimal>)]) {
    let seen: Vec<(&str, i64, Option<Decimal>)> = jobs
        .iter()
        .flat_map(|job| job.job.targets.iter())
        .map(|target| {
            (
                target.target_id.as_str(),
                target.attempt_count.unwrap(),
                target.spend,
            )
        })
        .collect();
    assert_eq!(seen, expected);
}

async fn assert_statuses(case: StatusCase) {
    let jobs = list_shadow_eval_jobs(&case.store, &plain_query(50), case.now)
        .await
        .unwrap();
    let seen: Vec<(&str, ShadowEvalStatus)> = jobs
        .iter()
        .map(|job| (job.job.job_id.as_str(), job.status))
        .collect();
    assert_eq!(seen, case.expected);
}

fn assert_labels(jobs: &[ShadowJobResponse], expected: &[(&str, Option<&str>, Option<&str>)]) {
    let mut seen: Vec<(&str, Option<&str>, Option<&str>)> = Vec::new();
    for target in jobs.iter().flat_map(|job| job.job.targets.iter()) {
        seen.push((
            target.target_id.as_str(),
            target.target_alias.as_deref(),
            target.key_name.as_deref(),
        ));
    }
    assert_eq!(seen, expected);
}

async fn assert_detail(case: DetailCase) {
    let job = get_shadow_eval_job(&case.store, case.job_id, at(500))
        .await
        .unwrap();
    let detail = &job.job;
    assert_eq!(detail.judged_count, Some(case.expected.judged_count));
    assert_eq!(detail.error_count, Some(case.expected.error_count));
    assert_eq!(detail.judge_spend, Some(case.expected.judge_spend));
    assert_eq!(detail.last_error.as_deref(), case.expected.last_error);
    assert_eq!(job.status, case.expected.status);
    assert_eq!(
        detail
            .results
            .as_ref()
            .map(|results| results.overall_shadow_win_rate_pct),
        case.expected.overall_shadow_win
    );
    let targets: Vec<(ShadowEvalTargetType, &str, i64, Decimal, Option<&str>)> = detail
        .targets
        .iter()
        .map(|target| {
            (
                target.target_type,
                target.target_id.as_str(),
                target.attempt_count.unwrap(),
                target.spend.unwrap(),
                target.verdicts.as_ref().map(|row| row.group.as_str()),
            )
        })
        .collect();
    assert_eq!(targets, case.expected.targets);
}

fn assert_viewer(user_role: Option<&'static str>, store: bool, expected_status: Option<u16>) {
    let identity = Identity {
        user_id: Some("caller".to_owned()),
        user_role: user_role.map(str::to_owned),
    };
    let fake = FakeStore::default();
    let outcome = require_viewer(&identity, "view shadow evals", store.then_some(&fake));
    match expected_status {
        None => {
            outcome.unwrap();
        }
        Some(status) => {
            let error = outcome.err().unwrap();
            assert_eq!(error.status_code(), status);
        }
    }
}

enum Boundary {
    Query {
        raw: Vec<u8>,
        expected: Option<ShadowQuery>,
    },
    QueryError {
        raw: Vec<u8>,
        error: ShadowQueryError,
    },
    Pair {
        target_type: Option<ShadowEvalTargetType>,
        target_id: Option<&'static str>,
    },
    MissingJob {
        store: Box<FakeStore>,
        job_id: &'static str,
    },
    EmptyList,
}

fn query(
    target_type: Option<ShadowEvalTargetType>,
    target_id: Option<&str>,
    limit: i64,
) -> ShadowQuery {
    ShadowQuery {
        target_type,
        target_id: target_id.map(str::to_owned),
        limit,
    }
}

// a valid query dragged out to an exact byte length by an unknown parameter both stacks ignore
fn padded(bytes: usize) -> Vec<u8> {
    let mut raw = b"limit=50&pad=".to_vec();
    raw.resize(bytes, b'x');
    raw
}

// a limit spelled as a run of nines of a chosen length, with text on either side of the run
fn digit_limit(prefix: &str, digits: usize, suffix: &str) -> Vec<u8> {
    format!("limit={prefix}{}{suffix}", "9".repeat(digits)).into_bytes()
}

// the pydantic issue FastAPI's validation body carries for one rejected query parameter
const fn invalid(
    parameter: &'static str,
    kind: &'static str,
    msg: &'static str,
) -> ShadowQueryError {
    ShadowQueryError::Invalid {
        parameter,
        kind,
        msg,
    }
}

#[rstest]
#[case::query_defaults_to_fifty(Boundary::Query { raw: Vec::new(), expected: Some(query(None, None, DEFAULT_SHADOW_LIMIT)) })]
#[case::query_reads_every_parameter(Boundary::Query {
    raw: b"target_type=key&target_id=k1&limit=7".to_vec(),
    expected: Some(query(Some(Key), Some("k1"), 7)),
})]
#[case::query_keeps_the_last_repeated_value(Boundary::Query {
    raw: b"limit=5&limit=9&target_type=team&target_type=user".to_vec(),
    expected: Some(query(Some(User), None, 9)),
})]
#[case::query_ignores_unknown_parameters(Boundary::Query {
    raw: b"skip=10&limit=2&other=value".to_vec(),
    expected: Some(query(None, None, 2)),
})]
#[case::query_accepts_an_explicit_empty_target_id(Boundary::Query {
    raw: b"target_type=key&target_id=".to_vec(),
    expected: Some(query(Some(Key), Some(""), DEFAULT_SHADOW_LIMIT)),
})]
#[case::query_accepts_the_lowest_limit(Boundary::Query { raw: b"limit=1".to_vec(), expected: Some(query(None, None, 1)) })]
#[case::query_accepts_the_highest_limit(Boundary::Query { raw: b"limit=200".to_vec(), expected: Some(query(None, None, 200)) })]
#[case::query_accepts_a_signed_limit(Boundary::Query { raw: b"limit=%2B7".to_vec(), expected: Some(query(None, None, 7)) })]
#[case::query_accepts_exactly_the_byte_bound(Boundary::Query {
    raw: padded(MAX_SHADOW_QUERY_BYTES),
    expected: Some(query(None, None, DEFAULT_SHADOW_LIMIT)),
})]
#[case::query_accepts_a_trailing_ampersand(Boundary::Query {
    raw: b"limit=3&".to_vec(),
    expected: Some(query(None, None, 3)),
})]
#[case::query_rejects_zero_limit(Boundary::QueryError { raw: b"limit=0".to_vec(), error: invalid("limit", "greater_than_equal", "Input should be greater than or equal to 1") })]
#[case::query_rejects_a_limit_over_two_hundred(Boundary::QueryError { raw: b"limit=201".to_vec(), error: invalid("limit", "less_than_equal", "Input should be less than or equal to 200") })]
#[case::query_rejects_a_limit_above_the_signed_range(Boundary::QueryError { raw: b"limit=99999999999999999999".to_vec(), error: invalid("limit", "less_than_equal", "Input should be less than or equal to 200") })]
#[case::query_rejects_a_limit_below_the_signed_range(Boundary::QueryError { raw: b"limit=-99999999999999999999".to_vec(), error: invalid("limit", "greater_than_equal", "Input should be greater than or equal to 1") })]
#[case::query_rejects_an_overflow_with_trailing_text(Boundary::QueryError { raw: b"limit=99999999999999999999x".to_vec(), error: invalid("limit", "int_parsing", "Input should be a valid integer, unable to parse string as an integer") })]
#[case::query_rejects_an_empty_limit(Boundary::QueryError { raw: b"limit=".to_vec(), error: invalid("limit", "int_parsing", "Input should be a valid integer, unable to parse string as an integer") })]
#[case::query_rejects_a_fractional_limit(Boundary::QueryError { raw: b"limit=1.5".to_vec(), error: invalid("limit", "int_parsing", "Input should be a valid integer, unable to parse string as an integer") })]
#[case::query_rejects_an_unknown_target_type(Boundary::QueryError { raw: b"target_type=org".to_vec(), error: invalid("target_type", "literal_error", "Input should be 'key', 'team' or 'user'") })]
#[case::query_rejects_an_empty_target_type(Boundary::QueryError { raw: b"target_type=".to_vec(), error: invalid("target_type", "literal_error", "Input should be 'key', 'team' or 'user'") })]
#[case::query_reports_target_type_before_an_invalid_limit(Boundary::QueryError { raw: b"target_type=org&limit=0".to_vec(), error: invalid("target_type", "literal_error", "Input should be 'key', 'team' or 'user'") })]
#[case::query_rejects_one_byte_past_the_bound(Boundary::QueryError { raw: padded(MAX_SHADOW_QUERY_BYTES + 1), error: ShadowQueryError::TooLarge { bytes: MAX_SHADOW_QUERY_BYTES + 1, limit: MAX_SHADOW_QUERY_BYTES } })]
#[case::query_bounds_a_4300_digit_limit(Boundary::QueryError { raw: digit_limit("", 4_300, ""), error: invalid("limit", "less_than_equal", "Input should be less than or equal to 200") })]
#[case::query_rejects_a_4301_digit_limit(Boundary::QueryError { raw: digit_limit("", 4_301, ""), error: invalid("limit", "int_parsing_size", "Unable to parse input string as an integer, exceeded maximum size") })]
#[case::query_bounds_a_4299_digit_negative_limit(Boundary::QueryError { raw: digit_limit("-", 4_299, ""), error: invalid("limit", "greater_than_equal", "Input should be greater than or equal to 1") })]
#[case::query_rejects_a_4300_digit_negative_limit(Boundary::QueryError { raw: digit_limit("-", 4_300, ""), error: invalid("limit", "int_parsing_size", "Unable to parse input string as an integer, exceeded maximum size") })]
#[case::query_rejects_a_padded_4301_digit_limit(Boundary::QueryError { raw: digit_limit("%20", 4_301, ""), error: invalid("limit", "int_parsing", "Input should be a valid integer, unable to parse string as an integer") })]
#[case::query_rejects_a_signed_4301_digit_limit(Boundary::QueryError { raw: digit_limit("%2B", 4_301, ""), error: invalid("limit", "int_parsing", "Input should be a valid integer, unable to parse string as an integer") })]
#[case::query_rejects_a_zero_padded_4301_digit_limit(Boundary::QueryError { raw: digit_limit("0", 4_301, ""), error: invalid("limit", "int_parsing", "Input should be a valid integer, unable to parse string as an integer") })]
#[case::query_rejects_a_4301_digit_limit_with_trailing_text(Boundary::QueryError { raw: digit_limit("", 4_301, "x"), error: invalid("limit", "int_parsing_size", "Unable to parse input string as an integer, exceeded maximum size") })]
#[case::query_accepts_an_underscored_limit(Boundary::Query { raw: b"limit=1_0".to_vec(), expected: Some(query(None, None, 10)) })]
#[case::query_accepts_a_zero_fraction_limit(Boundary::Query { raw: b"limit=7.0".to_vec(), expected: Some(query(None, None, 7)) })]
#[case::query_accepts_a_zero_padded_limit(Boundary::Query { raw: b"limit=00000200".to_vec(), expected: Some(query(None, None, 200)) })]
#[case::query_accepts_a_zero_then_underscored_limit(Boundary::Query { raw: b"limit=0_5".to_vec(), expected: Some(query(None, None, 5)) })]
#[case::query_rejects_a_zero_fraction_zero_limit(Boundary::QueryError { raw: b"limit=0.0".to_vec(), error: invalid("limit", "greater_than_equal", "Input should be greater than or equal to 1") })]
#[case::query_rejects_an_underscored_zero_limit(Boundary::QueryError { raw: b"limit=0_0".to_vec(), error: invalid("limit", "greater_than_equal", "Input should be greater than or equal to 1") })]
#[case::query_bounds_a_cleaned_negative_limit(Boundary::QueryError { raw: b"limit=-007".to_vec(), error: invalid("limit", "greater_than_equal", "Input should be greater than or equal to 1") })]
#[case::query_rejects_a_plus_then_minus_limit(Boundary::QueryError { raw: b"limit=%2B-5".to_vec(), error: invalid("limit", "int_parsing", "Input should be a valid integer, unable to parse string as an integer") })]
#[case::query_rejects_a_leading_underscore_limit(Boundary::QueryError { raw: b"limit=_10".to_vec(), error: invalid("limit", "int_parsing", "Input should be a valid integer, unable to parse string as an integer") })]
#[case::query_rejects_a_trailing_underscore_limit(Boundary::QueryError { raw: b"limit=10_".to_vec(), error: invalid("limit", "int_parsing", "Input should be a valid integer, unable to parse string as an integer") })]
#[case::query_rejects_a_doubled_underscore_limit(Boundary::QueryError { raw: b"limit=1__0".to_vec(), error: invalid("limit", "int_parsing", "Input should be a valid integer, unable to parse string as an integer") })]
#[case::list_rejects_a_type_without_an_id(Boundary::Pair { target_type: Some(Key), target_id: None })]
#[case::list_rejects_an_id_without_a_type(Boundary::Pair { target_type: None, target_id: Some("k1") })]
#[case::detail_answers_404_for_an_unknown_job(Boundary::MissingJob { store: Box::default(), job_id: "job-none" })]
#[case::list_answers_empty_for_an_empty_store(Boundary::EmptyList)]
#[tokio::test]
async fn shadow_eval_results_boundaries(#[case] boundary: Boundary) {
    match boundary {
        Boundary::Query { raw, expected } => {
            assert_eq!(decode_shadow_query(&raw).unwrap(), expected.unwrap());
        }
        Boundary::QueryError { raw, error } => {
            assert_eq!(decode_shadow_query(&raw).unwrap_err(), error);
        }
        Boundary::Pair {
            target_type,
            target_id,
        } => assert_pair(target_type, target_id).await,
        Boundary::MissingJob { store, job_id } => assert_missing_job(&store, job_id).await,
        Boundary::EmptyList => {
            let jobs = list_shadow_eval_jobs(&FakeStore::default(), &plain_query(50), at(500))
                .await
                .unwrap();
            assert!(jobs.is_empty());
        }
    }
}

async fn assert_pair(target_type: Option<ShadowEvalTargetType>, target_id: Option<&str>) {
    let store = FakeStore::default();
    let outcome = list_shadow_eval_jobs(&store, &query(target_type, target_id, 50), at(500)).await;
    let error = outcome.err().unwrap();
    assert_eq!(error.status_code(), 400);
}

async fn assert_missing_job(store: &FakeStore, job_id: &str) {
    let outcome = get_shadow_eval_job(store, job_id, at(500)).await;
    match outcome {
        Err(ShadowResultsRouteError::JobNotFound { job_id: named }) => assert_eq!(named, job_id),
        other => panic!("{other:?}"),
    }
}
