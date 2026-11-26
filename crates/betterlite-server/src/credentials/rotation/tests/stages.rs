#![cfg(test)]

use betterlite_db::mcp_stored_contracts::JsonValue;
use betterlite_db::model_secrets::DEFAULT_USER_ID;
use rstest::rstest;

use super::support::{
    McpFailure, NEW_KEY, OLD_KEY, assertion_row, credential_row, credential_writes, fixture, open,
    sealed, server_row, text,
};
use crate::mcp_rotation::store::{StoredUserCredentialRow, StoredUserEnvVarRow};

#[derive(Debug, Clone, Copy)]
enum Reject {
    Server,
    UserCredential,
    UserEnvVar,
}

// Every MCP stage sits in its own try/except in _rotate_master_key: one scan failure warns and every other stage still runs.
#[rstest]
#[case::servers(McpFailure::Servers, 0, 1, 1)]
#[case::user_credentials(McpFailure::UserCredentials, 1, 0, 1)]
#[case::user_env_vars(McpFailure::UserEnvVars, 1, 1, 0)]
#[tokio::test]
async fn each_mcp_stage_scan_failure_is_contained(
    #[case] failure: McpFailure,
    #[case] servers_updated: usize,
    #[case] credentials_updated: usize,
    #[case] env_vars_updated: usize,
) {
    let mut fixture = fixture();
    fixture.mcp.failures = vec![failure];
    fixture.mcp.servers = vec![server_row(
        "s1",
        format!(r#"{{"auth_value":"{}"}}"#, sealed("server-token")),
    )];
    fixture.mcp.user_credentials = vec![StoredUserCredentialRow {
        user_id: "u1".to_owned(),
        server_id: "s1".to_owned(),
        credential_b64: sealed("user-token"),
    }];
    fixture.mcp.user_env_vars = vec![StoredUserEnvVarRow {
        user_id: "u1".to_owned(),
        server_id: "s1".to_owned(),
        values_b64: sealed(r#"{"TOKEN":"value"}"#),
    }];
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
    assert_eq!(fixture.mcp.server_updates().len(), servers_updated);
    assert_eq!(
        fixture.mcp.user_credential_writes().len(),
        credentials_updated
    );
    assert_eq!(fixture.mcp.user_env_var_writes().len(), env_vars_updated);
    let assertions = fixture.assertions.rows.lock().unwrap().clone();
    assert_eq!(
        open(&assertions[0].assertion_b64, NEW_KEY).as_deref(),
        Some("subject-token")
    );
    assert_eq!(credential_writes(&fixture.store.writes()).len(), 1);
}

// A failed row update ends only its own stage: the rows written before it stay and every later stage still runs.
#[rstest]
#[case::server(Reject::Server, 1, 2, 2)]
#[case::user_credential(Reject::UserCredential, 2, 1, 2)]
#[case::user_env_var(Reject::UserEnvVar, 2, 2, 1)]
#[tokio::test]
async fn a_failed_mcp_row_update_keeps_the_earlier_commits(
    #[case] reject: Reject,
    #[case] servers_updated: usize,
    #[case] credentials_updated: usize,
    #[case] env_vars_updated: usize,
) {
    let mut fixture = fixture();
    fixture.mcp.servers = vec![
        server_row(
            "s1",
            format!(r#"{{"auth_value":"{}"}}"#, sealed("server-token")),
        ),
        server_row(
            "s2",
            format!(r#"{{"auth_value":"{}"}}"#, sealed("server-token")),
        ),
    ];
    fixture.mcp.user_credentials = vec![
        StoredUserCredentialRow {
            user_id: "u1".to_owned(),
            server_id: "s1".to_owned(),
            credential_b64: sealed("user-token"),
        },
        StoredUserCredentialRow {
            user_id: "u2".to_owned(),
            server_id: "s2".to_owned(),
            credential_b64: sealed("user-token"),
        },
    ];
    fixture.mcp.user_env_vars = vec![
        StoredUserEnvVarRow {
            user_id: "u1".to_owned(),
            server_id: "s1".to_owned(),
            values_b64: sealed(r#"{"TOKEN":"value"}"#),
        },
        StoredUserEnvVarRow {
            user_id: "u2".to_owned(),
            server_id: "s2".to_owned(),
            values_b64: sealed(r#"{"TOKEN":"value"}"#),
        },
    ];
    match reject {
        Reject::Server => fixture.mcp.rejected_servers = vec!["s2".to_owned()],
        Reject::UserCredential => {
            fixture.mcp.rejected_user_credentials = vec!["u2".to_owned()];
        }
        Reject::UserEnvVar => fixture.mcp.rejected_user_env_vars = vec!["u2".to_owned()],
    }
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
    let servers = fixture.mcp.server_updates();
    assert_eq!(servers.len(), servers_updated);
    assert_eq!(servers[0].updated_by, DEFAULT_USER_ID);
    let credentials = fixture.mcp.user_credential_writes();
    assert_eq!(credentials.len(), credentials_updated);
    assert_eq!(
        open(&credentials[0].value, NEW_KEY).as_deref(),
        Some("user-token")
    );
    let env_vars = fixture.mcp.user_env_var_writes();
    assert_eq!(env_vars.len(), env_vars_updated);
    assert_eq!(
        open(&env_vars[0].value, NEW_KEY).as_deref(),
        Some(r#"{"TOKEN":"value"}"#)
    );
    let assertions = fixture.assertions.rows.lock().unwrap().clone();
    assert_eq!(
        open(&assertions[0].assertion_b64, NEW_KEY).as_deref(),
        Some("subject-token")
    );
    assert_eq!(credential_writes(&fixture.store.writes()).len(), 1);
}

// With no failure every MCP stage writes its re-sealed value, both per-user stages included.
#[tokio::test]
async fn every_mcp_stage_writes_when_none_fails() {
    let mut fixture = fixture();
    fixture.mcp.servers = vec![server_row(
        "s1",
        format!(r#"{{"auth_value":"{}"}}"#, sealed("server-token")),
    )];
    fixture.mcp.user_credentials = vec![StoredUserCredentialRow {
        user_id: "u1".to_owned(),
        server_id: "s1".to_owned(),
        credential_b64: sealed("user-token"),
    }];
    fixture.mcp.user_env_vars = vec![StoredUserEnvVarRow {
        user_id: "u1".to_owned(),
        server_id: "s1".to_owned(),
        values_b64: sealed(r#"{"TOKEN":"value"}"#),
    }];
    fixture.rotation().run(None).await.unwrap();
    let servers = fixture.mcp.server_updates();
    assert_eq!(servers.len(), 1);
    assert_eq!(servers[0].updated_by, DEFAULT_USER_ID);
    let credentials = fixture.mcp.user_credential_writes();
    assert_eq!(credentials.len(), 1);
    assert_eq!(credentials[0].user_id, "u1");
    assert_eq!(credentials[0].server_id, "s1");
    assert_eq!(
        open(&credentials[0].value, NEW_KEY).as_deref(),
        Some("user-token")
    );
    assert_eq!(open(&credentials[0].value, OLD_KEY), None);
    let env_vars = fixture.mcp.user_env_var_writes();
    assert_eq!(env_vars.len(), 1);
    assert_eq!(env_vars[0].user_id, "u1");
    assert_eq!(env_vars[0].server_id, "s1");
    assert_eq!(
        open(&env_vars[0].value, NEW_KEY).as_deref(),
        Some(r#"{"TOKEN":"value"}"#)
    );
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
