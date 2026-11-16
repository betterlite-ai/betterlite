use betterlite_db::managed_resource_rows::mcp::McpEnvVarScope;
use betterlite_db::mcp::table_access::types::IdentifierField;
use betterlite_db::mcp_stored_contracts::JsonValue;
use uuid::Uuid;

use super::support::cipher::{SALT, cipher, opened_env_vars};
use super::support::database::{SEED_DRAFT_SQL, SERVER_COUNT_SQL, SERVER_NAME_COUNT_SQL, database};
use super::support::fake::RecordingInvalidation;
use super::support::logs::capture_logs;
use super::support::request;
use crate::mcp::server_writes::create::{create_mcp_server, create_mcp_server_if_identifier_free};
use crate::mcp::server_writes::draft::{create_draft_mcp_server, get_draft_mcp_server};
use crate::mcp::server_writes::{CreatedServer, DraftOptions};

const ENV_VARS: &str = r#"[
    {"name": "API_KEY", "value": "global-secret", "scope": "global"},
    {"name": "USER_TOKEN", "value": "hint", "scope": "user"}
]"#;

fn payload(server_id: &str, server_name: &str) -> String {
    let id = if server_id.is_empty() {
        String::new()
    } else {
        format!(r#""server_id": "{server_id}", "#)
    };
    format!(
        r#"{{{id}"server_name": "{server_name}", "transport": "http", "url": "https://mcp.example/sse", "env_vars": {ENV_VARS}}}"#
    )
}

fn env_value(entries: &[JsonValue], name: &str) -> String {
    for entry in entries {
        let JsonValue::Object(fields) = entry else {
            panic!("a stored env var is an object");
        };
        if fields.get("name") != Some(&JsonValue::String(name.to_owned())) {
            continue;
        }
        let Some(JsonValue::String(value)) = fields.get("value") else {
            panic!("the stored env var {name} has a string value");
        };
        return value.clone();
    }
    panic!("the stored env vars are missing {name}");
}

#[tokio::test]
async fn create_generates_a_uuid_sets_audit_and_returns_plaintext_env_vars() {
    let harness = database().await;
    let store = harness.writes();
    let cipher = cipher(SALT);
    let created = create_mcp_server(&store, &cipher, &request(&payload("", "weather")), "admin")
        .await
        .unwrap();
    Uuid::parse_str(&created.server_id).expect("the generated id is a uuid");
    assert_eq!(created.server_name.as_deref(), Some("weather"));
    assert_eq!(created.created_by.as_deref(), Some("admin"));
    assert_eq!(created.updated_by.as_deref(), Some("admin"));
    let returned = created.env_vars.as_ref().unwrap();
    assert_eq!(returned.len(), 2);
    assert_eq!(returned[0].value, "global-secret");
    assert_eq!(returned[0].scope, McpEnvVarScope::Global);
    assert_eq!(returned[1].value, "hint");
    assert_eq!(returned[1].scope, McpEnvVarScope::User);
    let column = harness
        .raw_column(
            r#"SELECT "env_vars"::text FROM "LiteLLM_MCPServerTable" WHERE "server_id" = $1"#,
            &created.server_id,
        )
        .await
        .unwrap();
    assert!(
        !column.contains("global-secret"),
        "the global value is encrypted at rest"
    );
    assert!(
        column.contains("hint"),
        "the per-user placeholder is stored plaintext"
    );
    let opened = opened_env_vars(&column, &cipher);
    assert_eq!(env_value(&opened, "API_KEY"), "global-secret");
    assert_eq!(env_value(&opened, "USER_TOKEN"), "hint");
    harness.close().await;
}

// A fixed id is kept as given; omitting one is what makes create generate a fresh uuid.
#[tokio::test]
async fn create_keeps_a_given_server_id() {
    let harness = database().await;
    let store = harness.writes();
    let cipher = cipher(SALT);
    let created = create_mcp_server(&store, &cipher, &request(&payload("srv-1", "one")), "admin")
        .await
        .unwrap();
    assert_eq!(created.server_id, "srv-1");
    assert_eq!(harness.count(SERVER_COUNT_SQL).await, 1);
    harness.close().await;
}

#[tokio::test]
async fn create_if_identifier_free_returns_the_created_row() {
    let harness = database().await;
    let store = harness.writes();
    let cipher = cipher(SALT);
    let request = request(&payload("srv-1", "weather"));
    let created = create_mcp_server_if_identifier_free(&store, &cipher, &request, "admin")
        .await
        .unwrap();
    let CreatedServer::Created(server) = created else {
        panic!("the identifier is free");
    };
    assert_eq!(server.server_id, "srv-1");
    assert_eq!(harness.count(SERVER_COUNT_SQL).await, 1);
    harness.close().await;
}

// Alias is checked before server_name and both compare case-insensitively.
#[tokio::test]
async fn create_if_identifier_free_reports_case_insensitive_conflicts() {
    let harness = database().await;
    let store = harness.writes();
    let cipher = cipher(SALT);
    let first = request(&payload("srv-a", "Weather"));
    create_mcp_server_if_identifier_free(&store, &cipher, &first, "admin")
        .await
        .unwrap();
    let by_name = request(&payload("srv-b", "weather"));
    let named = create_mcp_server_if_identifier_free(&store, &cipher, &by_name, "admin")
        .await
        .unwrap();
    let CreatedServer::Conflict(name_conflict) = named else {
        panic!("the server_name collides");
    };
    assert_eq!(name_conflict.field, IdentifierField::ServerName);
    assert_eq!(name_conflict.value, "weather");
    assert_eq!(name_conflict.server_id, "srv-a");
    let mut with_alias = request(&payload("srv-c", "unrelated"));
    with_alias.alias = Some("WEATHER".to_owned());
    let aliased = create_mcp_server_if_identifier_free(&store, &cipher, &with_alias, "admin")
        .await
        .unwrap();
    let CreatedServer::Conflict(alias_conflict) = aliased else {
        panic!("the alias collides");
    };
    assert_eq!(alias_conflict.field, IdentifierField::Alias);
    assert_eq!(alias_conflict.value, "WEATHER");
    assert_eq!(
        harness.count_param(SERVER_NAME_COUNT_SQL, "Weather").await,
        1
    );
    harness.close().await;
}

// Draft rows back a transient OAuth session, so their names never block a real server.
#[tokio::test]
async fn create_if_identifier_free_ignores_draft_rows() {
    let harness = database().await;
    let store = harness.writes();
    let cipher = cipher(SALT);
    let options = DraftOptions {
        touched_by: "admin".to_owned(),
        ttl_seconds: 300,
        server_id: Some("srv-draft".to_owned()),
    };
    let draft = request(&payload("srv-d", "DRAFTED"));
    create_draft_mcp_server(
        &store,
        &cipher,
        &draft,
        &options,
        &RecordingInvalidation::new(),
    )
    .await
    .unwrap();
    let draft_name = request(&payload("srv-e", "drafted"));
    let created = create_mcp_server_if_identifier_free(&store, &cipher, &draft_name, "admin")
        .await
        .unwrap();
    assert!(
        matches!(created, CreatedServer::Created(_)),
        "drafts are excluded"
    );
    harness.close().await;
}

// Two inserts of one identifier: the advisory locks let exactly one pass the collision check.
#[tokio::test]
async fn create_if_identifier_free_serializes_concurrent_inserts() {
    let harness = database().await;
    let store = harness.writes();
    let cipher = cipher(SALT);
    let left_request = request(&payload("srv-left", "shared"));
    let right_request = request(&payload("srv-right", "shared"));
    let (left, right) = tokio::join!(
        create_mcp_server_if_identifier_free(&store, &cipher, &left_request, "admin"),
        create_mcp_server_if_identifier_free(&store, &cipher, &right_request, "admin"),
    );
    let outcomes = [left.unwrap(), right.unwrap()];
    let created = outcomes
        .iter()
        .filter(|outcome| matches!(outcome, CreatedServer::Created(_)))
        .count();
    let conflicts = outcomes
        .iter()
        .filter(|outcome| matches!(outcome, CreatedServer::Conflict(_)))
        .count();
    assert_eq!((created, conflicts), (1, 1));
    assert_eq!(
        harness.count_param(SERVER_NAME_COUNT_SQL, "shared").await,
        1
    );
    harness.close().await;
}

// The read path drops a global value it cannot decrypt instead of forwarding the ciphertext.
#[tokio::test]
async fn get_draft_drops_undecryptable_global_env_vars() {
    let harness = database().await;
    let store = harness.writes();
    let cipher = cipher(SALT);
    let env_vars = r#"[{"name": "BROKEN", "value": "not-a-ciphertext", "scope": "global"},
        {"name": "PLAIN", "value": "kept", "scope": "user"}]"#;
    harness
        .execute(
            SEED_DRAFT_SQL,
            &[
                &"srv-draft".to_owned(),
                &"broken".to_owned(),
                &env_vars.to_owned(),
            ],
        )
        .await;
    let (logs, guard) = capture_logs();
    let found = get_draft_mcp_server(&store, &cipher, "srv-draft", 300)
        .await
        .unwrap()
        .unwrap();
    drop(guard);
    let entries = found.env_vars.as_ref().unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].name, "BROKEN");
    assert_eq!(entries[0].value, "");
    assert_eq!(entries[1].value, "kept");
    assert!(logs.contents().contains("failed to decrypt"));
    harness.close().await;
}
