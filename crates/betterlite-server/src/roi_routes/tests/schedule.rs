#[test]
fn schedule_next_update_without_a_report_is_none() {
    let settings = settings_from(&ready_stored());
    let next = next_update(&settings, &idle_status(), None).expect("next update");
    assert_eq!(next, None);
}

#[test]
fn schedule_next_update_without_repositories_is_none() {
    let mut settings = settings_from(&ready_stored());
    settings.repos.clear();
    let report = sample_report(minutes_before(fixed_now(), 120));
    let next = next_update(&settings, &idle_status(), Some(&report)).expect("next update");
    assert_eq!(next, None);
}

#[test]
fn schedule_next_update_without_an_estimator_model_is_none() {
    let mut settings = settings_from(&ready_stored());
    settings.estimator_model.clear();
    let report = sample_report(minutes_before(fixed_now(), 120));
    let next = next_update(&settings, &idle_status(), Some(&report)).expect("next update");
    assert_eq!(next, None);
}

#[test]
fn schedule_next_update_with_a_zero_interval_is_none() {
    let mut settings = settings_from(&ready_stored());
    settings.update_interval_minutes = 0.0_f64;
    let report = sample_report(minutes_before(fixed_now(), 120));
    let next = next_update(&settings, &idle_status(), Some(&report)).expect("next update");
    assert_eq!(next, None);
}

#[test]
fn schedule_next_update_while_running_is_none() {
    let settings = settings_from(&ready_stored());
    let report = sample_report(minutes_before(fixed_now(), 120));
    let status = finished_status("2026-02-01T10:00:00+00:00");
    let running = ROISyncStatus {
        running: true,
        ..status
    };
    let next = next_update(&settings, &running, Some(&report)).expect("next update");
    assert_eq!(next, None);
}

#[test]
fn schedule_next_update_prefers_the_finished_at() {
    let settings = settings_from(&ready_stored());
    let report = sample_report(fixed_now());
    let status = ROISyncStatus {
        started_at: Some("2026-02-01T09:00:00+00:00".to_owned()),
        finished_at: Some("2026-02-01T11:00:00+00:00".to_owned()),
        ..idle_status()
    };
    let next = next_update(&settings, &status, Some(&report))
        .expect("next update")
        .expect("a next update");
    assert_eq!(next.to_rfc3339(), "2026-02-01T12:00:00+00:00");
}

#[test]
fn schedule_next_update_uses_the_started_at() {
    let settings = settings_from(&ready_stored());
    let report = sample_report(fixed_now());
    let status = ROISyncStatus {
        started_at: Some("2026-02-01T10:00:00+00:00".to_owned()),
        ..idle_status()
    };
    let next = next_update(&settings, &status, Some(&report))
        .expect("next update")
        .expect("a next update");
    assert_eq!(next.to_rfc3339(), "2026-02-01T11:00:00+00:00");
}

#[test]
fn schedule_next_update_uses_the_report_synced_at() {
    let settings = settings_from(&ready_stored());
    let mut report = sample_report(fixed_now());
    report.synced_at = "2026-02-01T10:30:00+00:00".to_owned();
    let next = next_update(&settings, &idle_status(), Some(&report))
        .expect("next update")
        .expect("a next update");
    assert_eq!(next.to_rfc3339(), "2026-02-01T11:30:00+00:00");
}

#[test]
fn schedule_next_update_skips_empty_timestamps() {
    let settings = settings_from(&ready_stored());
    let mut report = sample_report(fixed_now());
    report.synced_at = "2026-02-01T10:30:00+00:00".to_owned();
    let status = ROISyncStatus {
        started_at: Some(String::new()),
        finished_at: Some(String::new()),
        ..idle_status()
    };
    let next = next_update(&settings, &status, Some(&report))
        .expect("next update")
        .expect("a next update");
    assert_eq!(next.to_rfc3339(), "2026-02-01T11:30:00+00:00");
}

#[test]
fn schedule_next_update_reads_a_zulu_timestamp() {
    let settings = settings_from(&ready_stored());
    let report = sample_report(fixed_now());
    let status = finished_status("2026-02-01T11:00:00Z");
    let next = next_update(&settings, &status, Some(&report))
        .expect("next update")
        .expect("a next update");
    assert_eq!(next.to_rfc3339(), "2026-02-01T12:00:00+00:00");
}

#[test]
fn schedule_next_update_reads_an_offset_timestamp() {
    let settings = settings_from(&ready_stored());
    let report = sample_report(fixed_now());
    let status = finished_status("2026-02-01T13:00:00+02:00");
    let next = next_update(&settings, &status, Some(&report))
        .expect("next update")
        .expect("a next update");
    assert_eq!(next.to_rfc3339(), "2026-02-01T12:00:00+00:00");
}

#[test]
fn schedule_next_update_reads_a_date_only_timestamp() {
    let settings = settings_from(&ready_stored());
    let report = sample_report(fixed_now());
    let status = finished_status("2026-02-01");
    let next = next_update(&settings, &status, Some(&report))
        .expect("next update")
        .expect("a next update");
    assert_eq!(next.to_rfc3339(), "2026-02-01T01:00:00+00:00");
}

#[test]
fn schedule_next_update_adds_fractional_minutes() {
    let mut settings = settings_from(&ready_stored());
    settings.update_interval_minutes = 5.5_f64;
    let report = sample_report(fixed_now());
    let status = finished_status("2026-02-01T11:00:00+00:00");
    let next = next_update(&settings, &status, Some(&report))
        .expect("next update")
        .expect("a next update");
    assert_eq!(next.to_rfc3339(), "2026-02-01T11:05:30+00:00");
}

#[test]
fn schedule_next_update_rejects_a_bad_timestamp() {
    let settings = settings_from(&ready_stored());
    let report = sample_report(fixed_now());
    let status = finished_status("not a timestamp");
    let error = next_update(&settings, &status, Some(&report)).expect_err("a rejected anchor");
    assert!(matches!(error, RoiRoutesError::Timestamp { .. }));
}

#[test]
fn schedule_next_update_rejects_a_bare_number_timestamp() {
    let settings = settings_from(&ready_stored());
    let report = sample_report(fixed_now());
    let status = finished_status("1700000000");
    let error = next_update(&settings, &status, Some(&report)).expect_err("a rejected anchor");
    assert!(matches!(error, RoiRoutesError::Timestamp { .. }));
}

#[test]
fn schedule_next_update_rejects_an_unrepresentable_interval() {
    let mut settings = settings_from(&ready_stored());
    settings.update_interval_minutes = 1e300_f64;
    let report = sample_report(fixed_now());
    let status = finished_status("2026-02-01T11:00:00+00:00");
    let error = next_update(&settings, &status, Some(&report)).expect_err("a huge interval");
    assert!(matches!(error, RoiRoutesError::TimestampOverflow));
}

#[test]
fn schedule_next_update_rejects_a_multiplied_interval_overflow() {
    let mut settings = settings_from(&ready_stored());
    settings.update_interval_minutes = 1e28_f64;
    let report = sample_report(fixed_now());
    let status = finished_status("2026-02-01T11:00:00+00:00");
    let error = next_update(&settings, &status, Some(&report)).expect_err("an interval overflow");
    assert!(matches!(error, RoiRoutesError::TimestampOverflow));
}

#[test]
fn schedule_next_update_rejects_an_interval_beyond_microseconds() {
    let mut settings = settings_from(&ready_stored());
    settings.update_interval_minutes = 1e13_f64;
    let report = sample_report(fixed_now());
    let status = finished_status("2026-02-01T11:00:00+00:00");
    let error = next_update(&settings, &status, Some(&report)).expect_err("too many microseconds");
    assert!(matches!(error, RoiRoutesError::TimestampOverflow));
}

#[test]
fn schedule_next_update_rejects_an_overflow_past_the_calendar() {
    let mut settings = settings_from(&ready_stored());
    settings.update_interval_minutes = 1.4e11_f64;
    let report = sample_report(fixed_now());
    let status = finished_status("9999-12-31T23:00:00+00:00");
    let error = next_update(&settings, &status, Some(&report)).expect_err("an overflow");
    assert!(matches!(error, RoiRoutesError::TimestampOverflow));
}

#[tokio::test]
async fn schedule_run_without_the_database_returns() {
    let harness = Harness::new().without_repository();
    run_scheduled_sync(&harness.state())
        .await
        .expect("a scheduled sync without a database");
    assert!(recorded(&harness.calls).is_empty());
}

#[tokio::test]
async fn schedule_run_with_manual_updates_returns() {
    let harness = Harness::new();
    harness.seed_settings(&stored_json(&["acme/widgets"], "gpt-4o", 0.0_f64, &[]));
    run_scheduled_sync(&harness.state())
        .await
        .expect("a scheduled sync with manual updates");
    assert!(!recorded(&harness.calls).contains(&"store.open".to_owned()));
}

#[tokio::test]
async fn schedule_run_without_ready_settings_returns() {
    let harness = Harness::new();
    run_scheduled_sync(&harness.state())
        .await
        .expect("a scheduled sync without repositories");
    assert!(!recorded(&harness.calls).contains(&"store.open".to_owned()));
}

#[tokio::test]
async fn schedule_run_before_the_next_update_returns() {
    let harness = Harness::new();
    harness.seed_settings(&ready_stored());
    harness.seed_report(&report_json(&sample_report(fixed_now())));
    harness
        .store
        .set_status(finished_status("2026-02-01T11:30:00+00:00"));
    run_scheduled_sync(&harness.state())
        .await
        .expect("a scheduled sync before the next update");
    assert!(harness.manager.runs().is_empty());
    assert!(recorded(&harness.calls).contains(&"store.status".to_owned()));
}

#[tokio::test]
async fn schedule_run_after_the_next_update_starts_the_sync() {
    let harness = Harness::new();
    harness.seed_settings(&ready_stored());
    harness.seed_report(&report_json(&sample_report(minutes_before(
        fixed_now(),
        120,
    ))));
    harness
        .store
        .set_status(finished_status("2026-02-01T10:00:00+00:00"));
    harness.manager.accept_start(running_status());
    run_scheduled_sync(&harness.state())
        .await
        .expect("a due scheduled sync");
    let runs = harness.manager.runs();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].interval.to_bits(), 60.0_f64.to_bits());
    assert!(runs[0].coordinated);
    assert_eq!(runs[0].model, "gpt-4o");
}

#[tokio::test]
async fn schedule_run_uses_the_manager_status_without_a_store_status() {
    let harness = Harness::new();
    harness.seed_settings(&ready_stored());
    harness.seed_report(&report_json(&sample_report(minutes_before(
        fixed_now(),
        120,
    ))));
    harness
        .manager
        .set_status(finished_status("2026-02-01T10:30:00+00:00"));
    run_scheduled_sync(&harness.state())
        .await
        .expect("a due scheduled sync");
    assert_eq!(harness.manager.runs().len(), 1);
    assert!(recorded(&harness.calls).contains(&"manager.status".to_owned()));
}

#[tokio::test]
async fn schedule_run_answers_ok_when_the_start_is_refused() {
    let harness = Harness::new();
    harness.seed_settings(&ready_stored());
    harness.seed_report(&report_json(&sample_report(minutes_before(
        fixed_now(),
        120,
    ))));
    harness
        .store
        .set_status(finished_status("2026-02-01T10:00:00+00:00"));
    run_scheduled_sync(&harness.state())
        .await
        .expect("a refused scheduled sync");
    assert_eq!(harness.manager.runs().len(), 1);
}

#[tokio::test]
async fn schedule_run_propagates_a_store_failure() {
    let harness = Harness::new();
    harness.seed_settings(&ready_stored());
    harness.store.fail_status();
    let error = run_scheduled_sync(&harness.state())
        .await
        .expect_err("a store failure");
    assert!(matches!(
        error,
        RoiRoutesError::Conflict("the store failed")
    ));
}

#[tokio::test]
async fn schedule_run_propagates_a_report_failure() {
    let harness = Harness::new();
    harness.seed_settings(&ready_stored());
    harness.seed_report("[]");
    let error = run_scheduled_sync(&harness.state())
        .await
        .expect_err("a report failure");
    assert!(matches!(error, RoiRoutesError::Settings(_)));
}

#[tokio::test]
async fn schedule_run_propagates_a_completion_failure() {
    let harness = Harness::new().with_completion(missing_key_completion());
    harness.seed_settings(&ready_stored());
    harness.seed_report(&report_json(&sample_report(minutes_before(
        fixed_now(),
        120,
    ))));
    harness
        .store
        .set_status(finished_status("2026-02-01T10:00:00+00:00"));
    let error = run_scheduled_sync(&harness.state())
        .await
        .expect_err("a completion failure");
    assert!(matches!(error, RoiRoutesError::Access(_)));
    assert!(harness.manager.runs().is_empty());
}
