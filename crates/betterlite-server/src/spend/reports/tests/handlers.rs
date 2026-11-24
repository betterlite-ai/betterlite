use std::sync::Arc;

use axum::http::StatusCode;
use betterlite_auth::auth_identity::records::Principal;
use betterlite_models::key_digest::hashing::hash_token;
use rstest::rstest;

use crate::spend::reports::rows::OrganizationKeySpend;

use super::super::error::DB_NOT_CONNECTED_ERROR;
use super::support::{
    DetailBody, InternalBody, call, decimal, fake_store, jwt_user, key_principal, master_key,
    model, organization_model, scoped_row, user,
};

const RANGE: &str = "start_date=2024-01-01&end_date=2024-01-02";

fn caller_hash() -> String {
    hash_token("sk-caller")
}

fn owner() -> Principal {
    key_principal(r#"{"api_key":"sk-caller","user_id":"u1"}"#)
}

fn scoped_call(route: &str, query: &str) -> String {
    let separator = if query.is_empty() { "" } else { "&" };
    format!("/{route}/spend/report?{RANGE}{separator}{query}")
}

fn expected_scoped(column: &str, scope: &str) -> String {
    format!("scoped_report({column},2024-01-01T00:00:00+00:00,2024-01-02T00:00:00+00:00,{scope})")
}

#[tokio::test]
async fn requests_without_authentication_are_rejected() {
    let store = fake_store();
    let reply = call("/key/spend/report", None, Some(&Arc::new(store))).await;
    assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn a_missing_database_is_a_500_before_dates_or_scope() {
    let reply = call(
        "/key/spend/report?start_date=not-a-date&api_key=other",
        Some(&master_key()),
        None,
    )
    .await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
    let body: DetailBody = reply.decode();
    assert_eq!(body.detail, DB_NOT_CONNECTED_ERROR);
}

#[rstest]
#[case::missing_end(
    "/key/spend/report?start_date=2024-01-01",
    "Please provide start_date and end_date"
)]
#[case::malformed(
    "/key/spend/report?start_date=2024-13-01&end_date=2024-01-02",
    "start_date and end_date must be in YYYY-MM-DD format"
)]
#[case::reversed(
    "/key/spend/report?start_date=2024-01-02&end_date=2024-01-01",
    "start_date must be on or before end_date"
)]
#[case::too_large(
    "/key/spend/report?start_date=2024-01-01&end_date=2025-01-02",
    "Date range too large; maximum is 366 days"
)]
#[tokio::test]
async fn date_errors_are_400_before_any_scope_or_query(#[case] uri: &str, #[case] detail: &str) {
    let store = Arc::new(fake_store());
    let reply = call(uri, Some(&master_key()), Some(&store)).await;
    assert_eq!(reply.status, StatusCode::BAD_REQUEST);
    let body: DetailBody = reply.decode();
    assert_eq!(body.detail, detail);
    assert!(store.calls().is_empty());
}

#[tokio::test]
async fn exactly_366_days_reaches_the_report_query() {
    let store = Arc::new(fake_store());
    let reply = call(
        "/key/spend/report?start_date=2024-01-01&end_date=2025-01-01",
        Some(&owner()),
        Some(&store),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(
        store.calls(),
        vec![format!(
            "scoped_report(ApiKey,2024-01-01T00:00:00+00:00,2025-01-01T00:00:00+00:00,{})",
            caller_hash()
        )]
    );
}

#[rstest]
#[case::absent_parameter("")]
#[case::empty_parameter("api_key=")]
#[tokio::test]
async fn key_reports_fall_back_to_the_hashed_logged_key(#[case] query: &str) {
    let store = Arc::new(fake_store());
    let uri = scoped_call("key", query);
    let reply = call(&uri, Some(&owner()), Some(&store)).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(
        store.calls(),
        vec![expected_scoped("ApiKey", &caller_hash())]
    );
}

#[rstest]
#[case::raw_key("sk-caller")]
#[case::precomputed_hash("hash")]
#[tokio::test]
async fn key_reports_accept_their_own_key_and_hash_it(#[case] requested: &str) {
    let api_key = if requested == "hash" {
        caller_hash()
    } else {
        requested.to_owned()
    };
    let store = Arc::new(fake_store());
    let uri = scoped_call("key", &format!("api_key={api_key}"));
    let reply = call(&uri, Some(&owner()), Some(&store)).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(
        store.calls(),
        vec![expected_scoped("ApiKey", &caller_hash())]
    );
}

#[tokio::test]
async fn key_reports_reject_a_foreign_key_for_non_admins() {
    let store = Arc::new(fake_store());
    let uri = scoped_call("key", "api_key=someone-else");
    let reply = call(&uri, Some(&owner()), Some(&store)).await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);
    let body: DetailBody = reply.decode();
    assert_eq!(
        body.detail,
        "Not authorized to view spend for a api_key other than your own"
    );
    assert!(store.calls().is_empty());
}

#[tokio::test]
async fn key_reports_without_a_caller_key_are_a_400() {
    let store = Arc::new(fake_store());
    let caller = jwt_user("u1");
    let uri = scoped_call("key", "");
    let reply = call(&uri, Some(&caller), Some(&store)).await;
    assert_eq!(reply.status, StatusCode::BAD_REQUEST);
    let body: DetailBody = reply.decode();
    assert_eq!(
        body.detail,
        "No api_key associated with this API key; pass a api_key query param"
    );
    assert!(store.calls().is_empty());
}

#[rstest]
#[case::master(master_key())]
#[case::key_admin(key_principal(r#"{"api_key":"sk-admin","user_role":"proxy_admin"}"#))]
#[case::key_viewer(key_principal(r#"{"api_key":"sk-viewer","user_role":"proxy_admin_viewer"}"#))]
#[tokio::test]
async fn admin_views_may_request_any_scope(#[case] caller: Principal) {
    let store = Arc::new(fake_store());
    let uri = scoped_call("key", "api_key=someone-else");
    let reply = call(&uri, Some(&caller), Some(&store)).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(
        store.calls(),
        vec![expected_scoped("ApiKey", "someone-else")]
    );
}

#[tokio::test]
async fn master_reports_fall_back_to_litellm_proxy_master_key() {
    let store = Arc::new(fake_store());
    let reply = call(&scoped_call("key", ""), Some(&master_key()), Some(&store)).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(
        store.calls(),
        vec![expected_scoped("ApiKey", "litellm_proxy_master_key")]
    );
}

#[tokio::test]
async fn session_tokens_log_and_query_their_key_alias() {
    let caller = key_principal(
        r#"{"api_key":"sk-session","is_session_token":true,"key_alias":"session-alias"}"#,
    );
    let store = Arc::new(fake_store());
    let reply = call(&scoped_call("key", ""), Some(&caller), Some(&store)).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(
        store.calls(),
        vec![expected_scoped("ApiKey", "session-alias")]
    );
}

#[rstest]
#[case::own_user("internal_user_id=u1")]
#[case::empty_falls_back("internal_user_id=")]
#[case::absent_falls_back("")]
#[tokio::test]
async fn user_reports_scope_to_the_callers_user_id(#[case] query: &str) {
    let store = Arc::new(fake_store());
    let uri = scoped_call("user", query);
    let reply = call(&uri, Some(&owner()), Some(&store)).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(store.calls(), vec![expected_scoped("User", "u1")]);
}

#[tokio::test]
async fn an_admin_may_query_any_user() {
    let store = Arc::new(fake_store());
    let uri = scoped_call("user", "internal_user_id=u2");
    let reply = call(&uri, Some(&master_key()), Some(&store)).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(store.calls(), vec![expected_scoped("User", "u2")]);
}

#[tokio::test]
async fn user_reports_reject_a_foreign_user_for_non_admins() {
    let store = Arc::new(fake_store());
    let uri = scoped_call("user", "internal_user_id=u2");
    let reply = call(&uri, Some(&owner()), Some(&store)).await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);
    let body: DetailBody = reply.decode();
    assert_eq!(
        body.detail,
        "Not authorized to view spend for a internal_user_id other than your own"
    );
    assert!(store.calls().is_empty());
}

#[tokio::test]
async fn user_reports_without_a_user_are_a_400() {
    let store = Arc::new(fake_store());
    let caller = key_principal(r#"{"api_key":"sk-no-user"}"#);
    let uri = scoped_call("user", "");
    let reply = call(&uri, Some(&caller), Some(&store)).await;
    assert_eq!(reply.status, StatusCode::BAD_REQUEST);
    let body: DetailBody = reply.decode();
    assert_eq!(
        body.detail,
        "No internal_user_id associated with this API key; pass a internal_user_id query param"
    );
    assert!(store.calls().is_empty());
}

#[rstest]
#[case::own_team("team_id=team-1")]
#[case::empty_falls_back("team_id=")]
#[case::absent_falls_back("")]
#[tokio::test]
async fn team_reports_scope_to_the_keys_team(#[case] query: &str) {
    let caller = key_principal(r#"{"api_key":"sk-team","team_id":"team-1"}"#);
    let store = Arc::new(fake_store());
    let uri = scoped_call("team", query);
    let reply = call(&uri, Some(&caller), Some(&store)).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(store.calls(), vec![expected_scoped("TeamId", "team-1")]);
}

#[tokio::test]
async fn team_reports_reject_a_foreign_team() {
    let store = Arc::new(fake_store());
    let caller = key_principal(r#"{"api_key":"sk-team","team_id":"team-1"}"#);
    let uri = scoped_call("team", "team_id=team-2");
    let reply = call(&uri, Some(&caller), Some(&store)).await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);
    let body: DetailBody = reply.decode();
    assert_eq!(
        body.detail,
        "Not authorized to view spend for a team_id other than your own"
    );
    assert!(store.calls().is_empty());
}

#[tokio::test]
async fn team_reports_without_a_team_are_a_400() {
    let store = Arc::new(fake_store());
    let caller = key_principal(r#"{"api_key":"sk-no-team"}"#);
    let uri = scoped_call("team", "");
    let reply = call(&uri, Some(&caller), Some(&store)).await;
    assert_eq!(reply.status, StatusCode::BAD_REQUEST);
    let body: DetailBody = reply.decode();
    assert_eq!(
        body.detail,
        "No team_id associated with this API key; pass a team_id query param"
    );
}

fn org_caller() -> Principal {
    key_principal(r#"{"api_key":"sk-org","user_id":"u1","org_id":"org-1","user_role":"org_admin"}"#)
}

fn membership(organization_id: &str) -> String {
    format!(
        r#"{{"user_id":"u1","spend":0,"organization_memberships":[{{"user_id":"u1","organization_id":"{organization_id}","user_role":"org_admin","created_at":"2024-01-01T00:00:00Z","updated_at":"2024-01-01T00:00:00Z"}}]}}"#
    )
}

#[tokio::test]
async fn organization_reports_verify_access_and_use_current_teams() {
    let caller = org_caller();
    let mut fake = fake_store();
    fake.team_ids = vec!["team-1".to_owned(), "team-2".to_owned()];
    fake.users
        .insert("u1".to_owned(), user(&membership("org-1")));
    fake.organization = vec![OrganizationKeySpend {
        api_key: "key-hash".to_owned(),
        total_cost: decimal("1.5"),
        total_input_tokens: 1,
        total_output_tokens: 2,
        model_details: vec![organization_model(Some("team-1"), "gpt", "1.5")],
    }];
    let store = Arc::new(fake);
    let uri = scoped_call("organization", "");
    let reply = call(&uri, Some(&caller), Some(&store)).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(
        store.calls(),
        vec![
            "organization_team_ids(org-1)".to_owned(),
            "organization_report(2024-01-01T00:00:00+00:00,2024-01-02T00:00:00+00:00,org-1,[\"team-1\", \"team-2\"])".to_owned(),
        ]
    );
    let rows: Vec<OrganizationKeySpend> = reply.decode();
    assert_eq!(rows[0].model_details[0].team_id.as_deref(), Some("team-1"));
    assert_eq!(rows[0].total_cost.to_string(), "1.5");
}

#[rstest]
#[case::empty_parameter("organization_id=")]
#[case::absent_parameter("")]
#[tokio::test]
async fn organization_reports_default_to_the_key_organization(#[case] query: &str) {
    let caller = org_caller();
    let mut fake = fake_store();
    fake.users
        .insert("u1".to_owned(), user(&membership("org-1")));
    let store = Arc::new(fake);
    let uri = scoped_call("organization", query);
    let reply = call(&uri, Some(&caller), Some(&store)).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(store.calls()[0], "organization_team_ids(org-1)");
}

#[tokio::test]
async fn organization_reports_reject_a_foreign_organization() {
    let caller = org_caller();
    let mut fake = fake_store();
    fake.users
        .insert("u1".to_owned(), user(&membership("org-2")));
    let store = Arc::new(fake);
    let uri = scoped_call("organization", "organization_id=org-1");
    let reply = call(&uri, Some(&caller), Some(&store)).await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);
    let body: DetailBody = reply.decode();
    assert_eq!(body.detail, "You do not have access to this organization");
    assert!(store.calls().is_empty());
}

#[tokio::test]
async fn organization_admins_need_a_known_user() {
    let caller = org_caller();
    let store = Arc::new(fake_store());
    let uri = scoped_call("organization", "");
    let reply = call(&uri, Some(&caller), Some(&store)).await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);
    assert!(store.calls().is_empty());
}

#[tokio::test]
async fn an_admin_without_an_organization_is_a_400() {
    let store = Arc::new(fake_store());
    let reply = call(
        "/organization/spend/report?start_date=2024-01-01&end_date=2024-01-02",
        Some(&master_key()),
        Some(&store),
    )
    .await;
    assert_eq!(reply.status, StatusCode::BAD_REQUEST);
    let body: DetailBody = reply.decode();
    assert_eq!(
        body.detail,
        "No organization_id associated with this API key; pass an organization_id query_param"
    );
    assert!(store.calls().is_empty());
}

#[tokio::test]
async fn an_admin_may_report_on_any_organization() {
    let store = Arc::new(fake_store());
    let reply = call(
        "/organization/spend/report?start_date=2024-01-01&end_date=2024-01-02&organization_id=org-9",
        Some(&master_key()),
        Some(&store),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(store.calls()[0], "organization_team_ids(org-9)");
}

#[tokio::test]
async fn organization_team_lookup_failures_are_internal_errors() {
    let caller = org_caller();
    let mut fake = fake_store();
    fake.users
        .insert("u1".to_owned(), user(&membership("org-1")));
    fake.failure = true;
    let store = Arc::new(fake);
    let uri = scoped_call("organization", "");
    let reply = call(&uri, Some(&caller), Some(&store)).await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
    let body: InternalBody = reply.decode();
    assert_eq!(body.error.message, "Internal server error");
    assert_eq!(body.error.kind, "internal_server_error");
}

#[tokio::test]
async fn report_query_failures_are_internal_errors() {
    let mut fake = fake_store();
    fake.failure = true;
    fake.scoped = vec![scoped_row(
        "key-hash",
        "1.0",
        vec![model("gpt", "1.0", 1, 2)],
    )];
    let store = Arc::new(fake);
    let reply = call(&scoped_call("key", ""), Some(&owner()), Some(&store)).await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
    let body: InternalBody = reply.decode();
    assert_eq!(body.error.message, "Internal server error");
    assert_eq!(body.error.kind, "internal_server_error");
}

#[tokio::test]
async fn empty_reports_answer_an_empty_array() {
    let store = Arc::new(fake_store());
    let reply = call(&scoped_call("key", ""), Some(&owner()), Some(&store)).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.text(), "[]");
}
