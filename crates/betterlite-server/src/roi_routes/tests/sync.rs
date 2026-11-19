#[tokio::test]
async fn sync_status_computes_next_update_from_the_finished_status() {
    let harness = Harness::new();
    harness.seed_settings(&ready_stored());
    let mut report = sample_report(minutes_before(fixed_now(), 120));
    report.synced_at = "2026-02-01T10:00:00+00:00".to_owned();
    harness.seed_report(&report_json(&report));
    harness
        .store
        .set_status(finished_status("2026-02-01T11:30:00+00:00"));
    let reply = call(harness.app(), get(SYNC_PATH, Some(&admin()))).await;
    assert_eq!(reply.status, StatusCode::OK);
    let status: ROISyncStatus = reply.json();
    assert_eq!(
        status.next_update.as_deref(),
        Some("2026-02-01T12:30:00+00:00")
    );
}

#[tokio::test]
async fn sync_status_without_a_report_has_no_next_update() {
    let harness = Harness::new();
    harness.seed_settings(&ready_stored());
    harness
        .store
        .set_status(finished_status("2026-02-01T11:30:00+00:00"));
    let reply = call(harness.app(), get(SYNC_PATH, Some(&admin()))).await;
    assert_eq!(reply.status, StatusCode::OK);
    let status: ROISyncStatus = reply.json();
    assert_eq!(status.next_update, None);
}

#[tokio::test]
async fn sync_status_with_manual_updates_has_no_next_update() {
    let harness = Harness::new();
    harness.seed_settings(&stored_json(&["acme/widgets"], "gpt-4o", 0.0_f64, &[]));
    harness.seed_report(&report_json(&sample_report(fixed_now())));
    harness
        .store
        .set_status(finished_status("2026-02-01T11:30:00+00:00"));
    let reply = call(harness.app(), get(SYNC_PATH, Some(&admin()))).await;
    assert_eq!(reply.status, StatusCode::OK);
    let status: ROISyncStatus = reply.json();
    assert_eq!(status.next_update, None);
}

#[tokio::test]
async fn sync_status_overwrites_a_running_next_update() {
    let harness = Harness::new();
    harness.seed_settings(&ready_stored());
    harness.seed_report(&report_json(&sample_report(fixed_now())));
    harness.store.set_status(ROISyncStatus {
        running: true,
        next_update: Some("2000-01-01T00:00:00+00:00".to_owned()),
        ..idle_status()
    });
    let reply = call(harness.app(), get(SYNC_PATH, Some(&viewer()))).await;
    assert_eq!(reply.status, StatusCode::OK);
    let status: ROISyncStatus = reply.json();
    assert_eq!(status.next_update, None);
}

#[tokio::test]
async fn sync_status_rejects_a_bad_stored_timestamp() {
    let harness = Harness::new();
    harness.seed_settings(&ready_stored());
    let mut report = sample_report(fixed_now());
    report.synced_at = "not a timestamp".to_owned();
    harness.seed_report(&report_json(&report));
    let reply = call(harness.app(), get(SYNC_PATH, Some(&admin()))).await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(reply.text().contains("internal_server_error"));
}

#[tokio::test]
async fn sync_status_propagates_a_store_failure() {
    let harness = Harness::new();
    harness.seed_settings(&ready_stored());
    harness.store.fail_status();
    let reply = call(harness.app(), get(SYNC_PATH, Some(&admin()))).await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(reply.text().contains("internal_server_error"));
    assert!(!recorded(&harness.calls).contains(&"manager.status".to_owned()));
}

#[tokio::test]
async fn sync_start_answers_conflict_when_setup_is_not_ready() {
    let harness = Harness::new();
    let reply = call(harness.app(), post(SYNC_PATH, Some(&admin()))).await;
    assert_eq!(reply.status, StatusCode::CONFLICT);
    assert_eq!(reply.detail(), NOT_READY);
    assert!(!recorded(&harness.calls).contains(&"manager.start".to_owned()));
    assert!(!recorded(&harness.calls).contains(&"store.open".to_owned()));
}

#[tokio::test]
async fn sync_start_answers_conflict_when_a_sync_is_running() {
    let harness = Harness::new();
    harness.seed_settings(&ready_stored());
    let reply = call(harness.app(), post(SYNC_PATH, Some(&admin()))).await;
    assert_eq!(reply.status, StatusCode::CONFLICT);
    assert_eq!(reply.detail(), ALREADY_RUNNING);
    assert_eq!(harness.manager.runs().len(), 1);
}

#[tokio::test]
async fn sync_start_records_the_run_and_answers_accepted() {
    let harness = Harness::new();
    harness.seed_settings(&ready_stored());
    harness.manager.accept_start(running_status());
    let reply = call(harness.app(), post(SYNC_PATH, Some(&admin()))).await;
    assert_eq!(reply.status, StatusCode::ACCEPTED);
    let status: ROISyncStatus = reply.json();
    assert!(status.running);
    let runs = harness.manager.runs();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].model, "gpt-4o");
    assert_eq!(runs[0].interval.to_bits(), 0.0_f64.to_bits());
    assert!(runs[0].coordinated);
    assert_eq!(runs[0].repos, vec!["acme/widgets".to_owned()]);
}

#[tokio::test]
async fn sync_start_reports_a_missing_estimator_key() {
    let harness = Harness::new().with_completion(missing_key_completion());
    harness.seed_settings(&ready_stored());
    let reply = call(harness.app(), post(SYNC_PATH, Some(&admin()))).await;
    assert_eq!(reply.status, StatusCode::CONFLICT);
    assert_eq!(reply.detail(), MISSING_KEY);
    assert!(!recorded(&harness.calls).contains(&"manager.start".to_owned()));
}

#[tokio::test]
async fn sync_start_requires_the_router_model_in_the_catalog() {
    let mut harness = Harness::new();
    harness.models(&["other-model"]);
    harness.seed_settings(&ready_stored());
    let reply = call(harness.app(), post(SYNC_PATH, Some(&admin()))).await;
    assert_eq!(reply.status, StatusCode::CONFLICT);
    assert_eq!(reply.detail(), NOT_READY);
}

#[tokio::test]
async fn sync_cancel_cancels_the_store_then_the_manager() {
    let harness = Harness::new();
    harness.store.set_status(idle_status());
    let reply = call(harness.app(), delete(SYNC_PATH, Some(&admin()))).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(
        recorded(&harness.calls),
        vec![
            "store.open".to_owned(),
            "store.cancel".to_owned(),
            "manager.cancel".to_owned(),
            "store.status".to_owned(),
        ]
    );
}

#[tokio::test]
async fn sync_cancel_answers_the_manager_status_without_a_store_status() {
    let harness = Harness::new();
    harness.manager.set_status(ROISyncStatus {
        stage: "Manager idle".to_owned(),
        ..idle_status()
    });
    let reply = call(harness.app(), delete(SYNC_PATH, Some(&admin()))).await;
    assert_eq!(reply.status, StatusCode::OK);
    let status: ROISyncStatus = reply.json();
    assert_eq!(status.stage, "Manager idle");
}

#[tokio::test]
async fn sync_cancel_requires_the_database() {
    let harness = Harness::new().without_repository();
    let reply = call(harness.app(), delete(SYNC_PATH, Some(&admin()))).await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(recorded(&harness.calls).is_empty());
}
