#[tokio::test]
async fn reset_answers_a_conflict_when_the_claim_is_taken() {
    let harness = Harness::new();
    harness.seed_settings(&ready_stored());
    harness.store.refuse_acquire();
    let reply = call(harness.app(), post(SETUP_RESET_PATH, Some(&admin()))).await;
    assert_eq!(reply.status, StatusCode::CONFLICT);
    assert_eq!(reply.detail(), RESTART_BLOCKED);
    let calls = recorded(&harness.calls);
    assert!(!calls.contains(&"repo.param:roi_calculator_settings".to_owned()));
    assert!(!calls.contains(&"store.finish".to_owned()));
}

#[tokio::test]
async fn reset_claims_with_the_restarting_status() {
    let harness = Harness::new();
    let reply = call(harness.app(), post(SETUP_RESET_PATH, Some(&admin()))).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(
        harness.store.acquired_owner(),
        Some("00000000-0000-0000-0000-000000000007".to_owned())
    );
    let claimed = harness.store.acquired_status().expect("claimed status");
    assert!(claimed.running);
    assert_eq!(claimed.phase, ROISyncPhase::Spend);
    assert_eq!(claimed.stage, "Restarting setup");
    assert_eq!(claimed.done, 0);
    assert_eq!(claimed.total, 0);
    assert_eq!(claimed.estimated, 0);
    assert_eq!(claimed.reused, 0);
    assert_eq!(claimed.needs_attention, 0);
    assert_eq!(claimed.error, None);
    assert_eq!(
        harness
            .store
            .acquired_interval()
            .expect("claimed interval")
            .to_bits(),
        0.0_f64.to_bits()
    );
}

#[tokio::test]
async fn reset_empties_the_repositories_and_clears_the_report() {
    let harness = Harness::new();
    harness.seed_settings(&stored_json(
        &["acme/widgets", "acme/api"],
        "gpt-4o",
        60.0,
        &[("alex", "alex@example.com")],
    ));
    harness.seed_report(&report_json(&sample_report(fixed_now())));
    let reply = call(harness.app(), post(SETUP_RESET_PATH, Some(&admin()))).await;
    assert_eq!(reply.status, StatusCode::OK);
    let body: ROISettingsResponse = reply.json();
    assert!(body.repos.is_empty());
    assert!(!body.ready);
    assert_eq!(body.available_models, vec!["gpt-4o".to_owned()]);
    assert_eq!(
        body.identity_map.get("alex").map(String::as_str),
        Some("alex@example.com")
    );
    assert!(harness.store.cleared());
    let stored = stored_peek(&harness);
    assert!(stored.repos.is_empty());
    assert_eq!(stored.estimator_model, "gpt-4o");
}

#[tokio::test]
async fn reset_keeps_the_ciphertexts() {
    let harness = Harness::new();
    harness.seed_settings(&ready_stored());
    let reply = call(harness.app(), post(SETUP_RESET_PATH, Some(&admin()))).await;
    assert_eq!(reply.status, StatusCode::OK);
    let stored = stored_peek(&harness);
    assert_eq!(stored.github_token, SEALED_TOKEN);
    assert_eq!(stored.estimator_key, SEALED_ESTIMATOR);
}

#[tokio::test]
async fn reset_finishes_the_lease_as_idle() {
    let harness = Harness::new();
    let reply = call(harness.app(), post(SETUP_RESET_PATH, Some(&admin()))).await;
    assert_eq!(reply.status, StatusCode::OK);
    let finished = harness.store.finished_status().expect("finished status");
    assert!(!finished.running);
    assert_eq!(finished.phase, ROISyncPhase::Idle);
    assert_eq!(finished.stage, "Idle");
    assert_eq!(finished.error, None);
}

#[tokio::test]
async fn reset_finishes_the_lease_when_saving_fails() {
    let harness = Harness::new();
    harness.seed_settings(&ready_stored());
    harness.repository.fail_writes();
    let reply = call(harness.app(), post(SETUP_RESET_PATH, Some(&admin()))).await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
    let finished = harness.store.finished_status().expect("finished status");
    assert!(!finished.running);
    assert_eq!(finished.stage, "Idle");
}

#[tokio::test]
async fn reset_finishes_the_lease_when_loading_fails() {
    let harness = Harness::new();
    harness.repository.fail_reads();
    let reply = call(harness.app(), post(SETUP_RESET_PATH, Some(&admin()))).await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
    let finished = harness.store.finished_status().expect("finished status");
    assert!(!finished.running);
    assert_eq!(finished.phase, ROISyncPhase::Idle);
}

#[tokio::test]
async fn reset_answers_a_clear_failure_after_finishing() {
    let harness = Harness::new();
    harness.seed_settings(&ready_stored());
    harness.store.fail_clear();
    let reply = call(harness.app(), post(SETUP_RESET_PATH, Some(&admin()))).await;
    assert_eq!(reply.status, StatusCode::CONFLICT);
    assert_eq!(reply.detail(), "the clear failed");
    assert!(harness.store.finished_status().is_some());
}

#[tokio::test]
async fn reset_answers_a_finish_failure() {
    let harness = Harness::new();
    harness.store.fail_finish();
    let reply = call(harness.app(), post(SETUP_RESET_PATH, Some(&admin()))).await;
    assert_eq!(reply.status, StatusCode::CONFLICT);
    assert_eq!(reply.detail(), "the finish failed");
}
