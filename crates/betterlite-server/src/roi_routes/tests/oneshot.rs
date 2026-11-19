#[tokio::test]
async fn oneshot_sync_status_answers_the_router() {
    let harness = Harness::new();
    harness.seed_settings(&ready_stored());
    let reply = call(harness.app(), get(SYNC_PATH, Some(&admin()))).await;
    assert_eq!(reply.status, StatusCode::OK);
    let _status: ROISyncStatus = reply.json();
}

#[tokio::test]
async fn oneshot_sync_start_answers_the_router() {
    let harness = Harness::new();
    harness.seed_settings(&ready_stored());
    harness.manager.accept_start(running_status());
    let reply = call(harness.app(), post(SYNC_PATH, Some(&admin()))).await;
    assert_eq!(reply.status, StatusCode::ACCEPTED);
    let status: ROISyncStatus = reply.json();
    assert!(status.running);
}

#[tokio::test]
async fn oneshot_sync_cancel_answers_the_router() {
    let harness = Harness::new();
    let reply = call(harness.app(), delete(SYNC_PATH, Some(&admin()))).await;
    assert_eq!(reply.status, StatusCode::OK);
    let _status: ROISyncStatus = reply.json();
}

#[tokio::test]
async fn oneshot_report_demo_answers_the_router() {
    let harness = Harness::new();
    let reply = call(
        harness.app(),
        get("/roi-calculator/report?mode=demo", Some(&admin())),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    let body: ReportReply = reply.json();
    assert_eq!(body.report.expect("demo summary").mode, "demo");
}

#[tokio::test]
async fn oneshot_identity_map_answers_the_router() {
    let harness = Harness::new();
    let reply = call(
        harness.app(),
        put(
            IDENTITY_MAP_PATH,
            Some(&admin()),
            r#"{"github_login":"octo","email":"octo@example.com"}"#,
        ),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    let body: IdentityReply = reply.json();
    assert_eq!(
        body.identity_map.get("octo").map(String::as_str),
        Some("octo@example.com")
    );
}

#[tokio::test]
async fn oneshot_setup_reset_answers_the_router() {
    let harness = Harness::new();
    let reply = call(harness.app(), post(SETUP_RESET_PATH, Some(&admin()))).await;
    assert_eq!(reply.status, StatusCode::OK);
    let body: ROISettingsResponse = reply.json();
    assert!(body.repos.is_empty());
}

#[tokio::test]
async fn oneshot_unknown_path_is_not_found() {
    let harness = Harness::new();
    let reply = call(
        harness.app(),
        get("/roi-calculator/not-a-route", Some(&admin())),
    )
    .await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND);
}
