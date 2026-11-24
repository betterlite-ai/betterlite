use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_error::proxy_error_types::common::CommonProxyErrors;
use rstest::rstest;
use tower::ServiceExt as _;

use super::support::{Reply, TestApp, admin_caller, key_caller, master_caller, number, parsed};
use crate::routes::router;
use crate::routes::tests::{empty_config, state_for};

const DEFAULT_BODY: &str = r#"{"organization_alias":"acme"}"#;
const DB_NOT_CONNECTED: &str = CommonProxyErrors::DbNotConnectedError.as_str();

fn assert_created_defaults(reply: &Reply) {
    assert_eq!(reply.field("models"), JsonData::Array(Vec::new()));
    assert_eq!(reply.field("metadata"), JsonData::Null);
    assert_eq!(reply.field("object_permission"), JsonData::Null);
    assert_eq!(reply.field("object_permission_id"), JsonData::Null);
    assert_eq!(reply.field("users"), JsonData::Null);
    assert!(matches!(reply.field("spend"), JsonData::Number(_)));
    assert!(matches!(reply.field("model_spend"), JsonData::Object(_)));
    assert!(matches!(reply.field("created_at"), JsonData::String(_)));
    assert!(matches!(reply.field("updated_at"), JsonData::String(_)));
}

#[tokio::test]
async fn create_request_defaults() {
    let app = TestApp::start().await;
    let reply = app.post(master_caller(), DEFAULT_BODY).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_created_defaults(&reply);
    let organization_id = reply.text_field("organization_id");
    uuid::Uuid::parse_str(&organization_id).unwrap();
    let budget_id = reply.text_field("budget_id");
    uuid::Uuid::parse_str(&budget_id).unwrap();
    assert_eq!(reply.text_field("organization_alias"), "acme");
    assert_eq!(reply.text_field("created_by"), "default_user_id");
    assert_eq!(reply.text_field("updated_by"), "default_user_id");

    let budget = app.database.budget_row(&budget_id).await.unwrap();
    assert_eq!(budget.get("max_budget"), Some(&JsonData::Null));
    assert_eq!(budget.get("soft_budget"), Some(&JsonData::Null));
    assert_eq!(
        budget.get("created_by"),
        Some(&JsonData::String(String::from("default_user_id")))
    );
    assert_eq!(
        budget.get("allowed_models"),
        Some(&JsonData::Array(Vec::new()))
    );

    let JsonData::Object(table) = reply.field("litellm_budget_table") else {
        panic!("the created budget table is nested in the response");
    };
    assert_eq!(
        table.get("budget_id"),
        Some(&JsonData::String(budget_id.clone()))
    );
    assert_eq!(table.get("max_budget"), Some(&JsonData::Null));
    assert_eq!(
        table.get("allowed_models"),
        Some(&JsonData::Array(Vec::new()))
    );

    let organization = app
        .database
        .organization_row(&organization_id)
        .await
        .unwrap();
    assert_eq!(
        organization.get("budget_id"),
        Some(&JsonData::String(budget_id))
    );
    app.close().await;
}

#[tokio::test]
async fn inherited_budget_fields_preserved() {
    let app = TestApp::start().await;
    let body = r#"{
        "organization_alias": "acme",
        "models": ["gpt-4"],
        "max_budget": 12.5,
        "soft_budget": 3,
        "max_parallel_requests": 4,
        "tpm_limit": 5,
        "rpm_limit": 6,
        "tpd_limit": 7,
        "model_max_budget": {"gpt-4": 9},
        "budget_duration": "1d",
        "allowed_models": ["gpt-4"],
        "temp_budget_increase": 2,
        "temp_budget_expiry": "2026-01-02T03:04:05+00:00"
    }"#;
    let reply = app.post(admin_caller(Some("u1")), body).await;
    assert_eq!(reply.status, StatusCode::OK);
    let budget_id = reply.text_field("budget_id");
    let budget = app.database.budget_row(&budget_id).await.unwrap();
    assert_eq!(number(budget.get("max_budget").unwrap()), 12.5);
    assert_eq!(number(budget.get("soft_budget").unwrap()), 3.0);
    assert_eq!(number(budget.get("max_parallel_requests").unwrap()), 4.0);
    assert_eq!(number(budget.get("tpm_limit").unwrap()), 5.0);
    assert_eq!(number(budget.get("rpm_limit").unwrap()), 6.0);
    assert_eq!(number(budget.get("tpd_limit").unwrap()), 7.0);
    assert_eq!(number(budget.get("temp_budget_increase").unwrap()), 2.0);
    assert_eq!(
        budget.get("budget_duration"),
        Some(&JsonData::String(String::from("1d")))
    );
    assert_eq!(
        budget.get("allowed_models"),
        Some(&JsonData::Array(vec![JsonData::String(String::from(
            "gpt-4"
        ))]))
    );
    // the jsonified mapping columns are stored as JSON text, like every LiteLLM Prisma Json write
    let Some(JsonData::String(model_max_budget)) = budget.get("model_max_budget") else {
        panic!("model_max_budget is jsonified into the row");
    };
    assert_eq!(
        serde_json::from_str::<JsonData>(model_max_budget).unwrap(),
        parsed(r#"{"gpt-4":9}"#)
    );

    let JsonData::Object(table) = reply.field("litellm_budget_table") else {
        panic!("the created budget table is nested in the response");
    };
    assert_eq!(
        table.get("model_max_budget"),
        Some(&parsed(r#"{"gpt-4":9}"#))
    );
    assert_eq!(
        table.get("budget_duration"),
        Some(&JsonData::String(String::from("1d")))
    );
    // the nested table echoes the request's money and limit columns, not only the stored row
    assert_eq!(number(table.get("max_budget").unwrap()), 12.5);
    assert_eq!(number(table.get("soft_budget").unwrap()), 3.0);
    assert_eq!(number(table.get("temp_budget_increase").unwrap()), 2.0);
    assert_eq!(table.get("max_parallel_requests"), Some(&parsed("4")));
    assert_eq!(table.get("tpm_limit"), Some(&parsed("5")));
    app.close().await;
}

enum GuardCase {
    DatabaseMissing,
    RoleBeforeRouter,
    RouterBeforeBudget,
    BudgetAfterRouter,
}

#[rstest]
#[case::database_missing_before_role(GuardCase::DatabaseMissing, 500, DB_NOT_CONNECTED)]
#[case::role_before_router(
    GuardCase::RoleBeforeRouter,
    401,
    "Only admins can create orgs. Your role is = internal_user"
)]
#[case::router_before_budget(GuardCase::RouterBeforeBudget, 500, "No models configured on proxy")]
#[case::budget_after_router(
    GuardCase::BudgetAfterRouter,
    400,
    "max_budget must be a non-negative finite number. Received: -1"
)]
#[tokio::test]
async fn create_guard_order(#[case] case: GuardCase, #[case] status: u16, #[case] message: &str) {
    let (app, caller, body) = match case {
        GuardCase::DatabaseMissing => {
            let app = TestApp::start_with(|state| state.database = None).await;
            (app, master_caller(), String::from(DEFAULT_BODY))
        }
        GuardCase::RoleBeforeRouter => {
            let app = TestApp::start_with(|state| state.router_configured = false).await;
            (
                app,
                key_caller(Some("u1"), Some("internal_user"), &[]),
                String::from(DEFAULT_BODY),
            )
        }
        GuardCase::RouterBeforeBudget => {
            let app = TestApp::start_with(|state| state.router_configured = false).await;
            (
                app,
                master_caller(),
                String::from(r#"{"organization_alias":"acme","max_budget":-1}"#),
            )
        }
        GuardCase::BudgetAfterRouter => {
            let app = TestApp::start().await;
            (
                app,
                master_caller(),
                String::from(r#"{"organization_alias":"acme","max_budget":-1}"#),
            )
        }
    };
    let reply = app.post(caller, &body).await;
    assert_eq!(reply.status, StatusCode::from_u16(status).unwrap());
    assert_eq!(reply.detail_error(), message);
    app.close().await;
}

#[rstest]
#[case::proxy_admin(Some("proxy_admin"), 200, "")]
#[case::proxy_admin_viewer(
    Some("proxy_admin_viewer"),
    401,
    "Only admins can create orgs. Your role is = proxy_admin_viewer"
)]
#[case::internal_user(
    Some("internal_user"),
    401,
    "Only admins can create orgs. Your role is = internal_user"
)]
#[case::org_admin(
    Some("org_admin"),
    401,
    "Only admins can create orgs. Your role is = org_admin"
)]
#[case::missing_role(None, 401, "Only admins can create orgs. Your role is = None")]
#[tokio::test]
async fn create_role_matrix(
    #[case] role: Option<&str>,
    #[case] status: u16,
    #[case] message: &str,
) {
    let app = TestApp::start().await;
    let caller = key_caller(Some("u1"), role, &[]);
    let reply = app.post(caller, DEFAULT_BODY).await;
    assert_eq!(reply.status, StatusCode::from_u16(status).unwrap());
    if status == 200 {
        assert_eq!(reply.text_field("organization_alias"), "acme");
    } else {
        assert_eq!(reply.detail_error(), message);
        assert_eq!(app.database.organization_count().await, 0);
    }
    app.close().await;
}

#[rstest]
#[case::max_budget_negative(
    r#"{"organization_alias":"a","max_budget":-1}"#,
    400,
    "max_budget must be a non-negative finite number. Received: -1"
)]
#[case::max_budget_zero(r#"{"organization_alias":"a","max_budget":0}"#, 200, "")]
#[case::max_budget_infinite(
    r#"{"organization_alias":"a","max_budget":1e400}"#,
    400,
    "max_budget must be a non-negative finite number. Received: inf"
)]
#[case::max_budget_negative_infinite(
    r#"{"organization_alias":"a","max_budget":-1e400}"#,
    400,
    "max_budget must be a non-negative finite number. Received: -inf"
)]
#[case::soft_budget_negative(
    r#"{"organization_alias":"a","soft_budget":-2.5}"#,
    400,
    "soft_budget must be a non-negative finite number. Received: -2.5"
)]
#[case::soft_budget_zero(r#"{"organization_alias":"a","soft_budget":0}"#, 200, "")]
#[case::max_budget_out_of_range(
    r#"{"organization_alias":"a","max_budget":1e30}"#,
    400,
    "max_budget must be a number the budget table can store. Received: 1e30"
)]
#[case::temp_budget_out_of_range(
    r#"{"organization_alias":"a","temp_budget_increase":1e30}"#,
    400,
    "temp_budget_increase must be a number the budget table can store. Received: 1e30"
)]
#[case::max_checked_before_soft(
    r#"{"organization_alias":"a","max_budget":-1,"soft_budget":-1}"#,
    400,
    "max_budget must be a non-negative finite number. Received: -1"
)]
#[tokio::test]
async fn create_budget_number_matrix(
    #[case] body: &str,
    #[case] status: u16,
    #[case] message: &str,
) {
    let app = TestApp::start().await;
    let reply = app.post(master_caller(), body).await;
    assert_eq!(reply.status, StatusCode::from_u16(status).unwrap());
    if status == 200 {
        assert_eq!(app.database.budget_count().await, 1);
    } else {
        assert_eq!(reply.detail_error(), message);
        assert_eq!(app.database.budget_count().await, 0);
    }
    app.close().await;
}

#[tokio::test]
async fn native_organization_routes_without_license() {
    let app = TestApp::start().await;
    let allowed = app.post(master_caller(), DEFAULT_BODY).await;
    assert_eq!(allowed.status, StatusCode::OK);
    assert_eq!(app.database.organization_count().await, 1);
    // the only refusal left is the handler's own admin check, never the enterprise license answer
    let refused = app
        .post(
            key_caller(Some("u1"), Some("internal_user"), &[]),
            DEFAULT_BODY,
        )
        .await;
    assert_eq!(refused.status, StatusCode::UNAUTHORIZED);
    assert_eq!(
        refused.detail_error(),
        "Only admins can create orgs. Your role is = internal_user"
    );
    app.close().await;
}

#[tokio::test]
async fn production_router_rejects_creation_path() {
    let application = router(
        state_for(&empty_config()),
        Arc::new(AtomicBool::new(false)),
        Router::new(),
        Router::new(),
    );
    let response = application
        .oneshot(
            Request::post("/organization/new")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    // the route-match gate does not admit the path, so the handler's mount is unreachable from the binary
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn create_nonfinite_literal_rejected() {
    let app = TestApp::start().await;
    let reply = app
        .post(
            master_caller(),
            r#"{"organization_alias":"acme","max_budget":NaN}"#,
        )
        .await;
    assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
    // the body never becomes a request, so no budget or organization row is written
    assert_eq!(app.database.budget_count().await, 0);
    assert_eq!(app.database.organization_count().await, 0);
    app.close().await;
}
