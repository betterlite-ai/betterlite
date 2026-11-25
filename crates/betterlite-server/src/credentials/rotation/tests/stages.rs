use betterlite_db::mcp_stored_contracts::JsonValue;
use betterlite_db::model_secrets::DEFAULT_USER_ID;
use rstest::rstest;

use super::support::{
    McpFailure, NEW_KEY, OLD_KEY, assertion_row, credential_row, credential_writes, fixture, open,
    sealed, server_row, text,
};

// Every MCP stage sits in its own try/except in _rotate_master_key: one failure warns and the SSO and credential stages still run.
#[rstest]
#[case::servers(McpFailure::Servers)]
#[case::user_credentials(McpFailure::UserCredentials)]
#[case::user_env_vars(McpFailure::UserEnvVars)]
#[tokio::test]
async fn each_mcp_stage_failure_is_contained(#[case] failure: McpFailure) {
    let mut fixture = fixture();
    fixture.mcp.failures = vec![failure];
    fixture
        .assertions
        .rows
        .lock()
        .unwrap()
        .push(assertion_row("u1", "subject-token"));
    fixture.store.credentials = vec![credential_row(
        "c1",
        vec![("api_key", text(&sealed("sk-live")))],
    )];
    fixture.rotation().run(None).await.unwrap();
    let assertions = fixture.assertions.rows.lock().unwrap().clone();
    assert_eq!(assertions.len(), 1);
    assert_eq!(
        open(&assertions[0].assertion_b64, NEW_KEY).as_deref(),
        Some("subject-token")
    );
    assert_eq!(credential_writes(&fixture.store.writes()).len(), 1);
}

// touched_by=user_api_key_dict.user_id or LITELLM_PROXY_ADMIN_NAME, so a caller with no user_id writes the admin name.
#[tokio::test]
async fn mcp_servers_are_touched_by_the_default_user() {
    let mut fixture = fixture();
    fixture.mcp.servers = vec![server_row(
        "s1",
        format!(r#"{{"auth_value":"{}"}}"#, sealed("secret-token")),
    )];
    fixture.rotation().run(None).await.unwrap();
    let updates = fixture.mcp.server_updates();
    assert_eq!(updates.len(), 1);
    assert_eq!(updates[0].updated_by, DEFAULT_USER_ID);
    let Some(JsonValue::String(rotated)) = updates[0].credentials.clone() else {
        panic!("the rotated blob is a JSON string");
    };
    let JsonValue::Object(fields) = serde_json::from_str::<JsonValue>(&rotated).unwrap() else {
        panic!("the rotated blob is a JSON object");
    };
    let JsonValue::String(auth_value) = &fields["auth_value"] else {
        panic!("the rotated auth value is text");
    };
    assert_eq!(open(auth_value, NEW_KEY).as_deref(), Some("secret-token"));
    assert_eq!(open(auth_value, OLD_KEY), None);
}

#[tokio::test]
async fn mcp_servers_are_touched_by_the_actor() {
    let mut fixture = fixture();
    fixture.mcp.servers = vec![server_row(
        "s1",
        format!(r#"{{"auth_value":"{}"}}"#, sealed("secret-token")),
    )];
    fixture.rotation().run(Some("admin-1")).await.unwrap();
    let updates = fixture.mcp.server_updates();
    assert_eq!(updates.len(), 1);
    assert_eq!(updates[0].updated_by, "admin-1");
}

// A failed assertion scan warns and the credential stage still runs.
#[tokio::test]
async fn sso_assertion_failures_are_contained() {
    let mut fixture = fixture();
    fixture.assertions.failures.find_all = Some(0);
    fixture
        .assertions
        .rows
        .lock()
        .unwrap()
        .push(assertion_row("u1", "subject-token"));
    fixture.store.credentials = vec![credential_row(
        "c1",
        vec![("api_key", text(&sealed("sk-live")))],
    )];
    fixture.rotation().run(None).await.unwrap();
    let assertions = fixture.assertions.rows.lock().unwrap().clone();
    assert_eq!(
        open(&assertions[0].assertion_b64, OLD_KEY).as_deref(),
        Some("subject-token")
    );
    assert_eq!(credential_writes(&fixture.store.writes()).len(), 1);
}
