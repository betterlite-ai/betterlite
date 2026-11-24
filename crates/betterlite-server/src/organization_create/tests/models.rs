use std::sync::Arc;

use axum::http::StatusCode;

use super::support::{RecordingView, TestApp, admin_caller, key_caller, parsed};
use crate::organization_create::error::ALL_MODELS_DENIED;

#[tokio::test]
async fn unrestricted_caller_skips_model_checks() {
    let view = Arc::new(RecordingView::default());
    let probe = Arc::clone(&view);
    let app = TestApp::start_with(|state| state.view = view).await;
    let reply = app
        .post(
            key_caller(Some("u1"), Some("proxy_admin"), &[]),
            r#"{"organization_alias":"acme","models":["zeta","alpha"]}"#,
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert!(
        probe.calls().is_empty(),
        "a caller with no model list never reaches the router"
    );
    let organization = app
        .database
        .organization_row(&reply.text_field("organization_id"))
        .await
        .unwrap();
    assert_eq!(
        organization.get("models"),
        Some(&parsed(r#"["zeta","alpha"]"#))
    );
    app.close().await;
}

#[tokio::test]
async fn restricted_caller_all_models_denied() {
    let view = Arc::new(RecordingView::default());
    let probe = Arc::clone(&view);
    let app = TestApp::start_with(|state| state.view = view).await;
    let reply = app
        .post(
            key_caller(Some("u1"), Some("proxy_admin"), &["gpt-4"]),
            r#"{"organization_alias":"acme"}"#,
        )
        .await;
    assert_eq!(reply.status, StatusCode::BAD_REQUEST);
    assert_eq!(reply.detail_error(), ALL_MODELS_DENIED);
    // the budget row was written before the model checks ran
    assert_eq!(app.database.budget_count().await, 1);
    assert_eq!(app.database.organization_count().await, 0);
    assert!(probe.calls().is_empty());
    app.close().await;
}

#[tokio::test]
async fn model_check_inputs_order() {
    let view = Arc::new(RecordingView::default());
    let probe = Arc::clone(&view);
    let app = TestApp::start_with(|state| state.view = view).await;
    app.database.seed_user("u1", None, &["alpha", "zeta"]).await;
    let caller = || key_caller(Some("u1"), Some("proxy_admin"), &["zeta"]);

    let allowed = app
        .post(
            caller(),
            r#"{"organization_alias":"acme","models":["zeta","alpha"]}"#,
        )
        .await;
    assert_eq!(allowed.status, StatusCode::OK);
    assert_eq!(
        probe.calls(),
        vec![String::from("zeta"), String::from("alpha")]
    );
    let organization = app
        .database
        .organization_row(&allowed.text_field("organization_id"))
        .await
        .unwrap();
    assert_eq!(
        organization.get("models"),
        Some(&parsed(r#"["zeta","alpha"]"#))
    );

    // the request's order decides the checks, and the first refusal ends them
    let denied = app
        .post(
            caller(),
            r#"{"organization_alias":"other","models":["beta","zeta"]}"#,
        )
        .await;
    assert_eq!(denied.status, StatusCode::FORBIDDEN);
    assert_eq!(
        probe.calls(),
        vec![
            String::from("zeta"),
            String::from("alpha"),
            String::from("beta")
        ]
    );
    assert!(
        denied
            .error_text("message")
            .contains("The requested model 'beta' is not available"),
        "{body}",
        body = denied.body
    );
    assert_eq!(denied.error_text("type"), "user_model_access_denied");
    assert_eq!(app.database.organization_count().await, 1);
    app.close().await;
}

#[tokio::test]
async fn budget_survives_permission_failure() {
    let app = TestApp::start().await;
    app.database.drop_permission_table().await;
    let reply = app
        .post(
            admin_caller(Some("u1")),
            r#"{"organization_alias":"acme","object_permission":{}}"#,
        )
        .await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(reply.error_text("message"), "Internal server error");
    assert_eq!(reply.error_text("type"), "internal_server_error");
    assert_eq!(app.database.budget_count().await, 1);
    assert_eq!(app.database.organization_count().await, 0);
    app.close().await;
}

#[tokio::test]
async fn permission_survives_model_denial() {
    let app = TestApp::start().await;
    app.database.seed_user("u1", None, &["alpha"]).await;
    let reply = app
        .post(
            key_caller(Some("u1"), Some("proxy_admin"), &["zeta"]),
            r#"{"organization_alias":"acme","models":["beta"],"object_permission":{"vector_stores":["v1"]}}"#,
        )
        .await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);
    assert!(
        reply
            .error_text("message")
            .contains("The requested model 'beta' is not available"),
        "{body}",
        body = reply.body
    );
    assert_eq!(app.database.permission_count().await, 1);
    assert_eq!(app.database.budget_count().await, 1);
    assert_eq!(app.database.organization_count().await, 0);
    app.close().await;
}
