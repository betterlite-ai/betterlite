use std::collections::BTreeMap;

use axum::http::StatusCode;
use betterlite_db::budget_permission_rows::JsonData;

use super::support::{Reply, TestApp, admin_caller, assert_number, capture_logs, parsed};
use crate::organization_create::records::NewOrganizationRequest;

const DEFAULT_BODY: &str = r#"{"organization_alias":"acme"}"#;

fn text_value(value: Option<&JsonData>) -> &str {
    let Some(JsonData::String(text)) = value else {
        panic!("the value is not a JSON string");
    };
    text
}

#[test]
fn request_fields_projection() {
    let request: NewOrganizationRequest = serde_json::from_str(
        r#"{"organization_alias":"acme","models":["gpt-4"],"max_budget":12.5}"#,
    )
    .unwrap();
    let fields = request.fields().unwrap();
    // data.json(exclude_none=True): the required columns and every sent value enter the projection
    assert_eq!(
        fields.get("organization_alias"),
        Some(&JsonData::String(String::from("acme")))
    );
    assert_eq!(
        fields.get("models"),
        Some(&JsonData::Array(vec![JsonData::String(String::from(
            "gpt-4"
        ))]))
    );
    assert_number(&fields["max_budget"], 12.5_f64);
    // absent option fields are left out instead of written as null
    assert!(!fields.contains_key("metadata"));
    assert!(!fields.contains_key("object_permission"));
    assert!(!fields.contains_key("budget_id"));
}

#[tokio::test]
async fn metadata_field_partition() {
    let app = TestApp::start().await;
    let body = r#"{
        "organization_alias": "acme",
        "metadata": {"existing": 1},
        "model_rpm_limit": {"gpt-4": 5},
        "enable_prompt_caching": true,
        "temp_budget_increase": 2,
        "temp_budget_expiry": "2026-01-02T03:04:05+00:00"
    }"#;
    let reply = app.post(admin_caller(Some("u1")), body).await;
    assert_eq!(reply.status, StatusCode::OK);
    // enable_prompt_caching is not a NewOrganizationRequest field, so the unknown input never becomes metadata
    let expected = parsed(r#"{"existing":1,"model_rpm_limit":{"gpt-4":5}}"#);
    assert_eq!(reply.field("metadata"), expected);

    let organization = app
        .database
        .organization_row(&reply.text_field("organization_id"))
        .await
        .unwrap();
    let stored_metadata = text_value(organization.get("metadata"));
    assert_eq!(
        serde_json::from_str::<JsonData>(stored_metadata).unwrap(),
        expected
    );
    // the metadata fields have no organization column of their own
    assert!(!organization.contains_key("model_rpm_limit"));
    assert!(!organization.contains_key("enable_prompt_caching"));

    let budget = app
        .database
        .budget_row(&reply.text_field("budget_id"))
        .await
        .unwrap();
    // the temp budget columns stay on the budget row, not in metadata
    assert_number(&budget["temp_budget_increase"], 2.0_f64);
    assert!(matches!(
        budget.get("temp_budget_expiry"),
        Some(JsonData::String(_))
    ));
    assert!(!budget.contains_key("model_rpm_limit"));
    assert!(!budget.contains_key("enable_prompt_caching"));
    app.close().await;
}

#[tokio::test]
async fn organization_row_defaults() {
    let app = TestApp::start().await;
    let reply = app.post(admin_caller(Some("u1")), DEFAULT_BODY).await;
    assert_eq!(reply.status, StatusCode::OK);
    let organization_id = reply.text_field("organization_id");
    uuid::Uuid::parse_str(&organization_id).unwrap();
    assert_default_response(&reply);

    let stored = app
        .database
        .organization_row(&organization_id)
        .await
        .unwrap();
    assert_stored_defaults(&stored);

    // a sent id is used verbatim instead of a generated one
    assert_explicit_id_used(&app).await;
    app.close().await;
}

fn assert_default_response(reply: &Reply) {
    assert_eq!(reply.field("models"), JsonData::Array(Vec::new()));
    // the created row's metadata column defaults to an empty object
    assert_eq!(reply.field("metadata"), JsonData::Object(BTreeMap::new()));
    assert_eq!(reply.field("users"), JsonData::Null);
    assert_eq!(
        reply.field("model_spend"),
        JsonData::Object(BTreeMap::new())
    );
    assert_number(&reply.field("spend"), 0.0_f64);
    assert!(!text_value(Some(&reply.field("created_at"))).is_empty());
    assert!(!text_value(Some(&reply.field("updated_at"))).is_empty());
}

fn assert_stored_defaults(stored: &BTreeMap<String, JsonData>) {
    assert_eq!(
        stored["organization_alias"],
        JsonData::String(String::from("acme"))
    );
    assert_eq!(stored["created_by"], JsonData::String(String::from("u1")));
    assert_eq!(stored["models"], JsonData::Array(Vec::new()));
    assert_number(&stored["spend"], 0.0_f64);
    assert!(!text_value(stored.get("created_at")).is_empty());
    assert!(!text_value(stored.get("updated_at")).is_empty());
    // an absent metadata field takes the column default, and model_spend is jsonified
    assert_eq!(stored["metadata"], JsonData::Object(BTreeMap::new()));
    let stored_model_spend = text_value(stored.get("model_spend"));
    assert_eq!(
        serde_json::from_str::<JsonData>(stored_model_spend).unwrap(),
        JsonData::Object(BTreeMap::new())
    );
}

async fn assert_explicit_id_used(app: &TestApp) {
    let explicit = app
        .post(
            admin_caller(Some("u1")),
            r#"{"organization_alias":"explicit","organization_id":"org-fixed"}"#,
        )
        .await;
    assert_eq!(explicit.status, StatusCode::OK);
    assert_eq!(explicit.text_field("organization_id"), "org-fixed");
    let explicit_row = app.database.organization_row("org-fixed").await.unwrap();
    assert_eq!(
        explicit_row["organization_alias"],
        JsonData::String(String::from("explicit"))
    );
}

#[tokio::test]
async fn creation_log_before_insert() {
    let app = TestApp::start().await;
    app.database.seed_organization("org-taken").await;
    let (logs, guard) = capture_logs(tracing::Level::INFO);
    let reply = app
        .post(
            admin_caller(Some("u1")),
            r#"{"organization_alias":"dup","organization_id":"org-taken"}"#,
        )
        .await;
    drop(guard);
    // the insert collides with the stored row and fails after the row was logged
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(reply.error_text("message"), "Internal server error");
    let output = logs.contents();
    assert!(output.contains("new_organization_row:"), "{output}");
    assert!(
        output.contains("\"organization_alias\": \"dup\""),
        "the logged row is indented JSON: {output}"
    );
    assert_eq!(app.database.organization_count().await, 1);
    app.close().await;
}

#[tokio::test]
async fn organization_failure_keeps_children() {
    let app = TestApp::start().await;
    app.database.seed_organization("org-taken").await;
    let reply = app
        .post(
            admin_caller(Some("u1")),
            r#"{"organization_alias":"dup","organization_id":"org-taken","object_permission":{"vector_stores":["v1"]}}"#,
        )
        .await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(reply.error_text("message"), "Internal server error");
    // the handler's budget and permission rows survive the failed insert, beside the fixture's budget row
    assert_eq!(app.database.budget_count().await, 2);
    assert_eq!(app.database.permission_count().await, 1);
    assert_eq!(
        app.database.organization_ids().await,
        vec![String::from("org-taken")]
    );
    app.close().await;
}
