#[tokio::test]
async fn route_sync_status_answers_the_store_status() {
    let harness = Harness::new();
    harness.seed_settings(&ready_stored());
    harness.store.set_status(ROISyncStatus {
        stage: "Complete".to_owned(),
        ..idle_status()
    });
    let reply = call(harness.app(), get(SYNC_PATH, Some(&admin()))).await;
    assert_eq!(reply.status, StatusCode::OK);
    let status: ROISyncStatus = reply.json();
    assert_eq!(status.stage, "Complete");
    assert!(recorded(&harness.calls).contains(&"store.status".to_owned()));
}

#[tokio::test]
async fn route_sync_status_falls_back_to_the_manager_status() {
    let harness = Harness::new();
    harness.seed_settings(&ready_stored());
    harness.manager.set_status(ROISyncStatus {
        stage: "From the manager".to_owned(),
        ..running_status()
    });
    let reply = call(harness.app(), get(SYNC_PATH, Some(&viewer()))).await;
    assert_eq!(reply.status, StatusCode::OK);
    let status: ROISyncStatus = reply.json();
    assert_eq!(status.stage, "From the manager");
    assert!(status.running);
}

#[tokio::test]
async fn route_sync_status_requires_a_caller() {
    let harness = Harness::new();
    let reply = call(harness.app(), get(SYNC_PATH, None)).await;
    assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
    assert!(recorded(&harness.calls).is_empty());
}

#[tokio::test]
async fn route_sync_status_denies_a_member() {
    let harness = Harness::new();
    let reply = call(harness.app(), get(SYNC_PATH, Some(&member()))).await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);
    assert_eq!(reply.detail(), READ_FORBIDDEN);
}

#[tokio::test]
async fn route_report_demo_is_readable_by_a_viewer() {
    let harness = Harness::new();
    let reply = call(
        harness.app(),
        get("/roi-calculator/report?mode=demo", Some(&viewer())),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    let body: ReportReply = reply.json();
    assert_eq!(body.report.expect("demo summary").mode, "demo");
    assert!(recorded(&harness.calls).is_empty());
}

#[tokio::test]
async fn route_report_denies_a_member() {
    let harness = Harness::new();
    let reply = call(harness.app(), get(REPORT_PATH, Some(&member()))).await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);
    assert_eq!(reply.detail(), READ_FORBIDDEN);
}

#[tokio::test]
async fn route_sync_start_denies_a_viewer() {
    let harness = Harness::new();
    let reply = call(harness.app(), post(SYNC_PATH, Some(&viewer()))).await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);
    assert_eq!(reply.detail(), WRITE_FORBIDDEN);
    assert!(recorded(&harness.calls).is_empty());
}

#[tokio::test]
async fn route_sync_cancel_denies_a_viewer() {
    let harness = Harness::new();
    let reply = call(harness.app(), delete(SYNC_PATH, Some(&viewer()))).await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);
    assert_eq!(reply.detail(), WRITE_FORBIDDEN);
    assert!(recorded(&harness.calls).is_empty());
}

#[tokio::test]
async fn route_identity_map_denies_a_viewer() {
    let harness = Harness::new();
    let reply = call(
        harness.app(),
        put(
            IDENTITY_MAP_PATH,
            Some(&viewer()),
            r#"{"github_login":"octo","email":null}"#,
        ),
    )
    .await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);
    assert_eq!(reply.detail(), WRITE_FORBIDDEN);
}

#[tokio::test]
async fn route_identity_map_denies_a_member() {
    let harness = Harness::new();
    let reply = call(
        harness.app(),
        put(
            IDENTITY_MAP_PATH,
            Some(&member()),
            r#"{"github_login":"octo","email":null}"#,
        ),
    )
    .await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);
    assert_eq!(reply.detail(), WRITE_FORBIDDEN);
}

#[tokio::test]
async fn route_setup_reset_denies_a_viewer() {
    let harness = Harness::new();
    let reply = call(harness.app(), post(SETUP_RESET_PATH, Some(&viewer()))).await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);
    assert_eq!(reply.detail(), WRITE_FORBIDDEN);
    assert!(recorded(&harness.calls).is_empty());
}

#[tokio::test]
async fn route_setup_reset_denies_a_member() {
    let harness = Harness::new();
    let reply = call(harness.app(), post(SETUP_RESET_PATH, Some(&member()))).await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);
    assert_eq!(reply.detail(), WRITE_FORBIDDEN);
}

#[tokio::test]
async fn route_sync_path_rejects_put() {
    let harness = Harness::new();
    let reply = call(harness.app(), put(SYNC_PATH, Some(&admin()), "{}")).await;
    assert_eq!(reply.status, StatusCode::METHOD_NOT_ALLOWED);
}

#[tokio::test]
async fn route_report_path_rejects_post() {
    let harness = Harness::new();
    let reply = call(harness.app(), post(REPORT_PATH, Some(&admin()))).await;
    assert_eq!(reply.status, StatusCode::METHOD_NOT_ALLOWED);
}

#[tokio::test]
async fn route_identity_path_rejects_get() {
    let harness = Harness::new();
    let reply = call(harness.app(), get(IDENTITY_MAP_PATH, Some(&admin()))).await;
    assert_eq!(reply.status, StatusCode::METHOD_NOT_ALLOWED);
}

#[tokio::test]
async fn route_setup_reset_rejects_get() {
    let harness = Harness::new();
    let reply = call(harness.app(), get(SETUP_RESET_PATH, Some(&admin()))).await;
    assert_eq!(reply.status, StatusCode::METHOD_NOT_ALLOWED);
}

#[tokio::test]
async fn route_sync_status_requires_the_database() {
    let harness = Harness::new().without_repository();
    let reply = call(harness.app(), get(SYNC_PATH, Some(&admin()))).await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(reply.detail().starts_with("DB not connected."));
}

#[tokio::test]
async fn route_report_demo_requires_the_database() {
    let harness = Harness::new().without_repository();
    let reply = call(
        harness.app(),
        get("/roi-calculator/report?mode=demo", Some(&admin())),
    )
    .await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(reply.detail().starts_with("DB not connected."));
}

#[tokio::test]
async fn route_identity_map_requires_the_database() {
    let harness = Harness::new().without_repository();
    let reply = call(
        harness.app(),
        put(
            IDENTITY_MAP_PATH,
            Some(&admin()),
            r#"{"github_login":"octo","email":null}"#,
        ),
    )
    .await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(reply.detail().starts_with("DB not connected."));
}

#[tokio::test]
async fn route_setup_reset_requires_the_database() {
    let harness = Harness::new().without_repository();
    let reply = call(harness.app(), post(SETUP_RESET_PATH, Some(&admin()))).await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(reply.detail().starts_with("DB not connected."));
}
