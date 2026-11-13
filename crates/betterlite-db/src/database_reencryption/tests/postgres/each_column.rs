use betterlite_secrets::secret_codec::Algorithm;

use super::super::fixtures::{cipher, column, sealed, MASTER_KEY, PREVIOUS_KEY};
use super::support::{database, master, migrate, plaintexts, previous};
use crate::database_reencryption::counting::count_values_encrypted_with;
use crate::database_reencryption::outcome::MigrationOutcome;
use crate::database_reencryption::registry::SecretColumn;

fn stored_values(secret_column: &SecretColumn, plaintext: &str) -> [String; 2] {
    let legacy = sealed(plaintext, PREVIOUS_KEY, Algorithm::XSalsa20Poly1305);
    let aes = sealed(plaintext, PREVIOUS_KEY, Algorithm::Aes256Gcm);
    let values = [legacy, aes];
    match (secret_column.is_json, secret_column.marked_only) {
        (false, _) => values,
        (true, false) => values.map(|value| format!("{{\"secret\":\"{value}\"}}")),
        (true, true) => values.map(|value| format!("{{\"secret\":\"litellm_enc::{value}\"}}")),
    }
}

async fn migrates_both_codecs(secret_column: SecretColumn, plaintext: &str) {
    let database = database().await;
    database.create_table(&secret_column).await;
    for (index, value) in stored_values(&secret_column, plaintext).iter().enumerate() {
        database
            .insert(&secret_column, &format!("key-{index}"), value)
            .await;
    }
    let previous = previous();
    let count = count_values_encrypted_with(&database.client, &previous, 100)
        .await
        .expect("the count succeeds");
    assert_eq!(count, 2);
    let cipher = cipher(Algorithm::XSalsa20Poly1305);
    let outcome = migrate(&database, &previous, &master(), &cipher)
        .await
        .expect("the run succeeds")
        .expect("an outcome");
    assert!(matches!(
        outcome,
        MigrationOutcome::Migrated {
            migrated: 2,
            remaining: 0
        }
    ));
    for index in 0_u8..2 {
        let text = database
            .stored(&secret_column, &format!("key-{index}"))
            .await
            .expect("the row exists")
            .expect("the column is not null");
        assert_eq!(
            plaintexts(&text, secret_column.is_json, MASTER_KEY),
            vec![plaintext.to_owned()]
        );
        assert!(plaintexts(&text, secret_column.is_json, PREVIOUS_KEY).is_empty());
    }
    database.close().await;
}

#[tokio::test]
async fn postgres_each_secret_column_proxy_model_table_litellm_params() {
    Box::pin(migrates_both_codecs(
        column("LiteLLM_ProxyModelTable", "litellm_params"),
        "proxy",
    ))
    .await;
}

#[tokio::test]
async fn postgres_each_secret_column_credentials_table_credential_values() {
    Box::pin(migrates_both_codecs(
        column("LiteLLM_CredentialsTable", "credential_values"),
        "credentials",
    ))
    .await;
}

#[tokio::test]
async fn postgres_each_secret_column_config_param_value() {
    Box::pin(migrates_both_codecs(
        column("LiteLLM_Config", "param_value"),
        "config",
    ))
    .await;
}

#[tokio::test]
async fn postgres_each_secret_column_sso_config_sso_settings() {
    Box::pin(migrates_both_codecs(
        column("LiteLLM_SSOConfig", "sso_settings"),
        "sso",
    ))
    .await;
}

#[tokio::test]
async fn postgres_each_secret_column_cache_config_cache_settings() {
    Box::pin(migrates_both_codecs(
        column("LiteLLM_CacheConfig", "cache_settings"),
        "cache",
    ))
    .await;
}

#[tokio::test]
async fn postgres_each_secret_column_config_overrides_config_value() {
    Box::pin(migrates_both_codecs(
        column("LiteLLM_ConfigOverrides", "config_value"),
        "override",
    ))
    .await;
}

#[tokio::test]
async fn postgres_each_secret_column_mcp_server_credentials() {
    Box::pin(migrates_both_codecs(
        column("LiteLLM_MCPServerTable", "credentials"),
        "mcp-credentials",
    ))
    .await;
}

#[tokio::test]
async fn postgres_each_secret_column_mcp_server_static_headers() {
    Box::pin(migrates_both_codecs(
        column("LiteLLM_MCPServerTable", "static_headers"),
        "mcp-headers",
    ))
    .await;
}

#[tokio::test]
async fn postgres_each_secret_column_mcp_server_env_vars() {
    Box::pin(migrates_both_codecs(
        column("LiteLLM_MCPServerTable", "env_vars"),
        "mcp-env-vars",
    ))
    .await;
}

#[tokio::test]
async fn postgres_each_secret_column_mcp_server_env() {
    Box::pin(migrates_both_codecs(
        column("LiteLLM_MCPServerTable", "env"),
        "mcp-env",
    ))
    .await;
}

#[tokio::test]
async fn postgres_each_secret_column_mcp_server_oauth_client_credentials() {
    Box::pin(migrates_both_codecs(
        column("LiteLLM_MCPServerOAuthClient", "credentials"),
        "mcp-oauth",
    ))
    .await;
}

#[tokio::test]
async fn postgres_each_secret_column_mcp_user_credentials_credential_b64() {
    Box::pin(migrates_both_codecs(
        column("LiteLLM_MCPUserCredentials", "credential_b64"),
        "mcp-user",
    ))
    .await;
}

#[tokio::test]
async fn postgres_each_secret_column_mcp_user_env_vars_values_b64() {
    Box::pin(migrates_both_codecs(
        column("LiteLLM_MCPUserEnvVars", "values_b64"),
        "mcp-user-env",
    ))
    .await;
}

#[tokio::test]
async fn postgres_each_secret_column_sso_identity_assertion_assertion_b64() {
    Box::pin(migrates_both_codecs(
        column("LiteLLM_SSOIdentityAssertion", "assertion_b64"),
        "assertion",
    ))
    .await;
}

#[tokio::test]
async fn postgres_each_secret_column_team_table_metadata() {
    Box::pin(migrates_both_codecs(
        column("LiteLLM_TeamTable", "metadata"),
        "team",
    ))
    .await;
}

#[tokio::test]
async fn postgres_each_secret_column_verification_token_metadata() {
    Box::pin(migrates_both_codecs(
        column("LiteLLM_VerificationToken", "metadata"),
        "token",
    ))
    .await;
}

#[tokio::test]
async fn postgres_each_secret_column_user_table_metadata() {
    Box::pin(migrates_both_codecs(
        column("LiteLLM_UserTable", "metadata"),
        "user",
    ))
    .await;
}

#[tokio::test]
async fn postgres_each_secret_column_deleted_team_table_metadata() {
    Box::pin(migrates_both_codecs(
        column("LiteLLM_DeletedTeamTable", "metadata"),
        "deleted-team",
    ))
    .await;
}

#[tokio::test]
async fn postgres_each_secret_column_deleted_verification_token_metadata() {
    Box::pin(migrates_both_codecs(
        column("LiteLLM_DeletedVerificationToken", "metadata"),
        "deleted-token",
    ))
    .await;
}
