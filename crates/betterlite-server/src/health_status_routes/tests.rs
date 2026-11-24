#![cfg(test)]

mod support;
mod units;

use std::sync::Arc;

use axum::http::StatusCode;
use betterlite_models::decisions::Json;
use rstest::rstest;

use crate::health_status_routes::save::perform_health_check_and_save;
use crate::health_status_routes::tests::support::{
    FakeProvider, admin, assert_issue, background, capture_logs, cycle_catalog,
    database as open_database, deployment, deployment_with_info, deployment_without_id, endpoint,
    failing_router, fixture, get, insert_rows, key_context, latest_checks, probe, serve, settle,
    team_user, text, text_field, unconnected, user,
};

#[rstest]
#[case::history("/health/history")]
#[case::latest("/health/latest")]
#[tokio::test]
async fn check_history_and_latest_without_a_store_are_500(#[case] uri: &str) {
    let (state, _provider, _source) = fixture();
    let reply = get(state, Some(&admin()), uri).await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR, "{uri}");
    assert_eq!(reply.detail_error(), "Database not initialized", "{uri}");
}

#[tokio::test]
async fn save_returns_the_lists_and_their_counts() {
    let models = vec![
        deployment("a", "id-a", "openai/a"),
        deployment("b", "id-b", "openai/b"),
    ];
    let (mut state, _provider, _source) = fixture();
    let provider = Arc::new(FakeProvider::new().failing("openai/b"));
    state.cycle.provider = Arc::<FakeProvider>::clone(&provider);
    let outcome = perform_health_check_and_save(&state, probe(&models))
        .await
        .expect("the probe runs");
    let mut calls = provider.calls();
    calls.sort();
    assert_eq!(calls, vec!["openai/a".to_owned(), "openai/b".to_owned()]);
    assert_eq!(outcome.healthy_endpoints.len(), 1);
    assert_eq!(outcome.unhealthy_endpoints.len(), 1);
    assert_eq!(outcome.healthy_count, 1);
    assert_eq!(outcome.unhealthy_count, 1);
    let body = outcome.to_json();
    let keys: Vec<&str> = body.keys().map(String::as_str).collect();
    assert_eq!(
        keys,
        [
            "healthy_endpoints",
            "unhealthy_endpoints",
            "healthy_count",
            "unhealthy_count",
        ]
    );
}

#[tokio::test]
async fn save_probes_the_dependency_the_catalog_resolves() {
    let models = vec![
        deployment_with_info(
            "router",
            "id-router",
            vec![
                ("model", text("auto_router/default")),
                ("auto_router_default_model", text("dep")),
            ],
            Vec::new(),
        ),
        deployment("dep", "id-dep", "openai/dep"),
    ];
    let (mut state, provider, _source) = fixture();
    state.cycle.catalog = Some(cycle_catalog(&models));
    let mut call = probe(&models);
    call.model_id = Some("id-router");
    let outcome = perform_health_check_and_save(&state, call)
        .await
        .expect("the probe runs");
    assert_eq!(provider.calls(), vec!["openai/dep".to_owned()]);
    // the dependency probe leaves the display rows, so only the router it graded is healthy here
    assert_eq!(outcome.healthy_count, 1);
}

#[tokio::test]
async fn save_names_the_row_by_the_cli_model() {
    let mut database = open_database().await;
    let rows = Arc::new(database.rows());
    let (mut state, _provider, _source) = fixture();
    state.rows = Some(Arc::clone(&rows));
    let tasks = state.tasks.clone();
    let mut call = probe(&[]);
    call.cli_model = Some("cli-model");
    call.target_model = Some("group-a");
    let outcome = perform_health_check_and_save(&state, call)
        .await
        .expect("the probe runs");
    assert_eq!(outcome.healthy_count, 0);
    assert_eq!(tasks.len(), 1);
    settle(&mut database, &tasks).await;
    let stored = rows.fetch_latest().await;
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].model_name, "cli-model");
    assert_eq!(stored[0].checked_by.as_deref(), Some("user-1"));
    assert_eq!(stored[0].status, "unhealthy");
    database.finish().await;
}

#[tokio::test]
async fn save_falls_back_to_the_target_model() {
    let mut database = open_database().await;
    let rows = Arc::new(database.rows());
    let models = vec![deployment("group-a", "id-a", "openai/a")];
    let (mut state, provider, _source) = fixture();
    state.rows = Some(Arc::clone(&rows));
    let tasks = state.tasks.clone();
    let mut call = probe(&models);
    call.target_model = Some("group-a");
    call.model_id = Some("id-a");
    let outcome = perform_health_check_and_save(&state, call)
        .await
        .expect("the probe runs");
    assert_eq!(provider.calls(), vec!["openai/a".to_owned()]);
    assert_eq!(outcome.healthy_count, 1);
    assert_eq!(tasks.len(), 1);
    settle(&mut database, &tasks).await;
    let stored = rows.fetch_latest().await;
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].model_name, "group-a");
    assert_eq!(stored[0].model_id.as_deref(), Some("id-a"));
    assert_eq!(stored[0].healthy_count, 1_i32);
    assert_eq!(stored[0].status, "healthy");
    database.finish().await;
}

#[tokio::test]
async fn save_writes_nothing_without_a_store() {
    let models = vec![deployment("group-a", "id-a", "openai/a")];
    let (state, _provider, _source) = fixture();
    let mut call = probe(&models);
    call.target_model = Some("group-a");
    perform_health_check_and_save(&state, call)
        .await
        .expect("the probe runs");
    assert_eq!(state.tasks.len(), 0);
}

#[tokio::test]
async fn save_writes_nothing_without_a_model_name() {
    let models = vec![deployment("group-a", "id-a", "openai/a")];
    let (mut state, _provider, _source) = fixture();
    state.rows = Some(Arc::new(unconnected()));
    perform_health_check_and_save(&state, probe(&models))
        .await
        .expect("the probe runs");
    assert_eq!(state.tasks.len(), 0);
}

#[tokio::test]
async fn health_without_a_caller_is_401() {
    let (state, _provider, _source) = fixture();
    let reply = get(state, None, "/health").await;
    assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn health_without_a_model_list_or_cli_model_is_500() {
    let (buffer, _guard) = capture_logs();
    let (state, _provider, _source) = fixture();
    let reply = get(state, Some(&admin()), "/health").await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(reply.detail_error(), "Model list not initialized");
    let logs = buffer.text();
    assert!(
        logs.contains(
            "litellm.proxy.proxy_server.py::health_endpoint(): Exception occu\u{72}ed - Model list not initialized"
        ),
        "{logs}"
    );
}

#[tokio::test]
async fn health_without_a_model_list_saves_the_cli_model() {
    let mut database = open_database().await;
    let rows = Arc::new(database.rows());
    let (mut state, _provider, _source) = fixture();
    state.user_model = Some("cli-model".to_owned());
    state.rows = Some(Arc::clone(&rows));
    let tasks = state.tasks.clone();
    let reply = get(state, Some(&admin()), "/health").await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.count("healthy_count"), 0);
    assert_eq!(reply.count("unhealthy_count"), 0);
    assert_eq!(tasks.len(), 1);
    settle(&mut database, &tasks).await;
    let stored = rows.fetch_latest().await;
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].model_name, "cli-model");
    database.finish().await;
}

#[tokio::test]
async fn health_admin_receives_the_result_as_it_is() {
    let models = vec![deployment_with_info(
        "group-a",
        "id-a",
        vec![
            ("model", text("openai/a")),
            ("api_base", text("https://internal.example.com")),
        ],
        Vec::new(),
    )];
    let (mut state, provider, _source) = fixture();
    serve(&mut state, models);
    let reply = get(state, Some(&admin()), "/health").await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.count("healthy_count"), 1);
    assert_eq!(reply.header("litellm-health-field-notice"), None);
    assert_eq!(provider.calls(), vec!["openai/a".to_owned()]);
    let healthy = reply.list("healthy_endpoints");
    assert_eq!(
        text_field(&healthy[0], "api_base"),
        Some("https://internal.example.com".to_owned())
    );
}

#[tokio::test]
async fn health_non_admin_gets_the_notice_header_and_stripped_rows() {
    let models = vec![deployment_with_info(
        "a",
        "id-a",
        vec![
            ("model", text("openai/a")),
            ("api_base", text("https://internal.example.com")),
        ],
        Vec::new(),
    )];
    let (mut state, _provider, _source) = fixture();
    serve(&mut state, models);
    let reply = get(state, Some(&user(r#"["a"]"#)), "/health").await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(
        reply.header("litellm-health-field-notice").as_deref(),
        Some("api_base, api_version, aws_bedrock_runtime_endpoint are admin-only on this endpoint")
    );
    let healthy = reply.list("healthy_endpoints");
    assert_eq!(text_field(&healthy[0], "api_base"), None);
    assert_eq!(
        text_field(&healthy[0], "model"),
        Some("openai/a".to_owned())
    );
}

#[tokio::test]
async fn health_probes_only_the_deployment_a_model_id_names() {
    let models = vec![
        deployment("a", "id-a", "openai/a"),
        deployment("b", "id-b", "openai/b"),
    ];
    let (mut state, provider, _source) = fixture();
    serve(&mut state, models);
    let reply = get(state, Some(&admin()), "/health?model_id=id-b").await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.count("healthy_count"), 1);
    assert_eq!(provider.calls(), vec!["openai/b".to_owned()]);
}

#[tokio::test]
async fn health_probes_only_the_deployments_a_model_name_names() {
    let models = vec![
        deployment("a", "id-a", "openai/a"),
        deployment("b", "id-b", "openai/b"),
    ];
    let (mut state, provider, _source) = fixture();
    serve(&mut state, models);
    let reply = get(state, Some(&admin()), "/health?model=b").await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(provider.calls(), vec!["openai/b".to_owned()]);
}

#[tokio::test]
async fn health_unknown_model_id_is_a_404_before_the_access_checks() {
    let (buffer, _guard) = capture_logs();
    let models = vec![deployment("a", "id-a", "openai/a")];
    let (mut state, _provider, _source) = fixture();
    serve(&mut state, models);
    let reply = get(state, Some(&admin()), "/health?model=a&model_id=missing").await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND);
    assert_eq!(reply.detail_error(), "Model with ID missing not found");
    let logs = buffer.text();
    assert!(!logs.contains("Exception occu\u{72}ed"), "{logs}");
}

#[tokio::test]
async fn health_empty_model_id_is_ignored() {
    let models = vec![deployment("a", "id-a", "openai/a")];
    let (mut state, provider, _source) = fixture();
    serve(&mut state, models);
    let reply = get(state, Some(&admin()), "/health?model=a&model_id=").await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.count("healthy_count"), 1);
    assert_eq!(provider.calls(), vec!["openai/a".to_owned()]);
}

#[tokio::test]
async fn health_restricted_caller_probes_only_permitted_deployments() {
    let models = vec![
        deployment("a", "id-a", "openai/a"),
        deployment("b", "id-b", "openai/b"),
    ];
    let (mut state, provider, _source) = fixture();
    serve(&mut state, models);
    let reply = get(state, Some(&user(r#"["a"]"#)), "/health").await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(provider.calls(), vec!["openai/a".to_owned()]);
    assert_eq!(reply.count("healthy_count"), 1);
}

#[tokio::test]
async fn health_restricted_caller_sees_no_other_teams_deployment() {
    let models = vec![
        deployment("a", "id-a", "openai/a"),
        deployment_with_info(
            "b",
            "id-b",
            vec![("model", text("openai/b"))],
            vec![("team_id", text("team-2"))],
        ),
    ];
    let (mut state, provider, _source) = fixture();
    serve(&mut state, models);
    let reply = get(state, Some(&team_user("[]", "team-1")), "/health").await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(provider.calls(), vec!["openai/a".to_owned()]);
}

#[tokio::test]
async fn health_admin_with_a_model_grant_is_still_restricted() {
    let models = vec![
        deployment("a", "id-a", "openai/a"),
        deployment("b", "id-b", "openai/b"),
    ];
    let (mut state, _provider, _source) = fixture();
    serve(&mut state, models);
    let caller = key_context(r#"{"user_id":"admin-1","user_role":"proxy_admin","models":["a"]}"#);
    let reply = get(state, Some(&caller), "/health?model_id=id-b").await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);
    assert_eq!(
        reply.detail_error(),
        "key not allowed to health-check model_id id-b"
    );
}

#[tokio::test]
async fn health_restricted_caller_targeting_a_model_is_403() {
    let models = vec![
        deployment("a", "id-a", "openai/a"),
        deployment("b", "id-b", "openai/b"),
    ];
    let (mut state, _provider, _source) = fixture();
    serve(&mut state, models);
    let reply = get(state, Some(&user(r#"["a"]"#)), "/health?model=b&model_id=").await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);
    assert_eq!(
        reply.detail_error(),
        "key not allowed to health-check model b"
    );
}

#[tokio::test]
async fn health_targeted_model_id_with_no_healthy_endpoint_is_503() {
    let models = vec![deployment("a", "id-a", "openai/a")];
    let (mut state, _provider, _source) = fixture();
    let provider = Arc::new(FakeProvider::new().failing("openai/a"));
    state.cycle.provider = Arc::<FakeProvider>::clone(&provider);
    serve(&mut state, models);
    let reply = get(state, Some(&admin()), "/health?model_id=id-a").await;
    assert_eq!(reply.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(reply.count("healthy_count"), 0);
    assert_eq!(reply.count("unhealthy_count"), 1);
    assert_eq!(reply.list("unhealthy_endpoints").len(), 1);
}

#[tokio::test]
async fn health_targeted_model_name_with_no_healthy_endpoint_is_503() {
    let models = vec![deployment("a", "id-a", "openai/a")];
    let (mut state, _provider, _source) = fixture();
    let provider = Arc::new(FakeProvider::new().failing("openai/a"));
    state.cycle.provider = Arc::<FakeProvider>::clone(&provider);
    serve(&mut state, models);
    let reply = get(state, Some(&admin()), "/health?model=a").await;
    assert_eq!(reply.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(reply.count("healthy_count"), 0);
}

#[tokio::test]
async fn health_admin_targeting_an_unknown_model_name_is_503() {
    let models = vec![deployment("a", "id-a", "openai/a")];
    let (mut state, _provider, _source) = fixture();
    serve(&mut state, models);
    let reply = get(state, Some(&admin()), "/health?model=missing").await;
    assert_eq!(reply.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(reply.count("healthy_count"), 0);
    assert_eq!(reply.count("unhealthy_count"), 0);
}

#[tokio::test]
async fn health_background_untargeted_admin_gets_the_cache_untouched() {
    let models = vec![
        deployment("a", "id-a", "openai/a"),
        deployment("b", "id-b", "openai/b"),
    ];
    let (mut state, provider, _source) = fixture();
    serve(&mut state, models);
    background(
        &mut state,
        vec![endpoint("a", "id-a"), endpoint("b", "id-b")],
    );
    let reply = get(state, Some(&admin()), "/health").await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.count("healthy_count"), 2);
    assert!(provider.calls().is_empty());
    let healthy = reply.list("healthy_endpoints");
    assert_eq!(text_field(&healthy[0], "model_id"), Some("id-a".to_owned()));
}

#[tokio::test]
async fn health_background_targeted_admin_filters_the_cache() {
    let models = vec![
        deployment("a", "id-a", "openai/a"),
        deployment("b", "id-b", "openai/b"),
    ];
    let (mut state, _provider, _source) = fixture();
    serve(&mut state, models);
    background(
        &mut state,
        vec![endpoint("a", "id-a"), endpoint("b", "id-b")],
    );
    let reply = get(state, Some(&admin()), "/health?model_id=id-a").await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.count("healthy_count"), 1);
    let healthy = reply.list("healthy_endpoints");
    assert_eq!(healthy.len(), 1);
    assert_eq!(text_field(&healthy[0], "model_id"), Some("id-a".to_owned()));
}

#[tokio::test]
async fn health_background_restricted_caller_sees_only_allowed_ids() {
    let models = vec![
        deployment("a", "id-a", "openai/a"),
        deployment("b", "id-b", "openai/b"),
    ];
    let (mut state, provider, _source) = fixture();
    serve(&mut state, models);
    background(
        &mut state,
        vec![endpoint("a", "id-a"), endpoint("b", "id-b")],
    );
    let reply = get(state, Some(&user(r#"["a"]"#)), "/health").await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.count("healthy_count"), 1);
    assert!(provider.calls().is_empty());
    let healthy = reply.list("healthy_endpoints");
    assert_eq!(healthy.len(), 1);
    assert_eq!(text_field(&healthy[0], "api_base"), None);
    assert!(reply.header("litellm-health-field-notice").is_some());
    assert_eq!(reply.body.get("warnings"), None);
}

#[tokio::test]
async fn health_background_restricted_caller_targets_one_allowed_model() {
    let models = vec![
        deployment("a", "id-a", "openai/a"),
        deployment("b", "id-b", "openai/b"),
    ];
    let (mut state, _provider, _source) = fixture();
    serve(&mut state, models);
    background(
        &mut state,
        vec![endpoint("a", "id-a"), endpoint("b", "id-b")],
    );
    let reply = get(state, Some(&user(r#"["a","b"]"#)), "/health?model_id=id-b").await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.count("healthy_count"), 1);
    let healthy = reply.list("healthy_endpoints");
    assert_eq!(text_field(&healthy[0], "model_id"), Some("id-b".to_owned()));
}

#[tokio::test]
async fn health_background_missing_ids_warns_and_adds_the_field() {
    let (buffer, _guard) = capture_logs();
    let models = vec![deployment_without_id("a", "openai/a")];
    let (mut state, _provider, _source) = fixture();
    serve(&mut state, models);
    background(&mut state, vec![endpoint("a", "id-a")]);
    let reply = get(state, Some(&user(r#"["a"]"#)), "/health").await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.count("healthy_count"), 0);
    match reply.body.get("warnings") {
        Some(Json::List(items)) => assert_eq!(items.len(), 1),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_))
        | None => panic!("warnings is a list"),
    }
    let logs = buffer.text();
    assert!(
        logs.contains("health_endpoint: scoped key user-1 has accessible models [a]"),
        "{logs}"
    );
}

#[tokio::test]
async fn health_background_restricted_caller_with_no_kept_deployments_gets_no_warning() {
    let models = vec![deployment_with_info(
        "b",
        "id-b",
        vec![("model", text("openai/b"))],
        vec![("team_id", text("team-2"))],
    )];
    let (mut state, _provider, _source) = fixture();
    serve(&mut state, models);
    background(&mut state, vec![endpoint("b", "id-b")]);
    let reply = get(state, Some(&team_user("[]", "team-1")), "/health").await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.count("healthy_count"), 0);
    assert_eq!(reply.body.get("warnings"), None);
}

#[tokio::test]
async fn health_router_lookup_failure_is_500_and_logged() {
    let (buffer, _guard) = capture_logs();
    let models = vec![deployment("a", "id-a", "openai/a")];
    let (mut state, _provider, _source) = fixture();
    state.deployments = Some(Arc::new(models));
    state.router = Some(failing_router());
    let reply = get(state, Some(&user(r#"["a"]"#)), "/health").await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(
        reply.detail_error().contains("the router lookup failed"),
        "{}",
        reply.detail_error()
    );
    let logs = buffer.text();
    assert!(
        logs.contains("Exception occu\u{72}ed - the router lookup failed"),
        "{logs}"
    );
}

#[tokio::test]
async fn history_returns_the_page() {
    let mut database = open_database().await;
    insert_rows(
        &database,
        &[
            ("hc-1", "group-a", None, "healthy", "2026-03-15 09:00:00"),
            ("hc-2", "group-b", None, "healthy", "2026-03-15 10:00:00"),
        ],
    )
    .await;
    database.release();
    let (mut state, _provider, _source) = fixture();
    state.rows = Some(Arc::new(database.rows()));
    let reply = get(state, Some(&admin()), "/health/history").await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.count("total_records"), 2);
    assert_eq!(reply.count("limit"), 100);
    assert_eq!(reply.count("offset"), 0);
    let checks = reply.list("health_checks");
    assert_eq!(checks.len(), 2);
    assert_eq!(
        text_field(&checks[0], "model_name"),
        Some("group-b".to_owned())
    );
    assert_eq!(
        text_field(&checks[0], "checked_at"),
        Some("2026-03-15T10:00:00+00:00".to_owned())
    );
    database.finish().await;
}

#[tokio::test]
async fn history_honours_limit_and_offset() {
    let mut database = open_database().await;
    insert_rows(
        &database,
        &[
            ("hc-1", "group-a", None, "healthy", "2026-03-15 09:00:00"),
            ("hc-2", "group-a", None, "healthy", "2026-03-15 10:00:00"),
            ("hc-3", "group-a", None, "healthy", "2026-03-15 11:00:00"),
        ],
    )
    .await;
    database.release();
    let (mut state, _provider, _source) = fixture();
    state.rows = Some(Arc::new(database.rows()));
    let reply = get(state, Some(&admin()), "/health/history?limit=1&offset=1").await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.count("total_records"), 1);
    assert_eq!(reply.count("limit"), 1);
    assert_eq!(reply.count("offset"), 1);
    let checks = reply.list("health_checks");
    assert_eq!(checks.len(), 1);
    assert_eq!(
        text_field(&checks[0], "health_check_id"),
        Some("hc-2".to_owned())
    );
    database.finish().await;
}

#[tokio::test]
async fn history_filters_by_model_and_status() {
    let mut database = open_database().await;
    insert_rows(
        &database,
        &[
            ("hc-1", "group-a", None, "healthy", "2026-03-15 09:00:00"),
            ("hc-2", "group-a", None, "unhealthy", "2026-03-15 10:00:00"),
            ("hc-3", "group-b", None, "unhealthy", "2026-03-15 11:00:00"),
        ],
    )
    .await;
    database.release();
    let (mut state, _provider, _source) = fixture();
    state.rows = Some(Arc::new(database.rows()));
    let reply = get(
        state,
        Some(&admin()),
        "/health/history?model=group-a&status_filter=unhealthy",
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    let checks = reply.list("health_checks");
    assert_eq!(checks.len(), 1);
    assert_eq!(
        text_field(&checks[0], "health_check_id"),
        Some("hc-2".to_owned())
    );
    database.finish().await;
}

const GE_LIMIT_ONE: &str = "Input should be greater than or equal to 1";
const LE_LIMIT_MAX: &str = "Input should be less than or equal to 1000";
const NOT_INTEGER: &str = "Input should be a valid integer, unable to parse string as an integer";
const GE_OFFSET_ZERO: &str = "Input should be greater than or equal to 0";

#[rstest]
#[case::limit_zero("limit=0", "greater_than_equal", "limit", GE_LIMIT_ONE)]
#[case::limit_high("limit=1001", "less_than_equal", "limit", LE_LIMIT_MAX)]
#[case::limit_text("limit=many", "int_parsing", "limit", NOT_INTEGER)]
#[case::limit_past_i64("limit=99999999999999999999", "less_than_equal", "limit", LE_LIMIT_MAX)]
#[case::limit_neg(
    "limit=-99999999999999999999",
    "greater_than_equal",
    "limit",
    GE_LIMIT_ONE
)]
#[case::offset_negative("offset=-1", "greater_than_equal", "offset", GE_OFFSET_ZERO)]
#[case::offset_neg(
    "offset=-99999999999999999999",
    "greater_than_equal",
    "offset",
    GE_OFFSET_ZERO
)]
#[tokio::test]
async fn history_invalid_bounds_are_422(
    #[case] query: &str,
    #[case] kind: &str,
    #[case] field: &str,
    #[case] message: &str,
) {
    let (state, _provider, _source) = fixture();
    let uri = format!("/health/history?{query}");
    let reply = get(state, Some(&admin()), &uri).await;
    assert_issue(&reply, &uri, kind, field, message);
}

#[rstest]
#[case::minimum("limit=1", 1)]
#[case::maximum("limit=1000&offset=0", 1000)]
#[tokio::test]
async fn history_limit_boundaries_are_accepted(#[case] query: &str, #[case] limit: i64) {
    let (mut state, _provider, _source) = fixture();
    state.rows = Some(Arc::new(unconnected()));
    let uri = format!("/health/history?{query}");
    let reply = get(state, Some(&admin()), &uri).await;
    assert_eq!(reply.status, StatusCode::OK, "{uri}");
    assert_eq!(reply.count("limit"), limit, "{uri}");
    assert_eq!(reply.count("offset"), 0, "{uri}");
}

#[tokio::test]
async fn history_huge_offset_is_an_empty_page() {
    let (mut state, _provider, _source) = fixture();
    state.rows = Some(Arc::new(unconnected()));
    let reply = get(
        state,
        Some(&admin()),
        "/health/history?offset=99999999999999999999",
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.count("offset"), i64::MAX);
    assert_eq!(reply.count("total_records"), 0);
}

#[tokio::test]
async fn history_failed_read_is_an_empty_page() {
    let (buffer, _guard) = capture_logs();
    let (mut state, _provider, _source) = fixture();
    state.rows = Some(Arc::new(unconnected()));
    let reply = get(state, Some(&admin()), "/health/history").await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.count("total_records"), 0);
    assert!(reply.list("health_checks").is_empty());
    let logs = buffer.text();
    assert!(
        logs.contains("Error getting health check history: "),
        "{logs}"
    );
}

#[tokio::test]
async fn latest_returns_the_latest_row_per_model() {
    let mut database = open_database().await;
    insert_rows(
        &database,
        &[
            ("hc-1", "group-a", None, "healthy", "2026-03-15 09:00:00"),
            ("hc-2", "group-a", None, "healthy", "2026-03-15 10:00:00"),
            ("hc-3", "group-b", None, "healthy", "2026-03-15 11:00:00"),
        ],
    )
    .await;
    database.release();
    let (mut state, _provider, _source) = fixture();
    state.rows = Some(Arc::new(database.rows()));
    let reply = get(state, Some(&admin()), "/health/latest").await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.count("total_models"), 2);
    let checks = latest_checks(&reply);
    assert_eq!(checks.len(), 2);
    match checks.get("group-a") {
        Some(Json::Object(fields)) => {
            assert_eq!(
                text_field(fields, "health_check_id"),
                Some("hc-2".to_owned())
            );
            assert_eq!(
                text_field(fields, "checked_at"),
                Some("2026-03-15T10:00:00+00:00".to_owned())
            );
        }
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_))
        | None => panic!("the entry is an object"),
    }
    database.finish().await;
}

#[tokio::test]
async fn latest_prints_the_converted_row() {
    let mut database = open_database().await;
    insert_rows(
        &database,
        &[(
            "hc-1",
            "group-a",
            Some("id-a"),
            "healthy",
            "2026-03-15 09:00:00",
        )],
    )
    .await;
    database.release();
    let (mut state, _provider, _source) = fixture();
    state.rows = Some(Arc::new(database.rows()));
    let reply = get(state, Some(&admin()), "/health/latest").await;
    let checks = latest_checks(&reply);
    match checks.get("id-a") {
        Some(Json::Object(fields)) => {
            let keys: Vec<&str> = fields.keys().map(String::as_str).collect();
            assert_eq!(keys.len(), 12);
            assert_eq!(keys[0], "health_check_id");
            assert_eq!(keys[11], "created_at");
            assert_eq!(
                text_field(fields, "checked_at"),
                Some("2026-03-15T09:00:00+00:00".to_owned())
            );
        }
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_))
        | None => panic!("the entry is an object"),
    }
    database.finish().await;
}

#[tokio::test]
async fn latest_keys_by_model_id_else_model_name() {
    let mut database = open_database().await;
    insert_rows(
        &database,
        &[
            (
                "hc-1",
                "group-a",
                Some("id-a"),
                "healthy",
                "2026-03-15 09:00:00",
            ),
            ("hc-2", "group-b", None, "healthy", "2026-03-15 10:00:00"),
        ],
    )
    .await;
    database.release();
    let (mut state, _provider, _source) = fixture();
    state.rows = Some(Arc::new(database.rows()));
    let reply = get(state, Some(&admin()), "/health/latest").await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.count("total_models"), 2);
    let checks = latest_checks(&reply);
    assert!(checks.contains_key("id-a"));
    assert!(checks.contains_key("group-b"));
    database.finish().await;
}

#[tokio::test]
async fn latest_collapses_rows_that_share_a_key() {
    let mut database = open_database().await;
    insert_rows(
        &database,
        &[
            (
                "hc-1",
                "group-a",
                Some(""),
                "healthy",
                "2026-03-15 09:00:00",
            ),
            ("hc-2", "group-a", None, "healthy", "2026-03-15 10:00:00"),
        ],
    )
    .await;
    database.release();
    let (mut state, _provider, _source) = fixture();
    state.rows = Some(Arc::new(database.rows()));
    let reply = get(state, Some(&admin()), "/health/latest").await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.count("total_models"), 1);
    let checks = latest_checks(&reply);
    match checks.get("group-a") {
        Some(Json::Object(fields)) => {
            assert_eq!(
                text_field(fields, "health_check_id"),
                Some("hc-2".to_owned())
            );
        }
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_))
        | None => panic!("the entry is an object"),
    }
    database.finish().await;
}

#[tokio::test]
async fn latest_failed_read_is_an_empty_object() {
    let (buffer, _guard) = capture_logs();
    let (mut state, _provider, _source) = fixture();
    state.rows = Some(Arc::new(unconnected()));
    let reply = get(state, Some(&admin()), "/health/latest").await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.count("total_models"), 0);
    assert!(latest_checks(&reply).is_empty());
    let logs = buffer.text();
    assert!(
        logs.contains("Error getting all latest health checks: "),
        "{logs}"
    );
}
