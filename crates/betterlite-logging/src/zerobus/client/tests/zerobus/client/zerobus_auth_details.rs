use std::time::Duration;

use super::super::support::{
    FakeClock, FakeZerobus, Reply, WORKSPACE_ID, client_with, connection, expected, token_reply,
};

async fn token_form() -> (String, String, String, String) {
    let fake = FakeZerobus::start(
        vec![Reply::new(200, &token_reply("tok-1", "3600"))],
        vec![Reply::new(202, "{}")],
    )
    .await;
    let clock = FakeClock::new(1_000.0);
    let client = client_with(connection(&fake.base), &clock, Duration::from_secs(5));

    client.insert(&[]).await.unwrap();

    let form = fake.token_calls()[0].form();
    let details = form["authorization_details"].clone();
    (
        form["grant_type"].clone(),
        form["scope"].clone(),
        form["resource"].clone(),
        details,
    )
}

#[tokio::test]
async fn the_token_request_asks_for_the_zerobus_resource_with_the_client_credentials_grant() {
    let (grant_type, scope, resource, _details) = token_form().await;

    assert_eq!(grant_type, "client_credentials");
    assert_eq!(scope, "all-apis");
    assert_eq!(
        resource,
        format!("api://databricks/workspaces/{WORKSPACE_ID}/zerobusDirectWriteApi")
    );
}

#[tokio::test]
async fn a_table_name_that_is_not_catalog_schema_table_is_a_permanent_failure() {
    let fake = FakeZerobus::start(
        vec![Reply::new(200, &token_reply("tok-1", "3600"))],
        vec![Reply::new(202, "{}")],
    )
    .await;
    let mut connection = connection(&fake.base);
    connection.table_name = "traces".to_owned();
    let clock = FakeClock::new(1_000.0);
    let client = client_with(connection, &clock, Duration::from_secs(5));

    let failure = client.insert(&[]).await.unwrap_err();

    assert!(!failure.retryable);
    assert!(failure.detail.contains("catalog.schema.table"));
}

#[tokio::test]
async fn the_token_request_carries_the_table_privileges() {
    let (_grant_type, _scope, _resource, details) = token_form().await;

    assert_eq!(
        expected(&details),
        expected(
            r#"[
                {
                    "type": "unity_catalog_privileges",
                    "privileges": ["USE CATALOG"],
                    "object_type": "CATALOG",
                    "object_full_path": "main"
                },
                {
                    "type": "unity_catalog_privileges",
                    "privileges": ["USE SCHEMA"],
                    "object_type": "SCHEMA",
                    "object_full_path": "main.litellm"
                },
                {
                    "type": "unity_catalog_privileges",
                    "privileges": ["SELECT", "MODIFY"],
                    "object_type": "TABLE",
                    "object_full_path": "main.litellm.traces"
                }
            ]"#
        )
    );
}
