use betterlite_db::managed_resource_rows::mcp::McpServerTable;
use betterlite_db::mcp_stored_contracts::JsonValue;

use super::support::cipher::{SALT, cipher, opened_credentials, seeded_credentials};
use super::support::database::{Harness, SEED_SERVER_SQL, database};
use super::support::update_payload;
use crate::mcp::server_writes::UpdatedServer;
use crate::mcp::server_writes::store::PostgresServerWrites;
use crate::mcp::server_writes::update::update_mcp_server;

const CREDENTIALS_SQL: &str =
    r#"SELECT "credentials"::text FROM "LiteLLM_MCPServerTable" WHERE "server_id" = $1"#;
const AUTH_TYPE_SQL: &str =
    r#"SELECT "auth_type"::text FROM "LiteLLM_MCPServerTable" WHERE "server_id" = $1"#;
const FLOW_SQL: &str = r#"UPDATE "LiteLLM_MCPServerTable" SET "url" = $2, "issuer" = $3, "token_url" = $4, "oauth2_flow" = $5, "token_exchange_endpoint" = $6, "audience" = $7 WHERE "server_id" = $1"#;
const TOKEN_EXCHANGE_SQL: &str = r#"SELECT "token_exchange_endpoint"::text FROM "LiteLLM_MCPServerTable" WHERE "server_id" = $1"#;

async fn seed_row(
    harness: &Harness,
    server_id: &str,
    auth_type: Option<&str>,
    credentials: Option<&str>,
) {
    let server_name = format!("name-{server_id}");
    harness
        .execute(
            SEED_SERVER_SQL,
            &[&server_id, &server_name, &auth_type, &credentials],
        )
        .await;
}

async fn seed_flow(harness: &Harness, server_id: &str, url: &str, issuer: Option<&str>) {
    harness
        .execute(
            FLOW_SQL,
            &[
                &server_id,
                &url,
                &issuer,
                &Some(format!("{url}/token")),
                &Some("client_credentials"),
                &Some("https://exchange"),
                &Some("the-audience"),
            ],
        )
        .await;
}

// The update the edit form sends, applied to a row that exists.
async fn updated_row(store: &PostgresServerWrites<'_>, payload: &str) -> McpServerTable {
    let (request, fields) = update_payload(payload);
    let updated = update_mcp_server(store, &cipher(SALT), &request, "editor", &fields)
        .await
        .unwrap();
    let UpdatedServer::Updated(Some(server)) = updated else {
        panic!("the row exists");
    };
    *server
}

fn blob_keys(column: &str) -> Vec<String> {
    let mut keys: Vec<String> = opened_credentials(column, &cipher(SALT))
        .into_keys()
        .collect();
    keys.sort();
    keys
}

fn blob_text(column: &str, field: &str) -> Option<String> {
    let fields = opened_credentials(column, &cipher(SALT));
    match fields.get(field) {
        Some(JsonValue::String(text)) => Some(text.clone()),
        Some(
            JsonValue::Null
            | JsonValue::Bool(_)
            | JsonValue::Number(_)
            | JsonValue::Array(_)
            | JsonValue::Object(_),
        )
        | None => None,
    }
}

async fn token_exchange_column(harness: &Harness) -> Option<String> {
    harness.raw_column(TOKEN_EXCHANGE_SQL, "srv-1").await
}

#[tokio::test]
async fn update_auth_type_change_replaces_cross_class_credentials_and_clears_flow_fields() {
    let harness = database().await;
    let store = harness.writes();
    let seeded = seeded_credentials(
        &[
            ("client_id", "cid"),
            ("client_secret", "sec"),
            ("access_token", "tok"),
        ],
        &cipher(SALT),
    );
    seed_row(&harness, "srv-1", Some("oauth2"), Some(&seeded)).await;
    seed_flow(&harness, "srv-1", "https://a", Some("https://issuer")).await;
    let server = updated_row(
        &store,
        r#"{"server_id": "srv-1", "auth_type": "api_key",
            "credentials": {"auth_value": "new-key"}}"#,
    )
    .await;
    assert_eq!(server.issuer, None);
    assert_eq!(server.token_url, None);
    assert_eq!(server.oauth2_flow, None);
    assert_eq!(server.token_exchange_endpoint, None);
    assert_eq!(server.audience, None);
    let column = harness.raw_column(CREDENTIALS_SQL, "srv-1").await.unwrap();
    assert_eq!(blob_keys(&column), ["auth_value"]);
    assert_eq!(blob_text(&column, "auth_value").as_deref(), Some("new-key"));
    harness.close().await;
}

#[tokio::test]
async fn update_auth_type_change_without_credentials_nulls_the_blob() {
    let harness = database().await;
    let store = harness.writes();
    let seeded = seeded_credentials(&[("client_id", "cid")], &cipher(SALT));
    seed_row(&harness, "srv-1", Some("oauth2"), Some(&seeded)).await;
    let server = updated_row(&store, r#"{"server_id": "srv-1", "auth_type": "api_key"}"#).await;
    assert!(server.credentials.is_none());
    assert_eq!(harness.raw_column(CREDENTIALS_SQL, "srv-1").await, None);
    harness.close().await;
}

// The edit form re-sends every field, so a resubmitted stale issuer must not survive the switch.
#[tokio::test]
async fn update_clears_a_resubmitted_stale_issuer_but_keeps_a_new_one() {
    let harness = database().await;
    let store = harness.writes();
    seed_row(&harness, "srv-1", Some("oauth2"), None).await;
    seed_flow(&harness, "srv-1", "https://a", Some("https://old")).await;
    let resubmitted = updated_row(
        &store,
        r#"{"server_id": "srv-1", "auth_type": "api_key", "issuer": "https://old",
            "token_url": "https://a/token"}"#,
    )
    .await;
    assert_eq!(resubmitted.issuer, None);
    assert_eq!(resubmitted.token_url, None);
    let renewed = updated_row(
        &store,
        r#"{"server_id": "srv-1", "auth_type": "api_key", "issuer": "https://new"}"#,
    )
    .await;
    assert_eq!(renewed.issuer.as_deref(), Some("https://new"));
    harness.close().await;
}

#[tokio::test]
async fn update_url_change_clears_flow_fields_only_when_the_url_moves() {
    let harness = database().await;
    let store = harness.writes();
    seed_row(&harness, "srv-1", Some("oauth2"), None).await;
    seed_flow(&harness, "srv-1", "https://a", Some("https://old")).await;
    let moved = updated_row(&store, r#"{"server_id": "srv-1", "url": "https://b"}"#).await;
    assert_eq!(moved.url.as_deref(), Some("https://b"));
    assert_eq!(moved.issuer, None);
    assert_eq!(moved.audience, None);
    seed_row(&harness, "srv-2", Some("oauth2"), None).await;
    seed_flow(&harness, "srv-2", "https://same", Some("https://kept")).await;
    let kept = updated_row(&store, r#"{"server_id": "srv-2", "url": "https://same"}"#).await;
    assert_eq!(kept.issuer.as_deref(), Some("https://kept"));
    assert_eq!(kept.audience.as_deref(), Some("the-audience"));
    harness.close().await;
}

#[tokio::test]
async fn update_issuer_change_requires_a_stored_issuer_to_clear() {
    let harness = database().await;
    let store = harness.writes();
    seed_row(&harness, "srv-1", Some("oauth2"), None).await;
    seed_flow(&harness, "srv-1", "https://a", None).await;
    let discovered = updated_row(
        &store,
        r#"{"server_id": "srv-1", "issuer": "https://first-discovery"}"#,
    )
    .await;
    assert_eq!(
        discovered.issuer.as_deref(),
        Some("https://first-discovery")
    );
    assert_eq!(discovered.token_url.as_deref(), Some("https://a/token"));
    let moved = updated_row(
        &store,
        r#"{"server_id": "srv-1", "issuer": "https://moved"}"#,
    )
    .await;
    assert_eq!(moved.issuer.as_deref(), Some("https://moved"));
    assert_eq!(moved.token_url, None);
    harness.close().await;
}

// Clearing a stored issuer is an issuer change too, so the endpoints discovered from it go.
#[tokio::test]
async fn update_clearing_the_issuer_clears_the_discovered_endpoints() {
    let harness = database().await;
    let store = harness.writes();
    seed_row(&harness, "srv-1", Some("oauth2"), None).await;
    seed_flow(&harness, "srv-1", "https://a", Some("https://old")).await;
    let cleared = updated_row(&store, r#"{"server_id": "srv-1", "issuer": null}"#).await;
    assert_eq!(cleared.issuer, None);
    assert_eq!(cleared.token_url, None);
    harness.close().await;
}

#[tokio::test]
async fn update_merges_partial_credentials_and_keeps_minted_tokens_without_a_rotation() {
    let harness = database().await;
    let store = harness.writes();
    let seeded = seeded_credentials(
        &[
            ("auth_value", "old-secret"),
            ("client_id", "cid-old"),
            ("aws_region_name", "eu-west-1"),
            ("access_token", "minted"),
        ],
        &cipher(SALT),
    );
    seed_row(&harness, "srv-1", Some("api_key"), Some(&seeded)).await;
    updated_row(
        &store,
        r#"{"server_id": "srv-1", "credentials": {"auth_value": "new-secret"}}"#,
    )
    .await;
    let column = harness.raw_column(CREDENTIALS_SQL, "srv-1").await.unwrap();
    assert_eq!(
        blob_text(&column, "auth_value").as_deref(),
        Some("new-secret")
    );
    assert_eq!(blob_text(&column, "client_id").as_deref(), Some("cid-old"));
    assert_eq!(
        blob_text(&column, "aws_region_name").as_deref(),
        Some("eu-west-1")
    );
    assert_eq!(
        blob_text(&column, "access_token").as_deref(),
        Some("minted")
    );
    harness.close().await;
}

#[tokio::test]
async fn update_client_rotation_drops_minted_tokens() {
    let harness = database().await;
    let store = harness.writes();
    let seeded = seeded_credentials(
        &[
            ("client_id", "cid-old"),
            ("client_secret", "sec-old"),
            ("access_token", "minted"),
            ("refresh_token", "refreshing"),
            ("expires_in", "3600"),
        ],
        &cipher(SALT),
    );
    seed_row(&harness, "srv-1", Some("oauth2"), Some(&seeded)).await;
    updated_row(
        &store,
        r#"{"server_id": "srv-1", "credentials": {"client_id": "cid-new",
            "client_secret": "sec-new"}}"#,
    )
    .await;
    let column = harness.raw_column(CREDENTIALS_SQL, "srv-1").await.unwrap();
    assert_eq!(blob_keys(&column), ["client_id", "client_secret"]);
    assert_eq!(blob_text(&column, "client_id").as_deref(), Some("cid-new"));
    harness.close().await;
}

// A switch inside the client-forwarded class keeps the declared app, so it merges instead of resetting.
#[tokio::test]
async fn update_within_client_forwarded_class_merges_credentials() {
    let harness = database().await;
    let store = harness.writes();
    let seeded = seeded_credentials(
        &[("client_id", "cid"), ("client_secret", "sec")],
        &cipher(SALT),
    );
    seed_row(&harness, "srv-1", Some("true_passthrough"), Some(&seeded)).await;
    updated_row(
        &store,
        r#"{"server_id": "srv-1", "auth_type": "oauth_delegate",
            "credentials": {"client_secret": "sec2"}}"#,
    )
    .await;
    let column = harness.raw_column(CREDENTIALS_SQL, "srv-1").await.unwrap();
    assert_eq!(blob_text(&column, "client_id").as_deref(), Some("cid"));
    assert_eq!(blob_text(&column, "client_secret").as_deref(), Some("sec2"));
    assert_eq!(
        harness.raw_column(AUTH_TYPE_SQL, "srv-1").await.as_deref(),
        Some("oauth_delegate")
    );
    harness.close().await;
}

#[tokio::test]
async fn update_lifts_legacy_token_exchange_settings_out_of_the_blob() {
    let harness = database().await;
    let store = harness.writes();
    let seeded = seeded_credentials(
        &[
            ("client_id", "cid"),
            ("token_exchange_endpoint", "https://te"),
            ("audience", "aud"),
        ],
        &cipher(SALT),
    );
    seed_row(&harness, "srv-1", Some("oauth2"), Some(&seeded)).await;
    let server = updated_row(
        &store,
        r#"{"server_id": "srv-1", "credentials": {"client_secret": "sec"}}"#,
    )
    .await;
    assert_eq!(
        server.token_exchange_endpoint.as_deref(),
        Some("https://te")
    );
    assert_eq!(server.audience.as_deref(), Some("aud"));
    let column = harness.raw_column(CREDENTIALS_SQL, "srv-1").await.unwrap();
    assert_eq!(blob_keys(&column), ["client_id", "client_secret"]);
    harness.close().await;
}

// A stored column wins over the legacy blob copy, which is still stripped from the blob.
#[tokio::test]
async fn update_keeps_a_stored_token_exchange_column_over_the_blob_copy() {
    let harness = database().await;
    let store = harness.writes();
    let seeded = seeded_credentials(
        &[
            ("client_id", "cid"),
            ("token_exchange_endpoint", "https://legacy"),
        ],
        &cipher(SALT),
    );
    seed_row(&harness, "srv-1", Some("oauth2"), Some(&seeded)).await;
    seed_flow(&harness, "srv-1", "https://a", None).await;
    updated_row(
        &store,
        r#"{"server_id": "srv-1", "credentials": {"client_secret": "sec"}}"#,
    )
    .await;
    assert_eq!(
        token_exchange_column(&harness).await.as_deref(),
        Some("https://exchange")
    );
    let column = harness.raw_column(CREDENTIALS_SQL, "srv-1").await.unwrap();
    assert_eq!(blob_keys(&column), ["client_id", "client_secret"]);
    harness.close().await;
}

// A column the caller cleared stays cleared: the blob copy is stripped so it cannot come back.
#[tokio::test]
async fn update_cleared_token_exchange_columns_do_not_repopulate() {
    let harness = database().await;
    let store = harness.writes();
    let seeded = seeded_credentials(
        &[
            ("client_id", "cid"),
            ("token_exchange_endpoint", "https://legacy"),
        ],
        &cipher(SALT),
    );
    seed_row(&harness, "srv-1", Some("oauth2"), Some(&seeded)).await;
    updated_row(
        &store,
        r#"{"server_id": "srv-1", "token_exchange_endpoint": null}"#,
    )
    .await;
    assert_eq!(token_exchange_column(&harness).await, None);
    let column = harness.raw_column(CREDENTIALS_SQL, "srv-1").await.unwrap();
    assert_eq!(blob_keys(&column), ["client_id"]);
    updated_row(
        &store,
        r#"{"server_id": "srv-1", "credentials": {"client_secret": "sec"}}"#,
    )
    .await;
    assert_eq!(token_exchange_column(&harness).await, None);
    harness.close().await;
}

// A token-exchange column write without legacy blob copies leaves the stored credentials as they are.
#[tokio::test]
async fn update_token_exchange_write_without_blob_copies_keeps_the_blob() {
    let harness = database().await;
    let store = harness.writes();
    let seeded = seeded_credentials(&[("client_id", "cid")], &cipher(SALT));
    seed_row(&harness, "srv-1", Some("oauth2"), Some(&seeded)).await;
    let before = harness.raw_column(CREDENTIALS_SQL, "srv-1").await;
    updated_row(
        &store,
        r#"{"server_id": "srv-1", "audience": "https://aud"}"#,
    )
    .await;
    assert_eq!(harness.raw_column(CREDENTIALS_SQL, "srv-1").await, before);
    harness.close().await;
}

// Only a token-exchange column the caller sent migrates the blob; the nulls a url change writes do not.
#[tokio::test]
async fn update_url_change_alone_leaves_the_legacy_blob_untouched() {
    let harness = database().await;
    let store = harness.writes();
    let seeded = seeded_credentials(
        &[
            ("client_id", "cid"),
            ("token_exchange_endpoint", "https://legacy"),
        ],
        &cipher(SALT),
    );
    seed_row(&harness, "srv-1", Some("oauth2"), Some(&seeded)).await;
    seed_flow(&harness, "srv-1", "https://a", Some("https://old")).await;
    let server = updated_row(&store, r#"{"server_id": "srv-1", "url": "https://b"}"#).await;
    assert_eq!(server.token_exchange_endpoint, None);
    let column = harness.raw_column(CREDENTIALS_SQL, "srv-1").await.unwrap();
    assert_eq!(blob_keys(&column), ["client_id", "token_exchange_endpoint"]);
    harness.close().await;
}
