use betterlite_db::mcp_stored_contracts::JsonValue;

use super::support::cipher::{SALT, cipher, opened_credentials, seeded_credentials};
use super::support::database::{Harness, SEED_SERVER_SQL, database};
use super::support::update_payload;
use crate::mcp::server_writes::UpdatedServer;
use crate::mcp::server_writes::update::update_mcp_server;

const CREDENTIALS_SQL: &str =
    r#"SELECT "credentials"::text FROM "LiteLLM_MCPServerTable" WHERE "server_id" = $1"#;
const AUTH_TYPE_SQL: &str =
    r#"SELECT "auth_type"::text FROM "LiteLLM_MCPServerTable" WHERE "server_id" = $1"#;
const FLOW_SQL: &str = r#"UPDATE "LiteLLM_MCPServerTable" SET "url" = $2, "issuer" = $3, "token_url" = $4, "oauth2_flow" = $5, "token_exchange_endpoint" = $6, "audience" = $7 WHERE "server_id" = $1"#;
const TOKEN_EXCHANGE_SQL: &str = r#"SELECT "token_exchange_endpoint"::text, "audience"::text FROM "LiteLLM_MCPServerTable" WHERE "server_id" = $1"#;

async fn seed_row(
    harness: &Harness,
    server_id: &str,
    auth_type: Option<&str>,
    credentials: Option<String>,
) {
    let server_name = format!("name-{server_id}");
    let auth_type = auth_type.map(str::to_owned);
    harness
        .execute(
            SEED_SERVER_SQL,
            &[
                &server_id.to_owned(),
                &server_name,
                &auth_type,
                &credentials,
            ],
        )
        .await;
}

async fn seed_flow(harness: &Harness, server_id: &str, url: &str, issuer: Option<&str>) {
    harness
        .execute(
            FLOW_SQL,
            &[
                &server_id.to_owned(),
                &url.to_owned(),
                &issuer.map(str::to_owned),
                &Some(format!("{url}/token")),
                &Some("client_credentials".to_owned()),
                &Some("https://exchange".to_owned()),
                &Some("the-audience".to_owned()),
            ],
        )
        .await;
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
        Some(JsonValue::Null)
        | Some(JsonValue::Bool(_))
        | Some(JsonValue::Number(_))
        | Some(JsonValue::Array(_))
        | Some(JsonValue::Object(_))
        | None => None,
    }
}

#[tokio::test]
async fn update_auth_type_change_replaces_cross_class_credentials_and_clears_flow_fields() {
    let harness = database().await;
    let store = harness.writes();
    let cipher = cipher(SALT);
    let seeded = seeded_credentials(
        &[
            ("client_id", "cid"),
            ("client_secret", "sec"),
            ("access_token", "tok"),
        ],
        &cipher,
    );
    seed_row(&harness, "srv-1", Some("oauth2"), Some(seeded)).await;
    seed_flow(&harness, "srv-1", "https://a", Some("https://issuer")).await;
    let (request, fields) = update_payload(
        r#"{"server_id": "srv-1", "auth_type": "api_key",
            "credentials": {"auth_value": "new-key"}}"#,
    );
    let updated = update_mcp_server(&store, &cipher, &request, "editor", &fields)
        .await
        .unwrap();
    let UpdatedServer::Updated(Some(server)) = updated else {
        panic!("the row exists");
    };
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
    let cipher = cipher(SALT);
    let seeded = seeded_credentials(&[("client_id", "cid")], &cipher);
    seed_row(&harness, "srv-1", Some("oauth2"), Some(seeded)).await;
    let (request, fields) = update_payload(r#"{"server_id": "srv-1", "auth_type": "api_key"}"#);
    let updated = update_mcp_server(&store, &cipher, &request, "editor", &fields)
        .await
        .unwrap();
    let UpdatedServer::Updated(Some(server)) = updated else {
        panic!("the row exists");
    };
    assert_eq!(server.credentials, None);
    assert_eq!(harness.raw_column(CREDENTIALS_SQL, "srv-1").await, None);
    harness.close().await;
}

// The edit form re-sends every field, so a resubmitted stale issuer must not survive the switch.
#[tokio::test]
async fn update_clears_a_resubmitted_stale_issuer_but_keeps_a_new_one() {
    let harness = database().await;
    let store = harness.writes();
    let cipher = cipher(SALT);
    seed_row(&harness, "srv-1", Some("oauth2"), None).await;
    seed_flow(&harness, "srv-1", "https://a", Some("https://old")).await;
    let (request, fields) = update_payload(
        r#"{"server_id": "srv-1", "auth_type": "api_key", "issuer": "https://old",
            "token_url": "https://a/token"}"#,
    );
    let updated = update_mcp_server(&store, &cipher, &request, "editor", &fields)
        .await
        .unwrap();
    let UpdatedServer::Updated(Some(server)) = updated else {
        panic!("the row exists");
    };
    assert_eq!(server.issuer, None);
    assert_eq!(server.token_url, None);
    let (request, fields) = update_payload(
        r#"{"server_id": "srv-1", "auth_type": "api_key", "issuer": "https://new"}"#,
    );
    let updated = update_mcp_server(&store, &cipher, &request, "editor", &fields)
        .await
        .unwrap();
    let UpdatedServer::Updated(Some(server)) = updated else {
        panic!("the row exists");
    };
    assert_eq!(server.issuer.as_deref(), Some("https://new"));
    harness.close().await;
}

#[tokio::test]
async fn update_url_change_clears_flow_fields_only_when_the_url_moves() {
    let harness = database().await;
    let store = harness.writes();
    let cipher = cipher(SALT);
    seed_row(&harness, "srv-1", Some("oauth2"), None).await;
    seed_flow(&harness, "srv-1", "https://a", Some("https://old")).await;
    let (request, fields) = update_payload(r#"{"server_id": "srv-1", "url": "https://b"}"#);
    let updated = update_mcp_server(&store, &cipher, &request, "editor", &fields)
        .await
        .unwrap();
    let UpdatedServer::Updated(Some(server)) = updated else {
        panic!("the row exists");
    };
    assert_eq!(server.url.as_deref(), Some("https://b"));
    assert_eq!(server.issuer, None);
    assert_eq!(server.audience, None);
    seed_row(&harness, "srv-2", Some("oauth2"), None).await;
    seed_flow(&harness, "srv-2", "https://same", Some("https://kept")).await;
    let (request, fields) = update_payload(r#"{"server_id": "srv-2", "url": "https://same"}"#);
    let updated = update_mcp_server(&store, &cipher, &request, "editor", &fields)
        .await
        .unwrap();
    let UpdatedServer::Updated(Some(server)) = updated else {
        panic!("the row exists");
    };
    assert_eq!(server.issuer.as_deref(), Some("https://kept"));
    harness.close().await;
}

#[tokio::test]
async fn update_issuer_change_requires_a_stored_issuer_to_clear() {
    let harness = database().await;
    let store = harness.writes();
    let cipher = cipher(SALT);
    seed_row(&harness, "srv-1", Some("oauth2"), None).await;
    seed_flow(&harness, "srv-1", "https://a", None).await;
    let (request, fields) =
        update_payload(r#"{"server_id": "srv-1", "issuer": "https://first-discovery"}"#);
    let updated = update_mcp_server(&store, &cipher, &request, "editor", &fields)
        .await
        .unwrap();
    let UpdatedServer::Updated(Some(server)) = updated else {
        panic!("the row exists");
    };
    assert_eq!(server.issuer.as_deref(), Some("https://first-discovery"));
    assert_eq!(server.token_url.as_deref(), Some("https://a/token"));
    let (request, fields) = update_payload(r#"{"server_id": "srv-1", "issuer": "https://moved"}"#);
    let updated = update_mcp_server(&store, &cipher, &request, "editor", &fields)
        .await
        .unwrap();
    let UpdatedServer::Updated(Some(server)) = updated else {
        panic!("the row exists");
    };
    assert_eq!(server.issuer.as_deref(), Some("https://moved"));
    assert_eq!(server.token_url, None);
    harness.close().await;
}

#[tokio::test]
async fn update_merges_partial_credentials_and_keeps_minted_tokens_without_a_rotation() {
    let harness = database().await;
    let store = harness.writes();
    let cipher = cipher(SALT);
    let seeded = seeded_credentials(
        &[
            ("auth_value", "old-secret"),
            ("client_id", "cid-old"),
            ("aws_region_name", "eu-west-1"),
            ("access_token", "minted"),
        ],
        &cipher,
    );
    seed_row(&harness, "srv-1", Some("api_key"), Some(seeded)).await;
    let (request, fields) =
        update_payload(r#"{"server_id": "srv-1", "credentials": {"auth_value": "new-secret"}}"#);
    update_mcp_server(&store, &cipher, &request, "editor", &fields)
        .await
        .unwrap();
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
    let cipher = cipher(SALT);
    let seeded = seeded_credentials(
        &[
            ("client_id", "cid-old"),
            ("client_secret", "sec-old"),
            ("access_token", "minted"),
            ("refresh_token", "refreshing"),
            ("expires_in", "3600"),
        ],
        &cipher,
    );
    seed_row(&harness, "srv-1", Some("oauth2"), Some(seeded)).await;
    let (request, fields) = update_payload(
        r#"{"server_id": "srv-1", "credentials": {"client_id": "cid-new",
            "client_secret": "sec-new"}}"#,
    );
    update_mcp_server(&store, &cipher, &request, "editor", &fields)
        .await
        .unwrap();
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
    let cipher = cipher(SALT);
    let seeded = seeded_credentials(&[("client_id", "cid"), ("client_secret", "sec")], &cipher);
    seed_row(&harness, "srv-1", Some("true_passthrough"), Some(seeded)).await;
    let (request, fields) = update_payload(
        r#"{"server_id": "srv-1", "auth_type": "oauth_delegate",
            "credentials": {"client_secret": "sec2"}}"#,
    );
    let updated = update_mcp_server(&store, &cipher, &request, "editor", &fields)
        .await
        .unwrap();
    assert!(matches!(updated, UpdatedServer::Updated(Some(_))));
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
    let cipher = cipher(SALT);
    let seeded = seeded_credentials(
        &[
            ("client_id", "cid"),
            ("token_exchange_endpoint", "https://te"),
            ("audience", "aud"),
        ],
        &cipher,
    );
    seed_row(&harness, "srv-1", Some("oauth2"), Some(seeded)).await;
    let (request, fields) =
        update_payload(r#"{"server_id": "srv-1", "credentials": {"client_secret": "sec"}}"#);
    let updated = update_mcp_server(&store, &cipher, &request, "editor", &fields)
        .await
        .unwrap();
    let UpdatedServer::Updated(Some(server)) = updated else {
        panic!("the row exists");
    };
    assert_eq!(
        server.token_exchange_endpoint.as_deref(),
        Some("https://te")
    );
    assert_eq!(server.audience.as_deref(), Some("aud"));
    let column = harness.raw_column(CREDENTIALS_SQL, "srv-1").await.unwrap();
    assert_eq!(blob_keys(&column), ["client_id", "client_secret"]);
    harness.close().await;
}

// A column the caller cleared stays cleared: the blob copy is stripped so it cannot come back.
#[tokio::test]
async fn update_cleared_token_exchange_columns_do_not_repopulate() {
    let harness = database().await;
    let store = harness.writes();
    let cipher = cipher(SALT);
    let seeded = seeded_credentials(
        &[
            ("client_id", "cid"),
            ("token_exchange_endpoint", "https://legacy"),
        ],
        &cipher,
    );
    seed_row(&harness, "srv-1", Some("oauth2"), Some(seeded)).await;
    let (request, fields) =
        update_payload(r#"{"server_id": "srv-1", "token_exchange_endpoint": null}"#);
    update_mcp_server(&store, &cipher, &request, "editor", &fields)
        .await
        .unwrap();
    let row = harness
        .client
        .query_one(TOKEN_EXCHANGE_SQL, &[&"srv-1".to_owned()])
        .await
        .unwrap();
    assert_eq!(row.get::<_, Option<String>>(0), None);
    let column = harness.raw_column(CREDENTIALS_SQL, "srv-1").await.unwrap();
    assert_eq!(blob_keys(&column), ["client_id"]);
    let (request, fields) =
        update_payload(r#"{"server_id": "srv-1", "credentials": {"client_secret": "sec"}}"#);
    let updated = update_mcp_server(&store, &cipher, &request, "editor", &fields)
        .await
        .unwrap();
    assert!(matches!(updated, UpdatedServer::Updated(Some(_))));
    let row = harness
        .client
        .query_one(TOKEN_EXCHANGE_SQL, &[&"srv-1".to_owned()])
        .await
        .unwrap();
    assert_eq!(row.get::<_, Option<String>>(0), None);
    harness.close().await;
}
