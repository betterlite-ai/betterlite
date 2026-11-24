use axum::http::StatusCode;
use betterlite_auth::auth_identity::roles::Role;
use betterlite_db::budget_permission_rows::JsonData;

use super::support::{TestApp, admin_caller, assert_number, key_caller, role_only_caller};

const DEFAULT_BODY: &str = r#"{"organization_alias":"acme"}"#;

#[tokio::test]
async fn user_lookup_failure_is_silent() {
    let app = TestApp::start().await;
    app.database.drop_user_table().await;
    let reply = app.post(admin_caller(Some("u1")), DEFAULT_BODY).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(app.database.organization_count().await, 1);
    app.close().await;
}

#[tokio::test]
async fn empty_request_budget_created() {
    let app = TestApp::start().await;
    let reply = app.post(admin_caller(Some("u1")), DEFAULT_BODY).await;
    assert_eq!(reply.status, StatusCode::OK);
    let budget_id = reply.text_field("budget_id");
    assert_eq!(app.database.budget_count().await, 1);
    let budget = app.database.budget_row(&budget_id).await.unwrap();
    assert_eq!(
        budget.get("created_by"),
        Some(&JsonData::String(String::from("u1")))
    );
    assert_eq!(
        budget.get("updated_by"),
        Some(&JsonData::String(String::from("u1")))
    );
    let organization = app
        .database
        .organization_row(&reply.text_field("organization_id"))
        .await
        .unwrap();
    assert_eq!(
        organization.get("budget_id"),
        Some(&JsonData::String(budget_id))
    );
    app.close().await;
}

#[tokio::test]
async fn explicit_budget_bypasses_create() {
    let app = TestApp::start().await;
    app.database.seed_budget("seeded-budget").await;
    let reply = app
        .post(
            admin_caller(Some("u1")),
            r#"{"organization_alias":"acme","budget_id":"seeded-budget","max_budget":99}"#,
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.text_field("budget_id"), "seeded-budget");
    // the handler never creates, checks or updates the sent budget row
    assert_eq!(app.database.budget_count().await, 1);
    let seeded = app.database.budget_row("seeded-budget").await.unwrap();
    assert_number(&seeded["max_budget"], 25.0_f64);
    let JsonData::Object(table) = reply.field("litellm_budget_table") else {
        panic!("the sent budget row is attached to the response");
    };
    assert_eq!(
        table.get("budget_id"),
        Some(&JsonData::String(String::from("seeded-budget")))
    );

    // an empty id is a present id: no budget is created, and the dangling reference fails the foreign key
    let empty = app
        .post(
            admin_caller(Some("u1")),
            r#"{"organization_alias":"empty","budget_id":""}"#,
        )
        .await;
    assert_eq!(empty.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(empty.error_text("message"), "Internal server error");
    assert_eq!(app.database.budget_count().await, 1);
    assert_eq!(app.database.organization_count().await, 1);
    app.close().await;
}

#[tokio::test]
async fn budget_actor_fallback() {
    let app = TestApp::start().await;
    let keyed = app
        .post(
            key_caller(Some("u1"), Some("proxy_admin"), &[]),
            DEFAULT_BODY,
        )
        .await;
    assert_eq!(keyed.status, StatusCode::OK);
    let keyed_budget = app
        .database
        .budget_row(&keyed.text_field("budget_id"))
        .await
        .unwrap();
    assert_eq!(
        keyed_budget.get("created_by"),
        Some(&JsonData::String(String::from("u1")))
    );

    // a proxy admin with no user id writes litellm_proxy_admin_name into both actor columns
    let admin = role_only_caller(&[Role::PlatformAdmin]);
    let fallback = app.post(admin, DEFAULT_BODY).await;
    assert_eq!(fallback.status, StatusCode::OK);
    assert_eq!(fallback.text_field("created_by"), "default_user_id");
    let fallback_budget = app
        .database
        .budget_row(&fallback.text_field("budget_id"))
        .await
        .unwrap();
    assert_eq!(
        fallback_budget.get("created_by"),
        Some(&JsonData::String(String::from("default_user_id")))
    );
    assert_eq!(
        fallback_budget.get("updated_by"),
        Some(&JsonData::String(String::from("default_user_id")))
    );
    app.close().await;
}

#[tokio::test]
async fn permission_none_and_empty() {
    let app = TestApp::start().await;
    let absent = app.post(admin_caller(Some("u1")), DEFAULT_BODY).await;
    assert_eq!(absent.status, StatusCode::OK);
    assert_eq!(absent.field("object_permission_id"), JsonData::Null);
    assert_eq!(app.database.permission_count().await, 0);

    assert_empty_permission_stored(&app).await;
    assert_null_permission_dropped(&app).await;
    app.close().await;
}

// the docstring claims an empty object means no permission; the handler stores a row for it
async fn assert_empty_permission_stored(app: &TestApp) {
    let empty = app
        .post(
            admin_caller(Some("u1")),
            r#"{"organization_alias":"acme","object_permission":{}}"#,
        )
        .await;
    assert_eq!(empty.status, StatusCode::OK);
    assert_eq!(app.database.permission_count().await, 1);
    let permission_id = empty.text_field("object_permission_id");
    uuid::Uuid::parse_str(&permission_id).unwrap();
    let permission = app.database.permission_row(&permission_id).await.unwrap();
    assert_eq!(
        permission.get("vector_stores"),
        Some(&JsonData::Array(Vec::new()))
    );
    let organization = app
        .database
        .organization_row(&empty.text_field("organization_id"))
        .await
        .unwrap();
    assert_eq!(
        organization.get("object_permission_id"),
        Some(&JsonData::String(permission_id))
    );
}

// an explicit null inside object_permission is dropped like exclude_none, so the column default applies
async fn assert_null_permission_dropped(app: &TestApp) {
    let nulled = app
        .post(
            admin_caller(Some("u1")),
            r#"{"organization_alias":"nulled","object_permission":{"vector_stores":null}}"#,
        )
        .await;
    assert_eq!(nulled.status, StatusCode::OK);
    let nulled_permission = app
        .database
        .permission_row(&nulled.text_field("object_permission_id"))
        .await
        .unwrap();
    assert_eq!(
        nulled_permission.get("vector_stores"),
        Some(&JsonData::Array(Vec::new()))
    );
}

#[tokio::test]
async fn ambiguous_permission_stops_after_budget() {
    let app = TestApp::start().await;
    app.database
        .seed_mcp_server("s1", "shared", "server-one")
        .await;
    app.database
        .seed_mcp_server("s2", "shared", "server-two")
        .await;
    let reply = app
        .post(
            admin_caller(Some("u1")),
            r#"{"organization_alias":"acme","object_permission":{"mcp_tool_permissions":{"shared":["tool"]}}}"#,
        )
        .await;
    assert_eq!(reply.status, StatusCode::BAD_REQUEST);
    let message = reply.detail_error();
    assert!(
        message.starts_with("Ambiguous mcp_tool_permissions key: 'shared' matches MCP servers"),
        "{message}"
    );
    assert!(
        message.contains("s1") && message.contains("s2"),
        "{message}"
    );
    // the budget row was written before the permission check and is not rolled back
    assert_eq!(app.database.budget_count().await, 1);
    assert_eq!(app.database.permission_count().await, 0);
    assert_eq!(app.database.organization_count().await, 0);
    app.close().await;
}

#[tokio::test]
async fn permission_removed_from_org_payload() {
    let app = TestApp::start().await;
    let reply = app
        .post(
            admin_caller(Some("u1")),
            r#"{"organization_alias":"acme","object_permission":{"vector_stores":["v1"],"mcp_servers":["m1"]}}"#,
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK);
    let permission_id = reply.text_field("object_permission_id");
    let permission = app.database.permission_row(&permission_id).await.unwrap();
    assert_eq!(
        permission.get("vector_stores"),
        Some(&JsonData::Array(vec![JsonData::String(String::from("v1"))]))
    );
    assert_eq!(
        permission.get("mcp_servers"),
        Some(&JsonData::Array(vec![JsonData::String(String::from("m1"))]))
    );
    // the request field is removed once the row exists, so no organization column carries it
    let organization = app
        .database
        .organization_row(&reply.text_field("organization_id"))
        .await
        .unwrap();
    assert!(!organization.contains_key("object_permission"));
    assert_eq!(
        organization.get("object_permission_id"),
        Some(&JsonData::String(permission_id))
    );
    assert_eq!(reply.field("object_permission"), JsonData::Null);
    app.close().await;
}
