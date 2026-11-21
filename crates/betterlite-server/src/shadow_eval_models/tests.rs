#![cfg(test)]

use std::collections::BTreeSet;

use chrono::{DateTime, Utc};
use rstest::rstest;
use rust_decimal::Decimal;

use super::job::ShadowJob;
use super::leg::ShadowLeg;
use super::{ShadowEvalModelError, ShadowEvalStatus, ShadowEvalTargetType};
use crate::shadow_eval_job_loading::jobs::ShadowEvalDirection;

const NO_TARGETS: &str = "a shadow eval job has at least one target";
const NO_ROUTER_ARMS: &str = "a shadow eval job has at least one router name";

fn at(text: &str) -> DateTime<Utc> {
    text.parse().unwrap()
}

fn clock() -> DateTime<Utc> {
    at("2026-11-21T12:00:00Z")
}

fn leg(target_id: &str) -> ShadowLeg {
    ShadowLeg {
        target_type: ShadowEvalTargetType::Key,
        target_id: target_id.to_owned(),
        max_turns: 10,
        max_budget: None,
        stopped_at: None,
        attempt_count: None,
        spend: None,
        verdicts: None,
        target_alias: Some("prod-key".to_owned()),
        key_name: Some("sk-...abcd".to_owned()),
    }
}

fn budget_leg(target_id: &str, budget: i64, spend: i64) -> ShadowLeg {
    ShadowLeg {
        max_budget: Some(Decimal::from(budget)),
        spend: Some(Decimal::from(spend)),
        ..leg(target_id)
    }
}

fn turn_leg(target_id: &str, max_turns: i64, attempts: i64) -> ShadowLeg {
    ShadowLeg {
        max_turns,
        attempt_count: Some(attempts),
        ..leg(target_id)
    }
}

fn stopped_leg(target_id: &str) -> ShadowLeg {
    ShadowLeg {
        stopped_at: Some(clock()),
        ..leg(target_id)
    }
}

fn job(targets: Vec<ShadowLeg>) -> ShadowJob {
    ShadowJob {
        job_id: "job-1".to_owned(),
        targets,
        router_names: vec!["router-a".to_owned(), "router-b".to_owned()],
        models: Vec::new(),
        direction: ShadowEvalDirection::Forward,
        baseline_model: None,
        judge_model: "anthropic/claude-sonnet-5".to_owned(),
        shadow_percentage: 10.0,
        created_at: clock(),
        ends_at: at("2026-11-28T12:00:00Z"),
        stopped_by: None,
        judged_count: None,
        error_count: None,
        judge_spend: None,
        last_error: None,
        results: None,
    }
}

#[rstest]
#[case::open_target_runs(job(vec![leg("hashed-key-1")]), ShadowEvalStatus::Running)]
#[case::one_target_out_of_budget_keeps_the_job_running(
    job(vec![budget_leg("hashed-key-1", 2, 2), leg("hashed-key-2")]),
    ShadowEvalStatus::Running
)]
#[case::every_target_out_of_budget_completes(
    job(vec![budget_leg("hashed-key-1", 2, 2), budget_leg("hashed-key-2", 5, 6)]),
    ShadowEvalStatus::Completed
)]
#[case::every_target_turn_valve_completes(
    job(vec![turn_leg("hashed-key-1", 10, 10), turn_leg("hashed-key-2", 10, 12)]),
    ShadowEvalStatus::Completed
)]
#[case::the_window_end_completes(
    ShadowJob { ends_at: at("2026-11-20T12:00:00Z"), ..job(vec![leg("hashed-key-1")]) },
    ShadowEvalStatus::Completed
)]
#[case::an_operator_stop_is_stopped(
    ShadowJob { stopped_by: Some("operator-1".to_owned()), ..job(vec![leg("hashed-key-1")]) },
    ShadowEvalStatus::Stopped
)]
#[case::every_target_stamped_is_stopped(
    job(vec![stopped_leg("hashed-key-1"), stopped_leg("hashed-key-2")]),
    ShadowEvalStatus::Stopped
)]
fn shadow_eval_models_behaviors(#[case] job: ShadowJob, #[case] expected: ShadowEvalStatus) {
    let response = job.response(clock()).unwrap();
    assert_eq!(response.status(), expected);
    assert_eq!(response.router_name(), "router-a");
    assert_eq!(response.job(), &job);
    let wire = serde_json::to_value(&response).unwrap();
    assert_eq!(wire["job_id"], "job-1");
    assert_eq!(wire["router_name"], "router-a");
    assert_eq!(wire["status"], serde_json::to_value(expected).unwrap());
    assert!(wire["results"].is_null());
    assert!(wire["judge_spend"].is_null());
    let targets = wire["targets"].as_array().unwrap();
    assert_eq!(targets.len(), job.targets.len());
    assert_eq!(targets[0]["target_id"], job.targets[0].target_id);
    assert_eq!(targets[0]["key_name"], "sk-...abcd");
    assert_eq!(targets[0]["target_alias"], "prod-key");
    assert!(targets[0]["verdicts"].is_null());
    let target_keys: BTreeSet<&str> = targets[0]
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        target_keys,
        BTreeSet::from([
            "attempt_count",
            "key_name",
            "max_budget",
            "max_turns",
            "spend",
            "stopped_at",
            "target_alias",
            "target_id",
            "target_type",
            "verdicts",
        ])
    );
}

#[rstest]
#[case::spend_at_the_budget(job(vec![budget_leg("k1", 2, 2)]), clock(), Ok(ShadowEvalStatus::Completed), vec![true])]
#[case::spend_below_the_budget(job(vec![budget_leg("k1", 2, 1)]), clock(), Ok(ShadowEvalStatus::Running), vec![false])]
#[case::spend_above_the_budget(job(vec![budget_leg("k1", 2, 3)]), clock(), Ok(ShadowEvalStatus::Completed), vec![true])]
#[case::budget_with_no_spend(
    job(vec![ShadowLeg { max_budget: Some(Decimal::from(2_i64)), ..leg("k1") }]),
    clock(),
    Ok(ShadowEvalStatus::Running),
    vec![false]
)]
#[case::spend_with_no_budget(
    job(vec![ShadowLeg { spend: Some(Decimal::from(2_i64)), ..leg("k1") }]),
    clock(),
    Ok(ShadowEvalStatus::Running),
    vec![false]
)]
#[case::attempts_at_the_valve(job(vec![turn_leg("k1", 4, 4)]), clock(), Ok(ShadowEvalStatus::Completed), vec![true])]
#[case::attempts_below_the_valve(job(vec![turn_leg("k1", 4, 3)]), clock(), Ok(ShadowEvalStatus::Running), vec![false])]
#[case::attempts_above_the_valve(job(vec![turn_leg("k1", 4, 5)]), clock(), Ok(ShadowEvalStatus::Completed), vec![true])]
#[case::a_zero_valve_at_zero_attempts(job(vec![turn_leg("k1", 0, 0)]), clock(), Ok(ShadowEvalStatus::Completed), vec![true])]
#[case::attempts_missing(
    job(vec![ShadowLeg { max_turns: 4, ..leg("k1") }]),
    clock(),
    Ok(ShadowEvalStatus::Running),
    vec![false]
)]
#[case::now_at_the_window_end(
    ShadowJob { ends_at: clock(), ..job(vec![leg("k1")]) },
    clock(),
    Ok(ShadowEvalStatus::Completed),
    vec![false]
)]
#[case::now_before_the_window_end(
    ShadowJob { ends_at: at("2026-11-21T12:00:01Z"), ..job(vec![leg("k1")]) },
    clock(),
    Ok(ShadowEvalStatus::Running),
    vec![false]
)]
#[case::stopped_by_outranks_the_window(
    ShadowJob {
        stopped_by: Some("operator-1".to_owned()),
        ends_at: at("2026-11-20T12:00:00Z"),
        ..job(vec![budget_leg("k1", 2, 2)])
    },
    clock(),
    Ok(ShadowEvalStatus::Stopped),
    vec![true]
)]
#[case::every_target_stamped_stops(job(vec![stopped_leg("k1"), stopped_leg("k2")]), clock(), Ok(ShadowEvalStatus::Stopped), vec![false, false])]
#[case::one_target_stamped_runs(job(vec![stopped_leg("k1"), leg("k2")]), clock(), Ok(ShadowEvalStatus::Running), vec![false, false])]
#[case::a_spent_budget_outranks_a_stamp(
    job(vec![ShadowLeg {
        stopped_at: Some(clock()),
        max_budget: Some(Decimal::from(2_i64)),
        spend: Some(Decimal::from(2_i64)),
        ..leg("k1")
    }]),
    clock(),
    Ok(ShadowEvalStatus::Completed),
    vec![true]
)]
#[case::no_targets(job(Vec::new()), clock(), Err((ShadowEvalModelError::NoTargets, NO_TARGETS)), vec![])]
#[case::no_router_arms(
    ShadowJob { router_names: Vec::new(), ..job(vec![leg("k1")]) },
    clock(),
    Err((ShadowEvalModelError::NoRouterArms, NO_ROUTER_ARMS)),
    vec![false]
)]
fn shadow_eval_models_boundaries(
    #[case] job: ShadowJob,
    #[case] now: DateTime<Utc>,
    #[case] expected: Result<ShadowEvalStatus, (ShadowEvalModelError, &str)>,
    #[case] spent: Vec<bool>,
) {
    let budget_spent = job
        .targets
        .iter()
        .map(ShadowLeg::budget_spent)
        .collect::<Vec<_>>();
    assert_eq!(budget_spent, spent);
    match (job.response(now), expected) {
        (Ok(response), Ok(status)) => {
            assert_eq!(response.status(), status);
            assert_eq!(response.router_name(), "router-a");
            assert_eq!(response.job(), &job);
        }
        (Err(error), Err((wanted, message))) => {
            assert_eq!(error, wanted);
            assert_eq!(error.to_string(), message);
        }
        (Ok(response), Err((wanted, _))) => {
            panic!("expected {wanted:?}, got {:?}", response.status())
        }
        (Err(error), Ok(status)) => panic!("expected {status:?}, got {error}"),
    }
}
