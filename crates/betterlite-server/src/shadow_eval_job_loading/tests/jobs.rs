#![cfg(test)]

use rust_decimal::Decimal;

use super::super::jobs::{ActiveShadowEvalJob, ShadowEvalDirection, ShadowJobError};
use super::support::job_row;

#[test]
fn shadow_eval_job_validation_accepts_a_forward_job_without_a_baseline() {
    let mut row = job_row("forward", None);
    row.models = vec!["gpt-4o".to_owned()];
    let job = ActiveShadowEvalJob::from_row(&row, 2, Decimal::new(5, 1)).unwrap();
    assert_eq!(job.id, "job-1");
    assert_eq!(job.direction, ShadowEvalDirection::Forward);
    assert_eq!(job.baseline_model, None);
    assert_eq!(job.arm_router_names, vec!["router-a"]);
    assert_eq!(job.models, ["gpt-4o".to_owned()].into());
    assert_eq!(job.attempts, 2);
    assert_eq!(job.spend, Decimal::new(5, 1));
    assert_eq!(job.arm_target("router-a"), "router-a");
}

#[test]
fn shadow_eval_job_validation_accepts_a_reverse_job_with_one_router() {
    let mut row = job_row("reverse", Some("baseline-model"));
    row.router_names = vec!["router-b".to_owned()];
    let job = ActiveShadowEvalJob::from_row(&row, 0, Decimal::ZERO).unwrap();
    assert_eq!(job.direction, ShadowEvalDirection::Reverse);
    assert_eq!(job.baseline_model, Some("baseline-model".to_owned()));
    assert_eq!(job.arm_router_names, vec!["router-b"]);
    assert_eq!(job.arm_target("router-b"), "baseline-model");
}

#[test]
fn shadow_eval_job_validation_accepts_a_forward_job_with_several_arms() {
    let mut row = job_row("forward", None);
    row.router_names = vec!["router-b".to_owned(), "router-c".to_owned()];
    let job = ActiveShadowEvalJob::from_row(&row, 0, Decimal::ZERO).unwrap();
    assert_eq!(job.arm_router_names, vec!["router-b", "router-c"]);
    assert_eq!(job.arm_target("router-c"), "router-c");
}

#[test]
fn shadow_eval_job_validation_rejects_an_unknown_direction() {
    let error =
        ActiveShadowEvalJob::from_row(&job_row("sideways", None), 0, Decimal::ZERO).unwrap_err();
    match error {
        ShadowJobError::UnknownDirection { direction } => assert_eq!(direction, "sideways"),
        other => panic!("unexpected validation error: {other}"),
    }
}

#[test]
fn shadow_eval_job_validation_rejects_a_baseline_on_a_forward_job() {
    let error = ActiveShadowEvalJob::from_row(
        &job_row("forward", Some("baseline-model")),
        0,
        Decimal::ZERO,
    )
    .unwrap_err();
    assert!(matches!(error, ShadowJobError::BaselineModelMismatch));
}

#[test]
fn shadow_eval_job_validation_rejects_a_reverse_job_without_a_baseline() {
    let error =
        ActiveShadowEvalJob::from_row(&job_row("reverse", None), 0, Decimal::ZERO).unwrap_err();
    assert!(matches!(error, ShadowJobError::BaselineModelMismatch));
}

#[test]
fn shadow_eval_job_validation_rejects_a_reverse_job_with_two_routers() {
    let mut row = job_row("reverse", Some("baseline-model"));
    row.router_names = vec!["router-b".to_owned(), "router-c".to_owned()];
    let error = ActiveShadowEvalJob::from_row(&row, 0, Decimal::ZERO).unwrap_err();
    assert!(matches!(error, ShadowJobError::ReverseEvaluatesOneRouter));
}

#[test]
fn shadow_eval_job_validation_round_trips_the_direction_names() {
    assert_eq!(
        ShadowEvalDirection::parse("forward").unwrap().as_str(),
        "forward"
    );
    assert_eq!(
        ShadowEvalDirection::parse("reverse").unwrap().as_str(),
        "reverse"
    );
    let error = ShadowEvalDirection::parse("Forward").unwrap_err();
    assert!(
        matches!(error, ShadowJobError::UnknownDirection { direction } if direction == "Forward")
    );
}
