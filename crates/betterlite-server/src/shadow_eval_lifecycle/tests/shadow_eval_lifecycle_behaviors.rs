use chrono::{DateTime, Duration, Utc};
use rust_decimal::Decimal;
use uuid::Uuid;

use super::support::{
    ALL_TARGETS, BASELINE, FakeAttempt, FakeStore, JUDGE, KEY_ONLY, REVERSE_KEY, ROUTER, error_of,
    leg, minted, now, request, start, targets,
};
use crate::shadow_eval_job_loading::jobs::ShadowEvalDirection as JobDirection;
use crate::shadow_eval_lifecycle::stop::stop_shadow_eval_job;
use crate::shadow_eval_models::{ShadowEvalStatus, ShadowEvalTargetType};

#[tokio::test]
async fn starts_one_job_with_a_leg_per_target_and_the_shared_config() {
    let store = FakeStore::new(now()).with_targets(targets());
    let parsed = request(ALL_TARGETS);
    let response = start(&store, &parsed, Some("admin-user"), now(), &mut minted(4))
        .await
        .unwrap();
    assert_eq!(response.status, ShadowEvalStatus::Running);
    assert_eq!(response.router_name, ROUTER);
    assert_eq!(response.job.job_id, Uuid::from_u128(1).to_string());
    assert_eq!(response.job.router_names, vec![ROUTER.to_owned()]);
    assert_eq!(response.job.direction, JobDirection::Forward);
    assert_eq!(response.job.judge_model, JUDGE);
    assert_eq!(response.job.shadow_percentage, 10.0_f64);
    assert_eq!(response.job.created_at, now());
    assert_eq!(response.job.ends_at, now() + Duration::days(7));
    assert_eq!(response.job.stopped_by, None);
    let kinds: Vec<ShadowEvalTargetType> = response
        .job
        .targets
        .iter()
        .map(|target| target.target_type)
        .collect();
    assert_eq!(
        kinds,
        vec![
            ShadowEvalTargetType::Key,
            ShadowEvalTargetType::Team,
            ShadowEvalTargetType::User
        ]
    );
    let key = &response.job.targets[0];
    assert_eq!(key.target_id, "key-1");
    assert_eq!(key.target_alias.as_deref(), Some("prod-key"));
    assert_eq!(key.key_name.as_deref(), Some("sk-...abcd"));
    assert_eq!(key.max_turns, 10_000);
    assert_eq!(key.max_budget, Some(Decimal::new(1000, 2)));
    assert_eq!(key.attempt_count, None);
    assert_eq!(key.spend, None);
    assert_eq!(key.stopped_at, None);
    let team = &response.job.targets[1];
    assert_eq!(team.target_alias.as_deref(), Some("Team One"));
    assert_eq!(team.key_name, None);
    let user = &response.job.targets[2];
    assert_eq!(user.target_alias.as_deref(), Some("user@example.com"));
    assert_eq!(
        store.recorded(),
        vec![
            "load",
            "sweep:key",
            "sweep:team",
            "sweep:user",
            "claim",
            "create:3",
            "seed:3"
        ]
    );
    let stored = store.stored_legs();
    assert_eq!(stored.len(), 3);
    assert_eq!(
        stored.iter().map(|row| row.id.as_str()).collect::<Vec<_>>(),
        vec![
            &Uuid::from_u128(2).to_string(),
            &Uuid::from_u128(3).to_string(),
            &Uuid::from_u128(4).to_string()
        ]
    );
    assert_eq!(stored[0].created_by.as_deref(), Some("admin-user"));
}

#[tokio::test]
async fn sweeps_a_finished_leg_free_before_it_claims_the_slot() {
    let mut finished = leg(
        "old-job",
        ShadowEvalTargetType::Key,
        "key-1",
        JobDirection::Forward,
        now() - Duration::days(7),
        now(),
    );
    finished.stopped_at = None;
    let store = FakeStore::new(now())
        .with_targets(targets())
        .with_legs(vec![finished]);
    let response = start(&store, &request(KEY_ONLY), None, now(), &mut minted(2))
        .await
        .unwrap();
    assert_eq!(response.status, ShadowEvalStatus::Running);
    let stored = store.stored_legs();
    assert_eq!(stored[0].group_id, "old-job");
    assert_eq!(stored[0].stopped_at, Some(now()));
    assert_eq!(stored[1].group_id, response.job.job_id);
}

#[tokio::test]
async fn refuses_a_target_claimed_in_the_same_direction() {
    let claimed = leg(
        "other-job",
        ShadowEvalTargetType::Key,
        "key-1",
        JobDirection::Forward,
        now(),
        now() + Duration::days(1),
    );
    let store = FakeStore::new(now())
        .with_targets(targets())
        .with_legs(vec![claimed]);
    let error = error_of(start(&store, &request(KEY_ONLY), None, now(), &mut minted(2)).await);
    assert_eq!(
        error.to_string(),
        "Already in an active forward shadow eval job: key key-1 (job other-job). Stop it first."
    );
    assert_eq!(error.status_code(), 409);
    assert!(
        !store
            .recorded()
            .iter()
            .any(|call| call.starts_with("create"))
    );
}

#[tokio::test]
async fn sorts_every_claimed_target_into_the_conflict_message() {
    let key = leg(
        "a-job",
        ShadowEvalTargetType::Key,
        "key-1",
        JobDirection::Forward,
        now(),
        now() + Duration::days(1),
    );
    let team = leg(
        "b-job",
        ShadowEvalTargetType::Team,
        "team-1",
        JobDirection::Forward,
        now(),
        now() + Duration::days(1),
    );
    let store = FakeStore::new(now())
        .with_targets(targets())
        .with_legs(vec![team, key]);
    let error = error_of(start(&store, &request(ALL_TARGETS), None, now(), &mut minted(4)).await);
    assert_eq!(
        error.to_string(),
        "Already in an active forward shadow eval job: key key-1 (job a-job), team team-1 (job b-job). Stop it first."
    );
}

#[tokio::test]
async fn a_reverse_job_runs_beside_a_forward_job_on_the_same_target() {
    let forward = leg(
        "forward-job",
        ShadowEvalTargetType::Key,
        "key-1",
        JobDirection::Forward,
        now(),
        now() + Duration::days(1),
    );
    let store = FakeStore::new(now())
        .with_targets(targets())
        .with_legs(vec![forward]);
    let response = start(&store, &request(REVERSE_KEY), None, now(), &mut minted(2))
        .await
        .unwrap();
    assert_eq!(response.status, ShadowEvalStatus::Running);
    assert_eq!(response.job.direction, JobDirection::Reverse);
    assert_eq!(response.job.baseline_model.as_deref(), Some(BASELINE));
    assert_eq!(store.stored_legs().len(), 2);
}

#[tokio::test]
async fn reports_a_lost_unique_index_as_a_concurrent_claim_without_minting_again() {
    let mut store = FakeStore::new(now()).with_targets(targets());
    store.create_conflict = true;
    let mut ids = minted(4);
    let error = error_of(start(&store, &request(ALL_TARGETS), None, now(), &mut ids).await);
    assert_eq!(
        error.to_string(),
        "A requested target was claimed by another forward shadow eval job concurrently. Stop it first."
    );
    assert_eq!(error.status_code(), 409);
    assert_eq!(ids.next(), None);
    assert!(store.stored_legs().is_empty());
}

#[tokio::test]
async fn keeps_the_job_when_the_funnel_seed_fails() {
    let mut store = FakeStore::new(now()).with_targets(targets());
    store.seeds_fail = true;
    let response = start(&store, &request(KEY_ONLY), None, now(), &mut minted(2))
        .await
        .unwrap();
    assert_eq!(response.status, ShadowEvalStatus::Running);
    assert_eq!(store.stored_legs().len(), 1);
    assert!(store.recorded().contains(&"seed:1".to_owned()));
}

#[tokio::test]
async fn rejects_an_unknown_target_before_any_slot_read() {
    let store = FakeStore::new(now()).with_targets(targets());
    let error = error_of(
        start(
            &store,
            &request(
                r#"{"router_names": ["shadow-router"], "user_ids": ["user-404"], "judge_model": "judge-model", "shadow_percentage": 10, "max_budget": 10}"#,
            ),
            None,
            now(),
            &mut minted(2),
        )
        .await,
    );
    assert_eq!(error.to_string(), "user_ids not on this proxy: user-404");
    assert_eq!(error.status_code(), 400);
    assert_eq!(store.recorded(), vec!["load"]);
}

#[tokio::test]
async fn stops_every_leg_and_reports_its_counts_and_labels() {
    let key = leg(
        "job-1",
        ShadowEvalTargetType::Key,
        "key-1",
        JobDirection::Forward,
        now() - Duration::days(1),
        now() + Duration::days(6),
    );
    let team = leg(
        "job-1",
        ShadowEvalTargetType::Team,
        "team-1",
        JobDirection::Forward,
        now() - Duration::days(1),
        now() + Duration::days(6),
    );
    let attempts = vec![
        FakeAttempt {
            job_id: key.id.clone(),
            created_at: now() - Duration::hours(2),
            spend: Decimal::new(100, 2),
        },
        FakeAttempt {
            job_id: key.id.clone(),
            created_at: now() - Duration::hours(1),
            spend: Decimal::new(50, 2),
        },
        FakeAttempt {
            job_id: team.id.clone(),
            created_at: now() - Duration::hours(1),
            spend: Decimal::new(25, 2),
        },
    ];
    let store = FakeStore::new(now())
        .with_targets(targets())
        .with_legs(vec![key, team])
        .with_attempts(attempts);
    let response = stop_shadow_eval_job(&store, "job-1", Some("admin-user"), now())
        .await
        .unwrap();
    assert_eq!(response.status, ShadowEvalStatus::Stopped);
    assert_eq!(response.job.stopped_by.as_deref(), Some("admin-user"));
    assert_eq!(response.job.job_id, "job-1");
    let key_target = &response.job.targets[0];
    assert_eq!(key_target.attempt_count, Some(2));
    assert_eq!(key_target.spend, Some(Decimal::new(150, 2)));
    assert_eq!(key_target.stopped_at, Some(now()));
    assert_eq!(key_target.target_alias.as_deref(), Some("prod-key"));
    assert_eq!(response.job.targets[1].attempt_count, Some(1));
    assert_eq!(response.job.targets[1].spend, Some(Decimal::new(25, 2)));
    assert_eq!(
        store.recorded(),
        vec!["stop:job-1", "legs", "stats", "load"]
    );
    let restamped: Vec<(Option<String>, Option<DateTime<Utc>>)> = store
        .stored_legs()
        .iter()
        .map(|stored| (stored.stopped_by.clone(), stored.stopped_at))
        .collect();
    assert_eq!(
        restamped,
        vec![
            (Some("admin-user".to_owned()), Some(now())),
            (Some("admin-user".to_owned()), Some(now())),
        ]
    );
}

#[tokio::test]
async fn a_stop_without_a_caller_is_recorded_as_operator() {
    let key = leg(
        "job-1",
        ShadowEvalTargetType::Key,
        "key-1",
        JobDirection::Forward,
        now(),
        now() + Duration::days(6),
    );
    let store = FakeStore::new(now())
        .with_targets(targets())
        .with_legs(vec![key]);
    let response = stop_shadow_eval_job(&store, "job-1", None, now())
        .await
        .unwrap();
    assert_eq!(response.job.stopped_by.as_deref(), Some("operator"));
}

#[tokio::test]
async fn a_leg_that_ended_on_its_own_keeps_its_stamp_and_gains_the_operator() {
    let earlier = now() - Duration::hours(3);
    let mut spent = leg(
        "job-1",
        ShadowEvalTargetType::Key,
        "key-1",
        JobDirection::Forward,
        now() - Duration::days(1),
        now() + Duration::days(6),
    );
    spent.stopped_at = Some(earlier);
    let active = leg(
        "job-1",
        ShadowEvalTargetType::Team,
        "team-1",
        JobDirection::Forward,
        now() - Duration::days(1),
        now() + Duration::days(6),
    );
    let store = FakeStore::new(now())
        .with_targets(targets())
        .with_legs(vec![spent, active]);
    let response = stop_shadow_eval_job(&store, "job-1", Some("admin-user"), now())
        .await
        .unwrap();
    assert_eq!(response.status, ShadowEvalStatus::Stopped);
    assert_eq!(response.job.targets[0].stopped_at, Some(earlier));
    assert_eq!(response.job.targets[1].stopped_at, Some(now()));
}

#[tokio::test]
async fn reports_404_for_a_job_that_was_never_started() {
    let store = FakeStore::new(now()).with_targets(targets());
    let error = error_of(stop_shadow_eval_job(&store, "missing", None, now()).await);
    assert_eq!(error.to_string(), "No shadow eval job missing");
    assert_eq!(error.status_code(), 404);
    assert_eq!(store.recorded(), vec!["stop:missing", "legs"]);
}

#[tokio::test]
async fn reports_400_with_the_derived_status_when_nothing_is_still_sampling() {
    let mut spent = leg(
        "job-1",
        ShadowEvalTargetType::Key,
        "key-1",
        JobDirection::Forward,
        now() - Duration::days(1),
        now() + Duration::days(6),
    );
    spent.stopped_at = Some(now() - Duration::hours(1));
    let store = FakeStore::new(now())
        .with_targets(targets())
        .with_legs(vec![spent]);
    let error = error_of(stop_shadow_eval_job(&store, "job-1", None, now()).await);
    assert_eq!(error.to_string(), "Job job-1 is already stopped");
    assert_eq!(error.status_code(), 400);
    let stored = store.stored_legs();
    assert_eq!(stored[0].stopped_by, None);
    assert_eq!(stored[0].stopped_at, Some(now() - Duration::hours(1)));
}

#[tokio::test]
async fn resolves_a_deleted_target_to_no_label_at_stop_time() {
    let key = leg(
        "job-1",
        ShadowEvalTargetType::Key,
        "key-1",
        JobDirection::Forward,
        now(),
        now() + Duration::days(6),
    );
    let mut rows = targets();
    rows.keys.clear();
    rows.teams.clear();
    rows.users.clear();
    let store = FakeStore::new(now())
        .with_targets(rows)
        .with_legs(vec![key]);
    let response = stop_shadow_eval_job(&store, "job-1", None, now())
        .await
        .unwrap();
    assert_eq!(response.job.targets[0].target_alias, None);
    assert_eq!(response.job.targets[0].key_name, None);
}

#[tokio::test]
async fn reports_the_start_clock_the_window_and_the_router_arm() {
    let store = FakeStore::new(now()).with_targets(targets());
    let body = r#"{"router_names": ["shadow-router"], "api_key_ids": ["key-1"], "judge_model": "judge-model", "shadow_percentage": 25.5, "max_budget": 0.01, "duration_days": 30}"#;
    let response = start(&store, &request(body), None, now(), &mut minted(2))
        .await
        .unwrap();
    assert_eq!(response.job.ends_at, now() + Duration::days(30));
    assert_eq!(response.job.shadow_percentage, 25.5_f64);
    assert_eq!(response.job.targets[0].max_budget, Some(Decimal::new(1, 2)));
    assert_eq!(response.status, ShadowEvalStatus::Running);
    assert_eq!(store.stored_legs()[0].router_names, vec![ROUTER.to_owned()]);
    assert_eq!(response.job.models, Vec::<String>::new());
}
