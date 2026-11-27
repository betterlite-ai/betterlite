use rstest::rstest;

use crate::auto_router_read_api::service::SessionResponse;
use crate::benchmark_aggregates::group::Benchmarks;
use crate::routing::auto_router_read_api::ConfiguredRouter;

use super::*;

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
    let store = FakeStore::with_rows(vec![compared()]);
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
    compared_group(&answer);
    compared_totals(&answer);
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

fn compared_group(answer: &Benchmarks) {
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
}

fn compared_totals(answer: &Benchmarks) {
    assert_eq!(answer.totals.sessions, 2);
    assert_eq!(answer.totals.avg_turns_per_session, money("2"));
    assert_eq!(answer.totals.avg_session_seconds, money("200"));
    assert_eq!(answer.totals.avg_tokens_per_session, money("500"));
    assert_eq!(answer.totals.baseline_spend, Some(money("4.0")));
}

async fn classifier_cost_only_when_covered() {
    let partial = SessionAggRow {
        classifier_cost_recorded_turns: 3,
        ..compared()
    };
    let store = FakeStore::with_rows(vec![partial]);
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
    let store = FakeStore::with_rows(vec![adaptive]);
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
    let store = FakeStore::with_rows(vec![unrecorded]);
    let answer = benchmarks_for(&store, &query(None, None, None, None), &[], day(1, 5))
        .await
        .unwrap();
    assert_eq!(answer.groups[0].savings_estimated_turns, 0);
    assert_eq!(
        answer.groups[0].savings_classifier_cost,
        Some(Decimal::ZERO)
    );
    assert_eq!(answer.groups[0].saved_spend, None);
    assert_eq!(answer.totals.saved_pct, None);
}

fn idle_registry() -> Vec<ConfiguredRouter> {
    vec![
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
    ]
}

async fn idle_routers_from_the_registry() {
    let deployments = idle_registry();
    let store = FakeStore::with_rows(vec![compared()]);
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
    let store = FakeStore::with_sessions(vec![session(HASH_A, "s1", "2024-01-02T00:00:00")]);
    let answer = session_for(
        &store,
        Some(HASH_A),
        &SessionQuery {
            session_id: "s1".to_owned(),
        },
    )
    .await
    .unwrap();
    session_identity(&answer);
    session_savings(&answer);
    session_baseline(&answer);
    assert_eq!(
        store.calls(),
        [RecordedCall::LatestSession {
            api_key: HASH_A.to_owned(),
            session_id: "s1".to_owned(),
        }]
    );
}

fn session_identity(answer: &SessionResponse) {
    assert_eq!(answer.session_id, "s1");
    assert_eq!(answer.router_name, "complexity-auto");
    assert_eq!(answer.router_type, "complexity");
    assert_eq!(answer.turns, 4);
    assert_eq!(answer.last_model, "gpt-4o");
}

fn session_savings(answer: &SessionResponse) {
    assert_eq!(answer.spend, money("1.5"));
    assert_eq!(answer.savings_estimated_turns, 4);
    assert_eq!(answer.savings_estimated_actual_spend, money("1.0"));
    assert_eq!(answer.saved_spend, Some(money("0.5")));
    assert_eq!(answer.baseline_spend, Some(money("2.0")));
    assert_eq!(answer.estimated_baseline_spend, Some(money("1.5")));
}

fn session_baseline(answer: &SessionResponse) {
    assert_eq!(answer.baseline_model.as_deref(), Some("m1"));
    assert_eq!(
        answer.baseline_models,
        BTreeMap::from([("m1".to_owned(), 3), ("m2".to_owned(), 1),])
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
    let store = FakeStore::with_sessions(vec![unrecorded]);
    let answer = session_for(
        &store,
        Some(HASH_A),
        &SessionQuery {
            session_id: "s2".to_owned(),
        },
    )
    .await
    .unwrap();
    assert_eq!(answer.saved_spend, None);
    assert_eq!(answer.baseline_spend, None);
    assert_eq!(answer.estimated_baseline_spend, None);
    assert_eq!(answer.baseline_model, None);
}
