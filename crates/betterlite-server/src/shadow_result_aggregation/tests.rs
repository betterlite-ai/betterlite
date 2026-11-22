#![cfg(test)]

mod postgres;

use chrono::{DateTime, Utc};
use rstest::rstest;
use rust_decimal::Decimal;
use std::fmt::Debug;

use super::aggregation::{AttemptAggRow, pct_of, round_to, slices};
use super::error::ShadowResultsError;
use super::labels::{LabelRow, label_responses, target_ids_of};
use super::legs::LegRow;
use super::responses::{
    AttemptTotals, ErrorAttempt, LegAttemptCount, LegCounts, detail_response, group_response,
    latest_error,
};
use super::results::{FunnelTotals, JobAggregates, JobResults, TargetVerdicts, stratified_results};
use super::targets::{TargetKey, parse_target_type};
use crate::shadow_eval_job_loading::jobs::ShadowEvalDirection;
use crate::shadow_eval_models::ShadowEvalTargetType;
use crate::shadow_eval_models::ShadowEvalTargetType::{Key, Team, User};
use crate::shadow_eval_models::job::ShadowJob;
use crate::shadow_eval_models::leg::ShadowLeg;
use crate::shadow_eval_models::results::{ShadowEvalResult, ShadowEvalSlice};

fn at(seconds: i64) -> DateTime<Utc> {
    DateTime::from_timestamp(seconds, 0).unwrap()
}

fn money(value: f64) -> Decimal {
    Decimal::from_f64_retain(value).unwrap()
}

fn spelled(text: &str) -> Decimal {
    text.parse().unwrap()
}

fn leg(id: &str, target_type: ShadowEvalTargetType, target_id: &str) -> LegRow {
    LegRow {
        id: id.to_owned(),
        group_id: "job-1".to_owned(),
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
        max_budget: Some(money(5.0)),
        created_at: at(0),
        ends_at: at(1_000),
        stopped_at: None,
        stopped_by: None,
    }
}

fn count(job_id: &str, attempt_count: i64, spend: f64) -> LegAttemptCount {
    LegAttemptCount {
        job_id: job_id.to_owned(),
        attempt_count,
        spend: money(spend),
    }
}

fn agg(
    group: &str,
    turns: i64,
    counts: (i64, i64, i64, i64),
    confidence: Option<f64>,
    spends: (f64, f64),
) -> AttemptAggRow {
    AttemptAggRow {
        group: group.to_owned(),
        turn_count: turns,
        real_wins: counts.0,
        shadow_wins: counts.1,
        ties: counts.2,
        cache_hit_turns: counts.3,
        avg_confidence: confidence,
        real_spend: money(spends.0),
        shadow_spend: money(spends.1),
    }
}

#[derive(Clone, Copy)]
struct SliceRow {
    group: &'static str,
    turns: i64,
    rates: (f64, f64, f64),
    confidence: f64,
    spends: (f64, f64),
    cache_hits: i64,
}

fn slice(row: SliceRow) -> ShadowEvalSlice {
    let (real_win_rate_pct, shadow_win_rate_pct, tie_rate_pct) = row.rates;
    let (real_spend, shadow_spend) = row.spends;
    ShadowEvalSlice {
        group: row.group.to_owned(),
        turn_count: row.turns,
        real_win_rate_pct,
        shadow_win_rate_pct,
        tie_rate_pct,
        avg_judge_confidence: row.confidence,
        real_spend: money(real_spend),
        shadow_spend: money(shadow_spend),
        cache_hit_turns: row.cache_hits,
    }
}

fn label_row(target_id: &str, target_alias: Option<&str>, key_name: Option<&str>) -> LabelRow {
    LabelRow {
        target_id: target_id.to_owned(),
        target_alias: target_alias.map(str::to_owned),
        key_name: key_name.map(str::to_owned),
    }
}

fn error_attempt(seconds: i64, error: Option<&str>) -> ErrorAttempt {
    ErrorAttempt {
        error: error.map(str::to_owned),
        created_at: at(seconds),
    }
}

fn job(targets: Vec<ShadowLeg>) -> ShadowJob {
    ShadowJob {
        job_id: "job-1".to_owned(),
        targets,
        router_names: vec!["router-a".to_owned()],
        models: Vec::new(),
        direction: ShadowEvalDirection::Forward,
        baseline_model: None,
        judge_model: "anthropic/claude-sonnet-5".to_owned(),
        shadow_percentage: 10.0,
        created_at: at(0),
        ends_at: at(1_000),
        stopped_by: None,
        judged_count: None,
        error_count: None,
        judge_spend: None,
        last_error: None,
        results: None,
    }
}

fn target_leg(target_type: ShadowEvalTargetType, target_id: &str) -> ShadowLeg {
    ShadowLeg {
        target_type,
        target_id: target_id.to_owned(),
        max_turns: 10_000,
        max_budget: None,
        stopped_at: None,
        attempt_count: None,
        spend: None,
        verdicts: None,
        target_alias: None,
        key_name: None,
    }
}

struct GroupCase {
    legs: Vec<LegRow>,
    counts: Vec<LegAttemptCount>,
    expected: GroupExpectation,
}

struct GroupExpectation {
    job_id: &'static str,
    router_names: Vec<&'static str>,
    stopped_by: Option<&'static str>,
    targets: Vec<(ShadowEvalTargetType, &'static str, i64, Decimal)>,
}

struct SlicesCase {
    rows: Vec<AttemptAggRow>,
    expected: Vec<ShadowEvalSlice>,
}

struct ResultsCase {
    legs: Vec<LegRow>,
    by_tier: Vec<AttemptAggRow>,
    by_model: Vec<AttemptAggRow>,
    by_leg: Vec<AttemptAggRow>,
    by_router: Vec<AttemptAggRow>,
    funnel: Option<FunnelTotals>,
    expected: ResultsExpectation,
}

struct ResultsExpectation {
    tier_groups: Vec<&'static str>,
    model_groups: Vec<&'static str>,
    router_groups: Vec<&'static str>,
    overall_shadow_win: f64,
    overall_tie: f64,
    real_spend: Decimal,
    shadow_spend: Decimal,
    funnel: Option<(i64, i64, i64, i64)>,
    verdicts: Vec<(ShadowEvalTargetType, &'static str, &'static str)>,
}

struct LabelsCase {
    responses: Vec<ShadowJob>,
    keys: Vec<LabelRow>,
    teams: Vec<LabelRow>,
    users: Vec<LabelRow>,
    expected: Vec<(
        ShadowEvalTargetType,
        &'static str,
        Option<&'static str>,
        Option<&'static str>,
    )>,
}

struct LatestErrorCase {
    attempts: Vec<ErrorAttempt>,
    expected: Option<&'static str>,
}

struct DetailCase {
    legs: Vec<LegRow>,
    totals: AttemptTotals,
    last_error: Option<&'static str>,
    by_tier: Vec<AttemptAggRow>,
    by_leg: Vec<AttemptAggRow>,
    expected: DetailExpectation,
}

struct DetailExpectation {
    judged_count: i64,
    error_count: i64,
    judge_spend: Decimal,
    last_error: Option<&'static str>,
    overall_shadow_win: f64,
    verdicts: Vec<(ShadowEvalTargetType, &'static str, Option<&'static str>)>,
}

enum Behavior {
    Group(GroupCase),
    Slices(SlicesCase),
    Results(ResultsCase),
    Labels(LabelsCase),
    LatestError(LatestErrorCase),
    Detail(DetailCase),
}

#[rstest]
#[case::group_response_orders_targets_and_applies_counts(Behavior::Group(GroupCase {
    legs: vec![
        {
            let mut first = leg("leg-user", User, "u1");
            first.router_names.clear();
            first.router_name = "router-first".to_owned();
            first.stopped_by = Some("alice".to_owned());
            first
        },
        {
            let mut second = leg("leg-team", Team, "t1");
            second.stopped_by = Some("bob".to_owned());
            second
        },
        leg("leg-key", Key, "k1"),
    ],
    counts: vec![count("leg-key", 3, 0.123_456_7), count("leg-user", 4, 0.5)],
    expected: GroupExpectation {
        job_id: "job-7",
        router_names: vec!["router-first"],
        stopped_by: Some("alice"),
        targets: vec![
            (Key, "k1", 3, spelled("0.123457")),
            (Team, "t1", 0, spelled("0.0")),
            (User, "u1", 4, spelled("0.5")),
        ],
    },
}))]
#[case::group_response_orders_targets_by_kind_before_id(Behavior::Group(GroupCase {
    legs: vec![leg("leg-user", User, "a"), leg("leg-key", Key, "z")],
    counts: Vec::new(),
    expected: GroupExpectation {
        job_id: "job-9",
        router_names: vec!["router-a"],
        stopped_by: None,
        targets: vec![
            (Key, "z", 0, Decimal::ZERO),
            (User, "a", 0, Decimal::ZERO),
        ],
    },
}))]
#[case::slices_rank_the_busiest_group_first(Behavior::Slices(SlicesCase {
    rows: vec![
        agg("simple", 4, (3, 1, 0, 1), Some(0.875_1_f64), (0.1_f64, 0.2_f64)),
        agg("complex", 2, (0, 1, 1, 0), None, (0.0_f64, 0.0_f64)),
        agg("equal-turn", 2, (2, 0, 0, 2), Some(0.5_f64), (0.3_f64, 0.0_f64)),
    ],
    expected: vec![
        slice(SliceRow {
            group: "simple",
            turns: 4,
            rates: (75.0_f64, 25.0_f64, 0.0_f64),
            confidence: 0.875_f64,
            spends: (0.1_f64, 0.2_f64),
            cache_hits: 1,
        }),
        slice(SliceRow {
            group: "complex",
            turns: 2,
            rates: (0.0_f64, 50.0_f64, 50.0_f64),
            confidence: 0.0_f64,
            spends: (0.0_f64, 0.0_f64),
            cache_hits: 0,
        }),
        slice(SliceRow {
            group: "equal-turn",
            turns: 2,
            rates: (100.0_f64, 0.0_f64, 0.0_f64),
            confidence: 0.5_f64,
            spends: (0.3_f64, 0.0_f64),
            cache_hits: 2,
        }),
    ],
}))]
#[case::results_sum_tiers_and_key_verdicts_by_target(Behavior::Results(ResultsCase {
    legs: vec![leg("leg-key", Key, "k1"), leg("leg-team", Team, "t1")],
    by_tier: vec![
        agg("simple", 3, (1, 2, 0, 0), Some(0.9_f64), (0.1_f64, 0.2_f64)),
        agg("complex", 2, (1, 0, 1, 0), Some(0.9_f64), (0.3_f64, 0.1_f64)),
    ],
    by_model: vec![agg(
        "gpt-5.2",
        5,
        (2, 2, 1, 0),
        Some(0.9_f64),
        (0.4_f64, 0.3_f64),
    )],
    by_leg: vec![
        agg("leg-key", 3, (1, 1, 1, 0), Some(0.7_f64), (0.2_f64, 0.1_f64)),
        agg("leg-team", 2, (1, 1, 0, 0), Some(0.8_f64), (0.2_f64, 0.2_f64)),
    ],
    by_router: vec![agg(
        "router-a",
        5,
        (2, 2, 1, 0),
        Some(0.9_f64),
        (0.4_f64, 0.3_f64),
    )],
    funnel: None,
    expected: ResultsExpectation {
        tier_groups: vec!["simple", "complex"],
        model_groups: vec!["gpt-5.2"],
        router_groups: vec!["router-a"],
        overall_shadow_win: 40.0,
        overall_tie: 20.0,
        real_spend: money(0.4),
        shadow_spend: money(0.300_000_000_000_000_04),
        funnel: None,
        verdicts: vec![(Key, "k1", "k1"), (Team, "t1", "t1")],
    },
}))]
#[case::results_carry_a_fully_covered_funnel(Behavior::Results(ResultsCase {
    legs: vec![leg("leg-key", Key, "k1"), leg("leg-team", Team, "t1")],
    by_tier: vec![agg("simple", 2, (1, 1, 0, 0), Some(0.5_f64), (0.0_f64, 0.0_f64))],
    by_model: Vec::new(),
    by_leg: Vec::new(),
    by_router: Vec::new(),
    funnel: Some(FunnelTotals {
        legs_with_rows: 2,
        not_sampled: 7,
        unjudgeable: 3,
        shed: 2,
        withheld: 1,
    }),
    expected: ResultsExpectation {
        tier_groups: vec!["simple"],
        model_groups: Vec::new(),
        router_groups: Vec::new(),
        overall_shadow_win: 50.0,
        overall_tie: 0.0,
        real_spend: money(0.0),
        shadow_spend: money(0.0),
        funnel: Some((7, 3, 2, 1)),
        verdicts: Vec::new(),
    },
}))]
#[case::labels_read_each_kind_and_keep_the_last_row(Behavior::Labels(LabelsCase {
    responses: vec![job(vec![
        target_leg(Key, "k1"),
        target_leg(Team, "t1"),
        target_leg(User, "u1"),
        target_leg(Key, "deleted"),
    ])],
    keys: vec![
        label_row("k1", Some("old-alias"), Some("sk-old")),
        label_row("k1", Some("key alias"), Some("sk-...abcd")),
    ],
    teams: vec![label_row("t1", Some("team alias"), None)],
    users: vec![label_row("u1", Some("user@example.com"), None)],
    expected: vec![
        (Key, "k1", Some("key alias"), Some("sk-...abcd")),
        (Team, "t1", Some("team alias"), None),
        (User, "u1", Some("user@example.com"), None),
        (Key, "deleted", None, None),
    ],
}))]
#[case::latest_error_takes_the_newest_attempt(Behavior::LatestError(LatestErrorCase {
    attempts: vec![
        error_attempt(5, Some("old")),
        error_attempt(9, Some("new")),
        error_attempt(1, Some("older")),
    ],
    expected: Some("new"),
}))]
#[case::latest_error_keeps_a_newest_attempt_without_a_message(Behavior::LatestError(LatestErrorCase {
    attempts: vec![error_attempt(9, None), error_attempt(5, Some("old"))],
    expected: None,
}))]
#[case::detail_sets_totals_spend_and_each_targets_verdict(Behavior::Detail(DetailCase {
    legs: vec![leg("leg-key", Key, "k1"), leg("leg-team", Team, "t1")],
    totals: AttemptTotals {
        judged_count: 7,
        error_count: 2,
        judge_spend: money(0.123_456_7),
    },
    last_error: Some("judge timed out"),
    by_tier: vec![agg("simple", 5, (2, 2, 1, 0), Some(0.9_f64), (0.4_f64, 0.3_f64))],
    by_leg: vec![
        agg("leg-key", 3, (1, 1, 1, 0), Some(0.7_f64), (0.2_f64, 0.1_f64)),
        agg("leg-team", 2, (1, 1, 0, 0), Some(0.8_f64), (0.2_f64, 0.2_f64)),
    ],
    expected: DetailExpectation {
        judged_count: 7,
        error_count: 2,
        judge_spend: spelled("0.123457"),
        last_error: Some("judge timed out"),
        overall_shadow_win: 40.0,
        verdicts: vec![(Key, "k1", Some("k1")), (Team, "t1", Some("t1"))],
    },
}))]
fn shadow_result_aggregation_behaviors(#[case] behavior: Behavior) {
    match behavior {
        Behavior::Group(case) => assert_group(&case),
        Behavior::Slices(case) => assert_slices(&case),
        Behavior::Results(case) => assert_results(&case),
        Behavior::Labels(case) => assert_labels(&case),
        Behavior::LatestError(case) => assert_latest_error(&case),
        Behavior::Detail(case) => assert_detail(&case),
    }
}

fn assert_group(case: &GroupCase) {
    let counts: LegCounts = case
        .counts
        .iter()
        .map(|row| (row.job_id.clone(), row.clone()))
        .collect();
    let response = group_response(case.expected.job_id, &case.legs, &counts).unwrap();
    assert_group_config(&response, &case.expected);
    assert_group_targets(&response, &case.expected);
}

fn assert_group_config(response: &ShadowJob, expected: &GroupExpectation) {
    assert_eq!(response.job_id, expected.job_id);
    assert_eq!(response.router_names, expected.router_names);
    assert_eq!(response.stopped_by.as_deref(), expected.stopped_by);
    assert_eq!(response.models, vec!["gpt-5.2".to_owned()]);
    assert_eq!(response.direction, ShadowEvalDirection::Forward);
    assert_eq!(response.created_at, at(0));
    assert_eq!(response.ends_at, at(1_000));
    assert_eq!(response.judged_count, None);
    assert_eq!(response.results, None);
}

fn assert_group_targets(response: &ShadowJob, expected: &GroupExpectation) {
    let targets: Vec<(ShadowEvalTargetType, String, Option<i64>, Option<Decimal>)> = response
        .targets
        .iter()
        .map(|target| {
            (
                target.target_type,
                target.target_id.clone(),
                target.attempt_count,
                target.spend,
            )
        })
        .collect();
    let wanted: Vec<_> = expected
        .targets
        .iter()
        .map(|(kind, id, attempts, spend)| (*kind, (*id).to_owned(), Some(*attempts), Some(*spend)))
        .collect();
    assert_eq!(targets, wanted);
}

fn assert_slices(case: &SlicesCase) {
    assert_eq!(slices(&case.rows).unwrap(), case.expected);
}

fn assert_results(case: &ResultsCase) {
    let aggregates = JobAggregates {
        by_tier: &case.by_tier,
        by_model: &case.by_model,
        by_leg: &case.by_leg,
        by_router: &case.by_router,
        funnel: case.funnel,
    };
    let outcome = stratified_results(&case.legs, &aggregates).unwrap();
    let result = outcome.results.clone().unwrap();
    assert_result_slices(&result, &case.expected);
    assert_result_totals(&result, &case.expected);
    assert_result_verdicts(&outcome.verdicts, &case.expected);
}

fn assert_result_slices(result: &ShadowEvalResult, expected: &ResultsExpectation) {
    assert_eq!(groups(&result.by_tier), expected.tier_groups);
    assert_eq!(groups(&result.by_current_model), expected.model_groups);
    assert_eq!(groups(&result.by_router), expected.router_groups);
}

fn assert_result_totals(result: &ShadowEvalResult, expected: &ResultsExpectation) {
    assert_eq!(
        result.overall_shadow_win_rate_pct.to_bits(),
        expected.overall_shadow_win.to_bits()
    );
    assert_eq!(
        result.overall_tie_rate_pct.to_bits(),
        expected.overall_tie.to_bits()
    );
    assert_eq!(result.sampled_real_spend, expected.real_spend);
    assert_eq!(result.sampled_shadow_spend, expected.shadow_spend);
    let funnel = expected.funnel;
    assert_eq!(result.not_sampled_count, funnel.map(|counts| counts.0));
    assert_eq!(result.unjudgeable_count, funnel.map(|counts| counts.1));
    assert_eq!(result.shed_count, funnel.map(|counts| counts.2));
    assert_eq!(result.withheld_count, funnel.map(|counts| counts.3));
}

fn assert_result_verdicts(verdicts: &TargetVerdicts, expected: &ResultsExpectation) {
    for (kind, id, group) in &expected.verdicts {
        let key = TargetKey {
            target_type: *kind,
            target_id: (*id).to_owned(),
        };
        let verdict = verdicts.get(&key).unwrap();
        assert_eq!(&verdict.group, group);
    }
}

fn groups(rows: &[ShadowEvalSlice]) -> Vec<&str> {
    rows.iter().map(|row| row.group.as_str()).collect()
}

fn assert_labels(case: &LabelsCase) {
    let labeled = label_responses(&case.responses, &case.keys, &case.teams, &case.users);
    let seen: Vec<(ShadowEvalTargetType, String, Option<String>, Option<String>)> = labeled
        .iter()
        .flat_map(|response| response.targets.iter())
        .map(|target| {
            (
                target.target_type,
                target.target_id.clone(),
                target.target_alias.clone(),
                target.key_name.clone(),
            )
        })
        .collect();
    let expected: Vec<_> = case
        .expected
        .iter()
        .map(|(kind, id, alias, key_name)| {
            (
                *kind,
                (*id).to_owned(),
                alias.map(str::to_owned),
                key_name.map(str::to_owned),
            )
        })
        .collect();
    assert_eq!(seen, expected);
}

fn assert_latest_error(case: &LatestErrorCase) {
    assert_eq!(latest_error(&case.attempts).as_deref(), case.expected);
}

fn assert_detail(case: &DetailCase) {
    let aggregates = JobAggregates {
        by_tier: &case.by_tier,
        by_model: &[],
        by_leg: &case.by_leg,
        by_router: &[],
        funnel: None,
    };
    let results = stratified_results(&case.legs, &aggregates).unwrap();
    let response = group_response("job-1", &case.legs, &LegCounts::new()).unwrap();
    let labeled = label_responses(&[response], &[], &[], &[]);
    let detail = detail_response(
        labeled.first().unwrap(),
        case.totals,
        case.last_error.map(str::to_owned),
        &results,
    )
    .unwrap();
    assert_detail_totals(&detail, &case.expected);
    assert_detail_verdicts(&detail, &case.expected);
}

fn assert_detail_totals(detail: &ShadowJob, expected: &DetailExpectation) {
    assert_eq!(detail.judged_count, Some(expected.judged_count));
    assert_eq!(detail.error_count, Some(expected.error_count));
    assert_eq!(detail.judge_spend, Some(expected.judge_spend));
    assert_eq!(detail.last_error.as_deref(), expected.last_error);
    assert_eq!(
        detail
            .results
            .as_ref()
            .map(|results| results.overall_shadow_win_rate_pct),
        Some(expected.overall_shadow_win)
    );
}

fn assert_detail_verdicts(detail: &ShadowJob, expected: &DetailExpectation) {
    let verdicts: Vec<(ShadowEvalTargetType, String, Option<String>)> = detail
        .targets
        .iter()
        .map(|target| {
            (
                target.target_type,
                target.target_id.clone(),
                target.verdicts.as_ref().map(|row| row.group.clone()),
            )
        })
        .collect();
    let wanted: Vec<_> = expected
        .verdicts
        .iter()
        .map(|(kind, id, group)| (*kind, (*id).to_owned(), group.map(str::to_owned)))
        .collect();
    assert_eq!(verdicts, wanted);
}

enum Boundary {
    Percent {
        numerator: i64,
        denominator: i64,
        expected: f64,
    },
    Round {
        value: f64,
        places: u32,
        expected: Decimal,
    },
    RoundUnrepresentable {
        value: f64,
    },
    SliceUnrepresentable {
        row: AttemptAggRow,
        name: &'static str,
    },
    NoLegs,
    UnknownLeg {
        legs: Vec<LegRow>,
        by_tier: Vec<AttemptAggRow>,
        by_leg: Vec<AttemptAggRow>,
    },
    Coverage {
        legs: Vec<LegRow>,
        reported: i64,
        expected: Option<i64>,
    },
    UnknownTarget {
        raw: &'static str,
    },
    ErrorTie {
        attempts: Vec<ErrorAttempt>,
        expected: Option<&'static str>,
    },
    TargetIds {
        responses: Vec<ShadowJob>,
        target_type: ShadowEvalTargetType,
        expected: Vec<&'static str>,
    },
}

#[rstest]
#[case::percent_one_third_rounds_to_one_decimal(Boundary::Percent { numerator: 1, denominator: 3, expected: 33.3_f64 })]
#[case::percent_two_thirds_rounds_up(Boundary::Percent { numerator: 2, denominator: 3, expected: 66.7_f64 })]
#[case::percent_five_per_mille_rounds_up(Boundary::Percent { numerator: 1, denominator: 2000, expected: 0.1_f64 })]
#[case::percent_just_under_one_hundred_rounds_to_it(Boundary::Percent { numerator: 1999, denominator: 2000, expected: 100.0_f64 })]
#[case::percent_zero_denominator_reads_zero(Boundary::Percent { numerator: 7, denominator: 0, expected: 0.0_f64 })]
#[case::percent_negative_denominator_reads_zero(Boundary::Percent { numerator: 7, denominator: -1, expected: 0.0_f64 })]
#[case::percent_zero_numerator_reads_zero(Boundary::Percent { numerator: 0, denominator: 7, expected: 0.0_f64 })]
#[case::round_six_places_keeps_the_representable_below(Boundary::Round { value: 0.123_456_5_f64, places: 6, expected: spelled("0.123456") })]
#[case::round_three_places_at_a_true_half(Boundary::Round { value: 0.000_5_f64, places: 3, expected: spelled("0.001") })]
#[case::round_one_decimal_keeps_the_representable_below(Boundary::Round { value: 0.15_f64, places: 1, expected: spelled("0.1") })]
#[case::round_one_decimal_ties_to_even(Boundary::Round { value: 6.25_f64, places: 1, expected: spelled("6.2") })]
#[case::round_zero_places_ties_to_even(Boundary::Round { value: 2.5_f64, places: 0, expected: spelled("2.0") })]
#[case::round_zero_places_ties_up_to_even(Boundary::Round { value: 3.5_f64, places: 0, expected: spelled("4.0") })]
#[case::round_six_places_carries(Boundary::Round { value: 0.999_999_5_f64, places: 6, expected: spelled("1.0") })]
#[case::round_nan_is_unrepresentable(Boundary::RoundUnrepresentable { value: f64::NAN })]
#[case::slice_nan_confidence_is_unrepresentable(Boundary::SliceUnrepresentable {
    row: agg("tier", 1, (1, 0, 0, 0), Some(f64::NAN), (0.5_f64, 0.0_f64)),
    name: "avg_judge_confidence",
})]
#[case::group_response_needs_a_leg(Boundary::NoLegs)]
#[case::verdicts_name_an_attempt_for_an_unknown_leg(Boundary::UnknownLeg {
    legs: vec![leg("leg-key", Key, "k1")],
    by_tier: vec![agg("simple", 1, (1, 0, 0, 0), Some(0.5_f64), (0.0_f64, 0.0_f64))],
    by_leg: vec![agg("ghost", 1, (1, 0, 0, 0), Some(0.5_f64), (0.0_f64, 0.0_f64))],
})]
#[case::funnel_with_one_leg_missing_is_unknown(Boundary::Coverage {
    legs: vec![leg("leg-key", Key, "k1"), leg("leg-team", Team, "t1")],
    reported: 1,
    expected: None,
})]
#[case::funnel_with_every_leg_is_known(Boundary::Coverage {
    legs: vec![leg("leg-key", Key, "k1"), leg("leg-team", Team, "t1")],
    reported: 2,
    expected: Some(4),
})]
#[case::funnel_with_more_rows_than_legs_is_unknown(Boundary::Coverage {
    legs: vec![leg("leg-key", Key, "k1"), leg("leg-team", Team, "t1")],
    reported: 3,
    expected: None,
})]
#[case::an_unknown_target_kind_names_itself(Boundary::UnknownTarget { raw: "org" })]
#[case::latest_error_ties_keep_the_first_attempt(Boundary::ErrorTie {
    attempts: vec![error_attempt(9, Some("first")), error_attempt(9, Some("second"))],
    expected: Some("first"),
})]
#[case::target_ids_are_sorted_and_unique(Boundary::TargetIds {
    responses: vec![job(vec![
        target_leg(Key, "b"),
        target_leg(Team, "a"),
        target_leg(Key, "a"),
        target_leg(Key, "b"),
    ])],
    target_type: Key,
    expected: vec!["a", "b"],
})]
#[case::target_ids_of_an_unused_kind_are_empty(Boundary::TargetIds {
    responses: vec![job(vec![target_leg(Key, "a")])],
    target_type: User,
    expected: Vec::new(),
})]
fn shadow_result_aggregation_boundaries(#[case] boundary: Boundary) {
    match boundary {
        Boundary::Percent {
            numerator,
            denominator,
            expected,
        } => assert_percent(numerator, denominator, expected),
        Boundary::Round {
            value,
            places,
            expected,
        } => assert_eq!(round_to(value, places, "rate").unwrap(), expected),
        Boundary::RoundUnrepresentable { value } => {
            assert_unrepresentable(round_to(value, 1, "rate"), "rate");
        }
        Boundary::SliceUnrepresentable { row, name } => {
            assert_unrepresentable(slices(&[row]), name);
        }
        Boundary::NoLegs => assert_no_legs(),
        Boundary::UnknownLeg {
            legs,
            by_tier,
            by_leg,
        } => assert_unknown_leg(&legs, &by_tier, &by_leg),
        Boundary::Coverage {
            legs,
            reported,
            expected,
        } => assert_coverage(&legs, reported, expected),
        Boundary::UnknownTarget { raw } => assert_unknown_target(raw),
        Boundary::ErrorTie { attempts, expected } => {
            assert_eq!(latest_error(&attempts).as_deref(), expected);
        }
        Boundary::TargetIds {
            responses,
            target_type,
            expected,
        } => assert_target_ids(&responses, target_type, &expected),
    }
}

fn assert_percent(numerator: i64, denominator: i64, expected: f64) {
    let rate = pct_of(numerator, denominator).unwrap();
    assert_eq!(rate.to_bits(), expected.to_bits());
}

fn assert_unrepresentable<Value>(outcome: Result<Value, ShadowResultsError>, name: &str)
where
    Value: Debug,
{
    match outcome {
        Err(ShadowResultsError::Unrepresentable { name: found }) => assert_eq!(found, name),
        other => panic!("{other:?}"),
    }
}

fn assert_no_legs() {
    match group_response("job-1", &[], &LegCounts::new()) {
        Err(ShadowResultsError::NoLegs) => {}
        other => panic!("{other:?}"),
    }
}

fn assert_unknown_leg(legs: &[LegRow], by_tier: &[AttemptAggRow], by_leg: &[AttemptAggRow]) {
    let aggregates = JobAggregates {
        by_tier,
        by_model: &[],
        by_leg,
        by_router: &[],
        funnel: None,
    };
    match stratified_results(legs, &aggregates) {
        Err(ShadowResultsError::UnknownLeg { leg_id }) => assert_eq!(leg_id, "ghost"),
        other => panic!("{other:?}"),
    }
}

fn assert_coverage(legs: &[LegRow], reported: i64, expected: Option<i64>) {
    let by_tier = vec![agg(
        "simple",
        1,
        (1, 0, 0, 0),
        Some(0.5_f64),
        (0.0_f64, 0.0_f64),
    )];
    let aggregates = JobAggregates {
        by_tier: &by_tier,
        by_model: &[],
        by_leg: &[],
        by_router: &[],
        funnel: Some(FunnelTotals {
            legs_with_rows: reported,
            not_sampled: 4,
            unjudgeable: 3,
            shed: 2,
            withheld: 1,
        }),
    };
    let results: JobResults = stratified_results(legs, &aggregates).unwrap();
    let result = results.results.unwrap();
    assert_eq!(result.not_sampled_count, expected);
    assert_eq!(result.unjudgeable_count, expected.map(|_| 3));
    assert_eq!(result.shed_count, expected.map(|_| 2));
    assert_eq!(result.withheld_count, expected.map(|_| 1));
}

fn assert_unknown_target(raw: &str) {
    match parse_target_type(raw) {
        Err(ShadowResultsError::UnknownTargetType { target_type }) => {
            assert_eq!(target_type, raw);
        }
        other => panic!("{other:?}"),
    }
}

fn assert_target_ids(
    responses: &[ShadowJob],
    target_type: ShadowEvalTargetType,
    expected: &[&str],
) {
    let wanted: Vec<String> = expected.iter().map(|id| (*id).to_owned()).collect();
    assert_eq!(target_ids_of(responses, target_type), wanted);
}
