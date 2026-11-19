#[tokio::test]
async fn report_demo_reads_no_stored_report() {
    let harness = Harness::new();
    harness.seed_settings(&ready_stored());
    harness.seed_report(&report_json(&sample_report(minutes_before(
        fixed_now(),
        1440,
    ))));
    let reply = call(
        harness.app(),
        get("/roi-calculator/report?mode=demo", Some(&admin())),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    let body: ReportReply = reply.json();
    let summary: SummaryPeek = body.report.expect("demo summary");
    assert_eq!(summary.mode, "demo");
    assert_eq!(summary.synced_at, "2026-02-01T12:00:00+00:00");
    assert_eq!(summary.repos, vec!["example/gateway".to_owned()]);
    assert!(recorded(&harness.calls).is_empty());
}

#[tokio::test]
async fn report_demo_uses_the_injected_clock() {
    let harness = Harness::new().at(minutes_before(fixed_now(), 1440));
    let reply = call(
        harness.app(),
        get("/roi-calculator/report?mode=demo", Some(&admin())),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    let body: ReportReply = reply.json();
    let summary: SummaryPeek = body.report.expect("demo summary");
    assert_eq!(summary.synced_at, "2026-01-31T12:00:00+00:00");
}

#[tokio::test]
async fn report_live_without_a_stored_report_answers_null() {
    let harness = Harness::new();
    harness.seed_settings(&ready_stored());
    let reply = call(harness.app(), get(REPORT_PATH, Some(&admin()))).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.text(), r#"{"report":null}"#);
    assert!(!recorded(&harness.calls).contains(&"repo.param:roi_calculator_settings".to_owned()));
}

#[tokio::test]
async fn report_live_summarizes_the_stored_report() {
    let harness = Harness::new();
    harness.seed_settings(&ready_stored());
    let report = sample_report(minutes_before(fixed_now(), 60));
    harness.seed_report(&report_json(&report));
    let reply = call(harness.app(), get(REPORT_PATH, Some(&viewer()))).await;
    assert_eq!(reply.status, StatusCode::OK);
    let body: ReportReply = reply.json();
    let summary: SummaryPeek = body.report.expect("live summary");
    assert_eq!(summary.synced_at, report.synced_at);
    assert_eq!(summary.repos, report.repos);
    assert!(recorded(&harness.calls).contains(&"repo.param:roi_calculator_report".to_owned()));
}

#[tokio::test]
async fn report_live_applies_the_stored_identity_map() {
    let harness = Harness::new();
    harness.seed_settings(&stored_json(
        &["acme/widgets"],
        "gpt-4o",
        60.0,
        &[("alex", "mapped@example.com")],
    ));
    harness.seed_report(&report_json(&sample_report(fixed_now())));
    let reply = call(harness.app(), get(REPORT_PATH, Some(&admin()))).await;
    assert_eq!(reply.status, StatusCode::OK);
    let body: ReportReply = reply.json();
    let summary: SummaryPeek = body.report.expect("live summary");
    assert!(
        summary
            .people
            .iter()
            .any(|person| person.email == "mapped@example.com")
    );
}

#[tokio::test]
async fn report_rejects_an_unknown_mode() {
    let harness = Harness::new();
    let reply = call(
        harness.app(),
        get("/roi-calculator/report?mode=weekly", Some(&admin())),
    )
    .await;
    assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
    let issues = reply.issues();
    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0].kind, "literal_error");
    assert_eq!(issues[0].loc, vec!["query".to_owned(), "mode".to_owned()]);
    assert_eq!(issues[0].msg, MODE_LITERAL);
}

#[tokio::test]
async fn report_mode_is_case_sensitive() {
    let harness = Harness::new();
    let reply = call(
        harness.app(),
        get("/roi-calculator/report?mode=Demo", Some(&admin())),
    )
    .await;
    assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(reply.issues()[0].kind, "literal_error");
}

#[tokio::test]
async fn report_live_propagates_a_stored_report_failure() {
    let harness = Harness::new();
    harness.seed_settings(&ready_stored());
    harness.seed_report("[]");
    let reply = call(harness.app(), get(REPORT_PATH, Some(&admin()))).await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(reply.detail(), "Stored ROI Calculator report is invalid.");
}
