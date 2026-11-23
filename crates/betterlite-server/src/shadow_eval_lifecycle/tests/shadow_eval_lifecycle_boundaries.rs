use chrono::Duration;
use rstest::rstest;
use rust_decimal::Decimal;

use super::support::{
    ALL_TARGETS, FakeAttempt, FakeStore, KEY_ONLY, error_of, identity, leg, minted, now, request,
    start, targets,
};
use crate::shadow_eval_job_loading::jobs::ShadowEvalDirection as JobDirection;
use crate::shadow_eval_lifecycle::access::require_writer;
use crate::shadow_eval_lifecycle::body::{
    SHADOW_BODY_MAX_BYTES, ShadowBodyError, decode_shadow_body,
};
use crate::shadow_eval_lifecycle::error::{DB_NOT_CONNECTED, LifecycleError};
use crate::shadow_eval_lifecycle::stop::stop_shadow_eval_job;
use crate::shadow_eval_models::ShadowEvalTargetType;

const FORBIDDEN_WRITER: &str = "only a proxy admin can start a shadow eval";

#[rstest]
#[case(Some("proxy_admin"), None)]
#[case(Some("proxy_admin_viewer"), Some(FORBIDDEN_WRITER))]
#[case(Some("internal_user"), Some(FORBIDDEN_WRITER))]
#[case(Some("org_admin"), Some(FORBIDDEN_WRITER))]
#[case(None, Some(FORBIDDEN_WRITER))]
#[case(Some("not-a-role"), Some(FORBIDDEN_WRITER))]
fn only_a_proxy_admin_may_start(#[case] role: Option<&str>, #[case] expected: Option<&str>) {
    let store = FakeStore::new(now());
    let chosen = require_writer(&identity(role), "start a shadow eval", Some(&store));
    match expected {
        Some(message) => {
            let error = match chosen {
                Ok(_store) => panic!("expected a refusal"),
                Err(error) => error,
            };
            assert_eq!(error.to_string(), message);
            assert_eq!(error.status_code(), 403);
        }
        None => {
            drop(chosen.unwrap());
        }
    }
}

#[tokio::test]
async fn a_missing_database_answers_500_only_after_the_writer_check() {
    let missing =
        require_writer::<FakeStore>(&identity(Some("proxy_admin")), "stop a shadow eval", None)
            .err()
            .unwrap();
    assert_eq!(missing.to_string(), DB_NOT_CONNECTED);
    assert_eq!(missing.status_code(), 500);
    let forbidden = require_writer::<FakeStore>(
        &identity(Some("proxy_admin_viewer")),
        "stop a shadow eval",
        None,
    )
    .err()
    .unwrap();
    assert_eq!(
        forbidden.to_string(),
        "only a proxy admin can stop a shadow eval"
    );
    assert_eq!(forbidden.status_code(), 403);
}

fn padded(length: usize) -> Vec<u8> {
    let mut body = String::from(KEY_ONLY).into_bytes();
    body.resize(length, b' ');
    body
}

#[rstest]
#[case(SHADOW_BODY_MAX_BYTES, true)]
#[case(SHADOW_BODY_MAX_BYTES + 1, false)]
fn the_shadow_body_bound_is_exactly_one_mebibyte(#[case] length: usize, #[case] accepted: bool) {
    let decoded = decode_shadow_body(&padded(length));
    match accepted {
        true => {
            let parsed = decoded.unwrap();
            assert_eq!(parsed.api_key_ids, vec!["key-1".to_owned()]);
        }
        false => {
            let ShadowBodyError::TooLarge { bytes, limit } = decoded.err().unwrap() else {
                panic!("expected the size gate to answer first");
            };
            assert_eq!(bytes, SHADOW_BODY_MAX_BYTES + 1);
            assert_eq!(limit, SHADOW_BODY_MAX_BYTES);
        }
    }
}

#[test]
fn a_parser_error_wrapped_for_the_route_answers_400() {
    let error = decode_shadow_body(b"{").err().unwrap();
    let wrapped = LifecycleError::from(error);
    assert_eq!(wrapped.status_code(), 400);
}

#[rstest]
#[case(1_u32, 1_i64)]
#[case(30_u32, 30_i64)]
#[tokio::test]
async fn a_jobs_window_runs_the_whole_declared_duration(#[case] days: u32, #[case] span: i64) {
    let store = FakeStore::new(now()).with_targets(targets());
    let body = format!(
        r#"{{"router_names": ["shadow-router"], "api_key_ids": ["key-1"], "judge_model": "judge-model", "shadow_percentage": 10, "max_budget": 10, "duration_days": {days}}}"#
    );
    let response = start(&store, &request(&body), None, now(), &mut minted(2))
        .await
        .unwrap();
    assert_eq!(response.job.ends_at, now() + Duration::days(span));
}

#[tokio::test]
async fn a_leg_whose_window_closed_this_instant_cannot_be_stopped() {
    let closed = leg(
        "job-1",
        ShadowEvalTargetType::Key,
        "key-1",
        JobDirection::Forward,
        now() - Duration::days(1),
        now(),
    );
    let store = FakeStore::new(now())
        .with_targets(targets())
        .with_legs(vec![closed]);
    let error = error_of(stop_shadow_eval_job(&store, "job-1", None, now()).await);
    assert_eq!(error.to_string(), "Job job-1 is already completed");
    assert_eq!(error.status_code(), 400);
}

#[tokio::test]
async fn a_leg_one_second_inside_its_window_can_be_stopped() {
    let open = leg(
        "job-1",
        ShadowEvalTargetType::Key,
        "key-1",
        JobDirection::Forward,
        now() - Duration::days(1),
        now() + Duration::seconds(1),
    );
    let store = FakeStore::new(now())
        .with_targets(targets())
        .with_legs(vec![open]);
    let response = stop_shadow_eval_job(&store, "job-1", None, now())
        .await
        .unwrap();
    assert_eq!(response.job.stopped_by.as_deref(), Some("operator"));
}

#[rstest]
#[case(Decimal::new(1000, 2), "Job job-1 is already completed")]
#[case(Decimal::new(999, 2), "")]
#[tokio::test]
async fn a_leg_that_reached_its_budget_exactly_reads_completed(
    #[case] spend: Decimal,
    #[case] expected: &str,
) {
    let running = leg(
        "job-1",
        ShadowEvalTargetType::Key,
        "key-1",
        JobDirection::Forward,
        now() - Duration::days(1),
        now() + Duration::days(6),
    );
    let attempts = vec![FakeAttempt {
        job_id: running.id.clone(),
        created_at: now() - Duration::hours(1),
        spend,
    }];
    let store = FakeStore::new(now())
        .with_targets(targets())
        .with_legs(vec![running])
        .with_attempts(attempts);
    let outcome = stop_shadow_eval_job(&store, "job-1", None, now()).await;
    match expected {
        "" => {
            assert_eq!(outcome.unwrap().job.stopped_by.as_deref(), Some("operator"));
        }
        message => {
            assert_eq!(error_of(outcome).to_string(), message);
        }
    }
}

#[rstest]
#[case(2_i64, "Job job-1 is already completed")]
#[case(1_i64, "")]
#[tokio::test]
async fn a_leg_that_reached_its_turn_valve_exactly_reads_completed(
    #[case] attempts_made: i64,
    #[case] expected: &str,
) {
    let mut running = leg(
        "job-1",
        ShadowEvalTargetType::Key,
        "key-1",
        JobDirection::Forward,
        now() - Duration::days(1),
        now() + Duration::days(6),
    );
    running.max_turns = 2;
    let attempts: Vec<FakeAttempt> = (0..attempts_made)
        .map(|hour| FakeAttempt {
            job_id: running.id.clone(),
            created_at: now() - Duration::hours(hour + 1),
            spend: Decimal::ZERO,
        })
        .collect();
    let store = FakeStore::new(now())
        .with_targets(targets())
        .with_legs(vec![running])
        .with_attempts(attempts);
    let outcome = stop_shadow_eval_job(&store, "job-1", None, now()).await;
    match expected {
        "" => {
            assert_eq!(outcome.unwrap().job.stopped_by.as_deref(), Some("operator"));
        }
        message => {
            assert_eq!(error_of(outcome).to_string(), message);
        }
    }
}

#[tokio::test]
async fn a_stamp_freezes_the_attempts_counted_against_it() {
    let stamp = now() - Duration::hours(1);
    let mut spent = leg(
        "job-1",
        ShadowEvalTargetType::Key,
        "key-1",
        JobDirection::Forward,
        now() - Duration::days(1),
        now() + Duration::days(6),
    );
    spent.stopped_at = Some(stamp);
    let active = leg(
        "job-1",
        ShadowEvalTargetType::Team,
        "team-1",
        JobDirection::Forward,
        now() - Duration::days(1),
        now() + Duration::days(6),
    );
    let attempts = vec![
        FakeAttempt {
            job_id: spent.id.clone(),
            created_at: stamp - Duration::seconds(1),
            spend: Decimal::new(50, 2),
        },
        FakeAttempt {
            job_id: spent.id.clone(),
            created_at: stamp + Duration::seconds(1),
            spend: Decimal::new(500, 2),
        },
    ];
    let store = FakeStore::new(now())
        .with_targets(targets())
        .with_legs(vec![spent, active])
        .with_attempts(attempts);
    let response = stop_shadow_eval_job(&store, "job-1", None, now())
        .await
        .unwrap();
    assert_eq!(response.job.targets[0].attempt_count, Some(1));
    assert_eq!(response.job.targets[0].spend, Some(Decimal::new(50, 2)));
}

#[rstest]
#[case(ShadowEvalTargetType::Team, "key-1", KEY_ONLY)]
#[case(ShadowEvalTargetType::Key, "team-1", r#"{"router_names": ["shadow-router"], "team_ids": ["team-1"], "judge_model": "judge-model", "shadow_percentage": 10, "max_budget": 10}"#)]
#[tokio::test]
async fn a_slot_matches_only_its_own_target_kind(
    #[case] kind: ShadowEvalTargetType,
    #[case] target_id: &str,
    #[case] body: &str,
) {
    let decoy = leg(
        "decoy-job",
        kind,
        target_id,
        JobDirection::Forward,
        now(),
        now() + Duration::days(1),
    );
    let store = FakeStore::new(now())
        .with_targets(targets())
        .with_legs(vec![decoy]);
    let response = start(&store, &request(body), None, now(), &mut minted(2))
        .await
        .unwrap();
    assert_ne!(response.job.job_id, "decoy-job");
}

#[tokio::test]
async fn a_start_with_no_ids_left_answers_500_before_any_write() {
    let store = FakeStore::new(now()).with_targets(targets());
    let error = error_of(start(&store, &request(ALL_TARGETS), None, now(), &mut minted(0)).await);
    assert_eq!(
        error.to_string(),
        "the shadow eval job id source is exhausted"
    );
    assert_eq!(error.status_code(), 500);
    assert!(store.stored_legs().is_empty());
}

#[tokio::test]
async fn a_start_short_of_one_leg_id_writes_nothing() {
    let store = FakeStore::new(now()).with_targets(targets());
    let mut ids = minted(2);
    let error = error_of(start(&store, &request(ALL_TARGETS), None, now(), &mut ids).await);
    assert_eq!(
        error.to_string(),
        "the shadow eval job id source is exhausted"
    );
    assert_eq!(ids.next(), None);
    assert!(store.stored_legs().is_empty());
    assert!(
        !store
            .recorded()
            .iter()
            .any(|call| call.starts_with("create"))
    );
}
