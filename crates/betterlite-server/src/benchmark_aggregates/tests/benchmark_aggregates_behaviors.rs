use std::collections::{BTreeMap, BTreeSet};
use std::str::FromStr as _;

use chrono::NaiveDate;
use rstest::rstest;
use rust_decimal::Decimal;

use crate::benchmark_aggregates::group::{benchmark_group, benchmarks, summed_agg_row};
use crate::benchmark_aggregates::row::{SessionAggRow, compared_row, savings_cohort};
use crate::benchmark_aggregates::totals::{BenchmarkTotals, benchmark_totals, pct};
use crate::benchmark_aggregates::window::BenchmarkWindow;
use crate::routing::benchmark_aggregates::{
    StrategyDeployment, idle_router_groups, strategy_router_key,
};

fn dec(text: &str) -> Decimal {
    Decimal::from_str(text).unwrap()
}

fn complexity_row() -> SessionAggRow {
    SessionAggRow {
        router_name: "complexity-auto".to_owned(),
        router_type: "complexity".to_owned(),
        tier_turns: BTreeMap::from([("SIMPLE".to_owned(), 1_i64), ("MEDIUM".to_owned(), 2_i64)]),
        sessions: 2,
        turns: 4,
        unordered_turns: 3,
        covered_turns: 4,
        cache_hits: 2,
        same_model_turns: 1,
        same_model_hits: 1,
        first_visit_turns: 1,
        first_visit_hits: 1,
        return_turns: 5,
        return_hits: 1,
        return_expired_misses: 1,
        return_within_ttl_misses: 1,
        ttl_5m_turns: 2,
        ttl_1h_turns: 3,
        total_tokens: 1024,
        spend: dec("2.0"),
        saved_spend: dec("2.0"),
        savings_estimated_turns: 2,
        savings_estimated_actual_spend: dec("1.5"),
        savings_classifier_cost: Some(dec("0.5")),
        savings_estimated_saved_spend: dec("2.5"),
        classifier_cost: dec("0.5"),
        classifier_cost_recorded_turns: 4,
        session_seconds: dec("7200.0"),
    }
}

fn window() -> BenchmarkWindow {
    let today = NaiveDate::from_ymd_opt(2024, 1, 2).unwrap();
    BenchmarkWindow::resolve(Some("2024-01-01"), Some("2024-01-02"), today).unwrap()
}

#[rstest]
#[case(1, 4, "25")]
#[case(2, 4, "50")]
#[case(4, 4, "100")]
#[case(0, 4, "0")]
#[case(1, 8, "12.5")]
#[case(1, 16, "6.2")]
#[case(3, 16, "18.8")]
#[case(7, 16, "43.8")]
#[case(5, 16, "31.2")]
#[case(1, 3, "33.3")]
#[case(2, 3, "66.7")]
#[case(4, 0, "0")]
#[case(4, -1, "0")]
fn the_pct_rounds_like_pythons_float_round(
    #[case] numerator: i64,
    #[case] denominator: i64,
    #[case] expected: &str,
) {
    assert_eq!(
        pct(Decimal::from(numerator), Decimal::from(denominator)),
        dec(expected)
    );
}

#[rstest]
#[case(4, 0, ("2.0", "0"), None, None)]
#[case(4, 2, ("1.5", "2.5"), Some("2.5"), Some("4.0"))]
#[case(0, 0, ("0", "0"), Some("0"), Some("0"))]
#[case(4, 0, ("2.0", "1.0"), Some("1.0"), Some("3.0"))]
#[case(4, 3, ("0", "0"), Some("0"), Some("0"))]
fn the_savings_cohort_needs_a_recorded_baseline(
    #[case] turns: i64,
    #[case] estimated_turns: i64,
    #[case] amounts: (&str, &str),
    #[case] expected_saved: Option<&str>,
    #[case] expected_baseline: Option<&str>,
) {
    let (spend, saved_spend) = amounts;
    let (saved, baseline) = savings_cohort(turns, estimated_turns, dec(spend), dec(saved_spend));
    assert_eq!(saved, expected_saved.map(dec));
    assert_eq!(baseline, expected_baseline.map(dec));
}

#[test]
fn a_complexity_row_with_a_baseline_joins_the_comparison() {
    let row = complexity_row();
    let compared = compared_row(&row);
    assert_eq!(compared.savings_estimated_turns, 4);
    assert_eq!(compared.savings_estimated_actual_spend, dec("2"));
    assert_eq!(compared.savings_estimated_saved_spend, dec("2"));
    assert_eq!(compared.savings_classifier_cost, Some(dec("0.5")));
    assert_eq!(compared.spend, row.spend);
    assert_eq!(compared.router_name, row.router_name);
}

#[test]
fn a_complexity_row_without_a_baseline_is_zeroed() {
    let mut row = complexity_row();
    row.savings_estimated_turns = 0;
    row.saved_spend = Decimal::ZERO;
    let compared = compared_row(&row);
    assert_eq!(compared.savings_estimated_turns, 0);
    assert_eq!(compared.savings_estimated_actual_spend, Decimal::ZERO);
    assert_eq!(compared.savings_estimated_saved_spend, Decimal::ZERO);
    assert_eq!(compared.savings_classifier_cost, Some(Decimal::ZERO));
}

#[test]
fn an_adaptive_row_never_compares_even_with_a_baseline() {
    let mut row = complexity_row();
    row.router_type = "adaptive".to_owned();
    let compared = compared_row(&row);
    assert_eq!(compared.savings_estimated_turns, 0);
    assert_eq!(compared.savings_classifier_cost, Some(Decimal::ZERO));
}

#[test]
fn a_comparison_with_unrecorded_classifier_cost_reports_none() {
    let mut row = complexity_row();
    row.classifier_cost_recorded_turns = 3;
    assert_eq!(compared_row(&row).savings_classifier_cost, None);
}

fn assert_savings_totals(totals: &BenchmarkTotals) {
    assert_eq!(
        (totals.classifier_cost, totals.savings_classifier_cost),
        (Some(dec("0.5")), Some(dec("0.5")))
    );
    assert_eq!(
        (
            totals.savings_estimated_actual_spend,
            totals.saved_spend,
            totals.baseline_spend,
        ),
        (dec("1.5"), Some(dec("2.5")), Some(dec("4")))
    );
    assert_eq!(
        (totals.saved_pct, totals.saved_per_session),
        (Some(dec("62.5")), Some(dec("1.25")))
    );
}

fn assert_cache_totals(totals: &BenchmarkTotals) {
    let (coverage, hit_rate) = (totals.cache.coverage_pct, totals.cache.hit_rate_pct);
    assert_eq!((coverage, hit_rate), (dec("100"), dec("50")));
    assert_eq!(
        (
            totals.cache.same_model.turns,
            totals.cache.same_model.hits,
            totals.cache.same_model.hit_rate_pct,
        ),
        (1, 1, dec("100"))
    );
    assert_eq!(
        (
            totals.cache.first_visit.turns,
            totals.cache.first_visit.hits,
            totals.cache.first_visit.hit_rate_pct,
        ),
        (1, 1, dec("100"))
    );
    assert_eq!(
        (
            totals.cache.return_to_tier.turns,
            totals.cache.return_to_tier.hits,
            totals.cache.return_to_tier.hit_rate_pct,
        ),
        (5, 1, dec("20"))
    );
    assert_eq!(
        (
            totals.cache.unordered_turns,
            totals.cache.return_misses_expired,
            totals.cache.return_misses_within_ttl,
            totals.cache.return_misses_unknown,
        ),
        (3, 1, 1, 2)
    );
    assert_eq!(
        (totals.cache.ttl_5m_turns, totals.cache.ttl_1h_turns),
        (2, 3)
    );
}

#[test]
fn the_totals_follow_the_row() {
    let totals = benchmark_totals(&complexity_row());
    assert_eq!(
        (
            totals.sessions,
            totals.turns,
            totals.savings_estimated_turns
        ),
        (2, 4, 2)
    );
    assert_eq!(
        (
            totals.avg_turns_per_session,
            totals.avg_session_seconds,
            totals.avg_tokens_per_session,
            totals.spend,
        ),
        (dec("2"), dec("3600"), dec("512"), dec("2"))
    );
    assert_savings_totals(&totals);
    assert_cache_totals(&totals);
}

#[test]
fn a_row_without_recorded_classifier_turns_reports_none() {
    let mut row = complexity_row();
    row.classifier_cost_recorded_turns = 3;
    assert_eq!(benchmark_totals(&row).classifier_cost, None);
}

#[test]
fn a_row_without_sessions_divides_nothing() {
    let mut row = complexity_row();
    row.sessions = 0;
    let totals = benchmark_totals(&row);
    assert_eq!(totals.avg_turns_per_session, Decimal::ZERO);
    assert_eq!(totals.avg_session_seconds, Decimal::ZERO);
    assert_eq!(totals.avg_tokens_per_session, Decimal::ZERO);
    assert_eq!(totals.saved_per_session, Some(Decimal::ZERO));
}

#[test]
fn a_row_without_a_cohort_reports_no_savings() {
    let mut row = complexity_row();
    row.savings_estimated_turns = 0;
    row.savings_estimated_saved_spend = Decimal::ZERO;
    let totals = benchmark_totals(&row);
    assert_eq!(totals.saved_spend, None);
    assert_eq!(totals.baseline_spend, None);
    assert_eq!(totals.saved_pct, None);
    assert_eq!(totals.saved_per_session, None);
}

#[test]
fn negative_return_misses_clamp_to_zero() {
    let mut row = complexity_row();
    row.return_turns = 1;
    row.return_hits = 2;
    row.return_expired_misses = 1;
    row.return_within_ttl_misses = 1;
    let totals = benchmark_totals(&row);
    assert_eq!(totals.cache.return_misses_unknown, 0);
    assert_eq!(totals.cache.return_to_tier.hit_rate_pct, dec("200"));
}

#[test]
fn the_unknown_return_misses_exclude_the_attributed_ones() {
    let mut row = complexity_row();
    row.return_turns = 5;
    row.return_hits = 1;
    row.return_expired_misses = 2;
    row.return_within_ttl_misses = 1;
    assert_eq!(benchmark_totals(&row).cache.return_misses_unknown, 1);
}

#[test]
fn a_group_carries_the_router_and_its_totals() {
    let group = benchmark_group(&complexity_row());
    assert_eq!(group.router_name, "complexity-auto");
    assert_eq!(group.router_type, "complexity");
    assert_eq!(group.tier_turns.get("MEDIUM"), Some(&2));
    assert_eq!(group.turns, 4);
    assert_eq!(group.spend, dec("2"));
    assert_eq!(group.cache.return_to_tier.hit_rate_pct, dec("20"));
}

fn two_rows_to_sum() -> [SessionAggRow; 2] {
    let mut second = complexity_row();
    second.turns = 2;
    second.sessions = 1;
    second.total_tokens = 512;
    second.spend = dec("1.5");
    second.saved_spend = dec("0.5");
    second.savings_estimated_turns = 2;
    second.savings_estimated_actual_spend = dec("1.0");
    second.savings_estimated_saved_spend = dec("0.5");
    second.savings_classifier_cost = Some(dec("0.25"));
    second.classifier_cost = dec("0.25");
    second.classifier_cost_recorded_turns = 2;
    second.session_seconds = dec("3600.0");
    [complexity_row(), second]
}

#[test]
fn the_summed_row_adds_every_measure() {
    let summed = summed_agg_row(&two_rows_to_sum());
    assert_eq!(
        (summed.router_name.as_str(), summed.router_type.as_str()),
        ("", "")
    );
    assert_eq!(summed.tier_turns, BTreeMap::new());
    assert_eq!(
        (summed.sessions, summed.turns, summed.total_tokens),
        (3, 6, 1536)
    );
    assert_eq!(
        (
            summed.spend,
            summed.saved_spend,
            summed.savings_estimated_actual_spend,
            summed.savings_estimated_saved_spend,
        ),
        (dec("3.5"), dec("2.5"), dec("2.5"), dec("3.0"))
    );
    assert_eq!(
        (
            summed.savings_estimated_turns,
            summed.savings_classifier_cost,
            summed.classifier_cost,
            summed.classifier_cost_recorded_turns,
        ),
        (4, Some(dec("0.75")), dec("0.75"), 6)
    );
    assert_eq!(summed.session_seconds, dec("10800.0"));
}

#[test]
fn an_empty_sum_is_the_zero_row() {
    let summed = summed_agg_row(&[]);
    assert_eq!(summed.turns, 0);
    assert_eq!(summed.spend, Decimal::ZERO);
    assert_eq!(summed.savings_classifier_cost, Some(Decimal::ZERO));
}

#[test]
fn a_summed_row_is_unavailable_when_any_classifier_cost_is() {
    let mut partial = complexity_row();
    partial.classifier_cost_recorded_turns = 3;
    let rows = [complexity_row(), partial];
    let compared: Vec<SessionAggRow> = rows.iter().map(compared_row).collect();
    assert_eq!(summed_agg_row(&compared).savings_classifier_cost, None);
    let response = benchmarks(&window(), &rows, None);
    assert_eq!(response.totals.savings_classifier_cost, None);
    assert_eq!(response.totals.classifier_cost, None);
    assert_eq!(response.groups[1].savings_classifier_cost, None);
    assert_eq!(response.groups[0].savings_classifier_cost, Some(dec("0.5")));
}

#[test]
fn a_summed_total_compares_when_any_row_recorded_a_baseline() {
    let mut second = complexity_row();
    second.savings_estimated_turns = 0;
    second.savings_estimated_actual_spend = Decimal::ZERO;
    second.savings_estimated_saved_spend = Decimal::ZERO;
    second.savings_classifier_cost = Some(Decimal::ZERO);
    second.saved_spend = Decimal::ZERO;
    let compared: Vec<SessionAggRow> = [complexity_row(), second]
        .iter()
        .map(compared_row)
        .collect();
    let totals = benchmark_totals(&summed_agg_row(&compared));
    assert_eq!(totals.savings_estimated_turns, 4);
    assert_eq!(totals.saved_spend, Some(dec("2")));
    assert_eq!(totals.baseline_spend, Some(dec("4")));
    assert_eq!(totals.savings_classifier_cost, Some(dec("0.5")));
}

fn mixed_deployments() -> [StrategyDeployment<'static>; 7] {
    [
        StrategyDeployment {
            model_name: Some("complexity-auto"),
            model: Some("auto_router/complexity_router"),
        },
        StrategyDeployment {
            model_name: Some("idle-quality"),
            model: Some("auto_router/quality_router"),
        },
        StrategyDeployment {
            model_name: Some("idle-adaptive"),
            model: Some("auto_router/adaptive_router"),
        },
        StrategyDeployment {
            model_name: Some("semantic"),
            model: Some("auto_router/semantic_router"),
        },
        StrategyDeployment {
            model_name: Some("plain"),
            model: Some("gpt-4o"),
        },
        StrategyDeployment {
            model_name: Some(""),
            model: Some("auto_router/adaptive_router"),
        },
        StrategyDeployment {
            model_name: Some("unnamed-model"),
            model: None,
        },
    ]
}

#[test]
fn idle_strategy_routers_get_zeroed_groups_after_the_recorded_ones() {
    let deployments = mixed_deployments();
    let response = benchmarks(&window(), &[complexity_row()], Some(&deployments));
    assert_eq!(response.start_date, "2024-01-01");
    assert_eq!(response.end_date, "2024-01-02");
    assert_eq!(response.routers_in_scope, 3);
    let names: Vec<&str> = response
        .groups
        .iter()
        .map(|group| group.router_name.as_str())
        .collect();
    assert_eq!(names, ["complexity-auto", "idle-adaptive", "idle-quality"]);
    assert_eq!(response.groups[1].router_type, "adaptive");
    assert_eq!(response.groups[1].turns, 0);
    assert_eq!(response.groups[1].cache.coverage_pct, Decimal::ZERO);
    assert_eq!(response.groups[1].classifier_cost, Some(Decimal::ZERO));
    assert_eq!(response.groups[1].saved_spend, Some(Decimal::ZERO));
    assert_eq!(response.groups[2].router_type, "quality");
    assert_eq!(response.totals.turns, 4);
    assert_eq!(response.totals.savings_estimated_turns, 4);
}

#[test]
fn a_window_without_a_deployment_list_has_no_idle_groups() {
    let response = benchmarks(&window(), &[complexity_row()], None);
    assert_eq!(response.routers_in_scope, 1);
    assert_eq!(response.groups.len(), 1);
}

#[test]
fn an_idle_group_list_is_empty_when_every_router_is_covered() {
    let deployments = [StrategyDeployment {
        model_name: Some("complexity-auto"),
        model: Some("auto_router/complexity_router"),
    }];
    let covered = BTreeSet::from([("complexity-auto".to_owned(), "complexity".to_owned())]);
    let groups = idle_router_groups(Some(&deployments), &covered);
    assert!(groups.is_empty());
}

#[rstest]
#[case(Some("auto-a"), Some("auto_router/complexity_router"), Some(("auto-a", "complexity")))]
#[case(Some("auto-a"), Some("auto_router/adaptive_router"), Some(("auto-a", "adaptive")))]
#[case(Some("auto-a"), Some("auto_router/quality_router"), Some(("auto-a", "quality")))]
#[case(Some("auto-a"), Some("auto_router/semantic_router"), None)]
#[case(Some("auto-a"), Some("auto_router/anything"), None)]
#[case(Some("auto-a"), Some("openai/gpt-4o"), None)]
#[case(Some("auto-a"), None, None)]
#[case(Some(""), Some("auto_router/adaptive_router"), None)]
#[case(None, Some("auto_router/adaptive_router"), None)]
fn a_deployment_contributes_its_strategy_router_key(
    #[case] model_name: Option<&str>,
    #[case] model: Option<&str>,
    #[case] expected: Option<(&str, &str)>,
) {
    let deployment = StrategyDeployment { model_name, model };
    assert_eq!(
        strategy_router_key(deployment),
        expected.map(|(name, kind)| (name.to_owned(), kind.to_owned()))
    );
}
