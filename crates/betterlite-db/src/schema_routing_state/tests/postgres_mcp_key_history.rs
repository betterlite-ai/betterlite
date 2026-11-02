#[derive(Clone, Copy)]
enum McpKeyHistoryCohort {
    DelegationAndOauthFalse,
    EnvVarDefinitionsArrayAndRawUserValues,
    McpUserEnvUniquePairAndIndexes,
    NullableOauthExchangeIssuerFields,
    UnguardedNullableColumnsRepeat42701,
    KeyTypeNullableBothTables,
    BudgetFallbacksEmptyBothTables,
    SettingsUpdatedAtNullableNoBackfill,
    OauthClientRawJsonAndTimestampDefaults,
    SsoAssertionRawTextAndTimestampDefaults,
    JwtKeyDeleteNowCascadesOnlyItsMappings,
}

fn mcp_key_history_checkpoint_for(cohort: McpKeyHistoryCohort) -> &'static str {
    match cohort {
        McpKeyHistoryCohort::DelegationAndOauthFalse => ADD_MCP_OAUTH_PASSTHROUGH.directory,
        McpKeyHistoryCohort::EnvVarDefinitionsArrayAndRawUserValues
        | McpKeyHistoryCohort::McpUserEnvUniquePairAndIndexes => ADD_MCP_ENV_VARS.directory,
        McpKeyHistoryCohort::NullableOauthExchangeIssuerFields => {
            ADD_ISSUER_TO_MCP_SERVER_TABLE.directory
        }
        McpKeyHistoryCohort::UnguardedNullableColumnsRepeat42701 => {
            ADD_MCP_SERVER_TIMEOUT.directory
        }
        McpKeyHistoryCohort::KeyTypeNullableBothTables => ADD_TOKEN_KEY_TYPE.directory,
        McpKeyHistoryCohort::BudgetFallbacksEmptyBothTables => ADD_TOKEN_BUDGET_FALLBACKS.directory,
        McpKeyHistoryCohort::SettingsUpdatedAtNullableNoBackfill => {
            ADD_AUTOROUTER_TIER_TURNS.directory
        }
        McpKeyHistoryCohort::OauthClientRawJsonAndTimestampDefaults => {
            ADD_MCP_OAUTH_CLIENT.directory
        }
        McpKeyHistoryCohort::SsoAssertionRawTextAndTimestampDefaults => {
            ADD_SSO_IDENTITY_ASSERTION.directory
        }
        McpKeyHistoryCohort::JwtKeyDeleteNowCascadesOnlyItsMappings => {
            JWT_KEY_CASCADE_DELETE.directory
        }
    }
}

async fn check_delegation_and_oauth_false(client: &Client) {
    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_MCPServerTable" (server_id, url) VALUES ($1, $2)"#,
        &[&"server-a", &"https://example.test/mcp"],
    )
    .await;
    assert_columns(
        client,
        "LiteLLM_MCPServerTable",
        &[
            (
                "delegate_auth_to_upstream",
                "boolean",
                false,
                Some("false"),
                "bool",
                None,
            ),
            (
                "oauth_passthrough",
                "boolean",
                false,
                Some("false"),
                "bool",
                None,
            ),
        ],
    )
    .await;
}

async fn check_env_var_definitions_array_and_raw_user_values(client: &Client) {
    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_MCPServerTable" (server_id, url) VALUES ($1, $2)"#,
        &[&"server-env", &"https://example.test/mcp"],
    )
    .await;
    assert_column(
        client,
        "LiteLLM_MCPServerTable",
        (
            "env_vars",
            "jsonb",
            true,
            Some("'[]'::jsonb"),
            "jsonb",
            None,
        ),
    )
    .await;
    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_MCPUserEnvVars" (id, user_id, server_id, values_b64) VALUES ($1, $2, $3, $4)"#,
        &[&"env-a", &"user-a", &"server-env", &"bm90LWRlY29kZWQ="],
    )
    .await;
    let stored = scalar_text(
        client,
        r#"SELECT values_b64 FROM "LiteLLM_MCPUserEnvVars" WHERE id = $1"#,
        &[&"env-a"],
    )
    .await;
    assert_eq!(stored, "bm90LWRlY29kZWQ=");
}

async fn check_mcp_user_env_unique_pair_and_indexes(client: &Client) {
    assert_repeat_insert_unique_violation(
        client,
        r#"INSERT INTO "LiteLLM_MCPUserEnvVars" (id, user_id, server_id, values_b64) VALUES ($1, $2, $3, $4)"#,
        &[&"env-first", &"user-shared", &"server-shared", &"first"],
        &[&"env-second", &"user-shared", &"server-shared", &"second"],
    )
    .await;
    assert!(index_exists(client, "LiteLLM_MCPUserEnvVars_user_id_server_id_key").await);
}

async fn check_nullable_oauth_exchange_issuer_fields(client: &Client) {
    assert_columns(
        client,
        "LiteLLM_MCPServerTable",
        &[
            ("dcr_bridge", "boolean", true, None, "bool", None),
            ("token_exchange_endpoint", "text", true, None, "text", None),
            ("audience", "text", true, None, "text", None),
            ("subject_token_type", "text", true, None, "text", None),
            ("token_exchange_profile", "text", true, None, "text", None),
            ("issuer", "text", true, None, "text", None),
        ],
    )
    .await;
}

async fn check_unguarded_nullable_columns_repeat_42701(client: &Client) {
    let repeat =
        batch_execute_err(client, migration_sql_of(ADD_MCP_SERVER_TIMEOUT.directory)).await;
    assert_eq!(sql_state(&repeat), SqlState::DUPLICATE_COLUMN.code());
}

async fn check_key_type_nullable_both_tables(client: &Client) {
    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_VerificationToken" (token) VALUES ($1)"#,
        &[&"token-key-type"],
    )
    .await;
    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_DeletedVerificationToken" (id, token) VALUES ($1, $2)"#,
        &[&"deleted-key-type", &"token-key-type-deleted"],
    )
    .await;
    assert_single_column(
        client,
        &[
            "LiteLLM_VerificationToken",
            "LiteLLM_DeletedVerificationToken",
        ],
        ("key_type", "text", true, None, "text", None),
    )
    .await;
}

async fn check_budget_fallbacks_empty_both_tables(client: &Client) {
    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_VerificationToken" (token) VALUES ($1)"#,
        &[&"token-budget-fallbacks"],
    )
    .await;
    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_DeletedVerificationToken" (id, token) VALUES ($1, $2)"#,
        &[
            &"deleted-budget-fallbacks",
            &"token-budget-fallbacks-deleted",
        ],
    )
    .await;
    assert_single_column(
        client,
        &[
            "LiteLLM_VerificationToken",
            "LiteLLM_DeletedVerificationToken",
        ],
        (
            "budget_fallbacks",
            "jsonb",
            false,
            Some("'{}'::jsonb"),
            "jsonb",
            None,
        ),
    )
    .await;
    let token_value = scalar_text(
        client,
        r#"SELECT budget_fallbacks::text FROM "LiteLLM_VerificationToken" WHERE token = $1"#,
        &[&"token-budget-fallbacks"],
    )
    .await;
    assert_eq!(token_value, "{}");
}

async fn check_settings_updated_at_nullable_no_backfill(client: &Client) {
    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_VerificationToken" (token) VALUES ($1)"#,
        &[&"token-settings-updated-at"],
    )
    .await;
    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_DeletedVerificationToken" (id, token) VALUES ($1, $2)"#,
        &[
            &"deleted-settings-updated-at",
            &"token-settings-updated-at-deleted",
        ],
    )
    .await;
    batch_execute(
        client,
        migration_sql_of(ADD_TOKEN_SETTINGS_UPDATED.directory),
    )
    .await;
    assert_single_column(
        client,
        &[
            "LiteLLM_VerificationToken",
            "LiteLLM_DeletedVerificationToken",
        ],
        (
            "settings_updated_at",
            "timestamp without time zone",
            true,
            None,
            "timestamp",
            Some(3_i32),
        ),
    )
    .await;
    let token_value = scalar_opt_text(
        client,
        r#"SELECT settings_updated_at::text FROM "LiteLLM_VerificationToken" WHERE token = $1"#,
        &[&"token-settings-updated-at"],
    )
    .await;
    assert_eq!(token_value, None);
}

async fn assert_created_updated_timestamps(client: &Client, table: &str) {
    assert_columns(
        client,
        table,
        &[
            (
                "created_at",
                "timestamp without time zone",
                false,
                Some("CURRENT_TIMESTAMP"),
                "timestamp",
                Some(3_i32),
            ),
            (
                "updated_at",
                "timestamp without time zone",
                false,
                Some("CURRENT_TIMESTAMP"),
                "timestamp",
                Some(3_i32),
            ),
        ],
    )
    .await;
}

async fn check_oauth_client_raw_json_and_timestamp_defaults(client: &Client) {
    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_MCPServerOAuthClient" (server_id, credentials) VALUES ($1, $2)"#,
        &[
            &"server-oauth",
            &serde_json::Value::String("raw-opaque-value".to_owned()),
        ],
    )
    .await;
    let stored = scalar_text(
        client,
        r#"SELECT credentials::text FROM "LiteLLM_MCPServerOAuthClient" WHERE server_id = $1"#,
        &[&"server-oauth"],
    )
    .await;
    assert_eq!(stored, r#""raw-opaque-value""#);
    assert_created_updated_timestamps(client, "LiteLLM_MCPServerOAuthClient").await;
}

async fn check_sso_assertion_raw_text_and_timestamp_defaults(client: &Client) {
    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_SSOIdentityAssertion" (user_id, assertion_b64) VALUES ($1, $2)"#,
        &[&"user-sso", &"opaque-assertion-bytes"],
    )
    .await;
    let stored = scalar_text(
        client,
        r#"SELECT assertion_b64 FROM "LiteLLM_SSOIdentityAssertion" WHERE user_id = $1"#,
        &[&"user-sso"],
    )
    .await;
    assert_eq!(stored, "opaque-assertion-bytes");
    assert_created_updated_timestamps(client, "LiteLLM_SSOIdentityAssertion").await;
}

async fn check_jwt_key_delete_now_cascades_only_its_mappings(client: &Client) {
    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_VerificationToken" (token) VALUES ($1)"#,
        &[&"token-keep"],
    )
    .await;
    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_VerificationToken" (token) VALUES ($1)"#,
        &[&"token-delete"],
    )
    .await;
    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_JWTKeyMapping" (id, jwt_claim_name, jwt_claim_value, token) VALUES ($1, $2, $3, $4)"#,
        &[&"mapping-keep", &"claim", &"value-keep", &"token-keep"],
    )
    .await;
    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_JWTKeyMapping" (id, jwt_claim_name, jwt_claim_value, token) VALUES ($1, $2, $3, $4)"#,
        &[&"mapping-delete", &"claim", &"value-delete", &"token-delete"],
    )
    .await;
    exec_params(
        client,
        r#"DELETE FROM "LiteLLM_VerificationToken" WHERE token = $1"#,
        &[&"token-delete"],
    )
    .await;
    let remaining = string_rows(
        client,
        r#"SELECT id FROM "LiteLLM_JWTKeyMapping" ORDER BY id"#,
        &[],
    )
    .await;
    assert_eq!(remaining, vec!["mapping-keep".to_owned()]);
}

async fn run_mcp_key_history_check(client: &Client, cohort: McpKeyHistoryCohort) {
    match cohort {
        McpKeyHistoryCohort::DelegationAndOauthFalse => {
            check_delegation_and_oauth_false(client).await;
        }
        McpKeyHistoryCohort::EnvVarDefinitionsArrayAndRawUserValues => {
            check_env_var_definitions_array_and_raw_user_values(client).await;
        }
        McpKeyHistoryCohort::McpUserEnvUniquePairAndIndexes => {
            check_mcp_user_env_unique_pair_and_indexes(client).await;
        }
        McpKeyHistoryCohort::NullableOauthExchangeIssuerFields => {
            check_nullable_oauth_exchange_issuer_fields(client).await;
        }
        McpKeyHistoryCohort::UnguardedNullableColumnsRepeat42701 => {
            check_unguarded_nullable_columns_repeat_42701(client).await;
        }
        McpKeyHistoryCohort::KeyTypeNullableBothTables => {
            check_key_type_nullable_both_tables(client).await;
        }
        McpKeyHistoryCohort::BudgetFallbacksEmptyBothTables => {
            check_budget_fallbacks_empty_both_tables(client).await;
        }
        McpKeyHistoryCohort::SettingsUpdatedAtNullableNoBackfill => {
            check_settings_updated_at_nullable_no_backfill(client).await;
        }
        McpKeyHistoryCohort::OauthClientRawJsonAndTimestampDefaults => {
            check_oauth_client_raw_json_and_timestamp_defaults(client).await;
        }
        McpKeyHistoryCohort::SsoAssertionRawTextAndTimestampDefaults => {
            check_sso_assertion_raw_text_and_timestamp_defaults(client).await;
        }
        McpKeyHistoryCohort::JwtKeyDeleteNowCascadesOnlyItsMappings => {
            check_jwt_key_delete_now_cascades_only_its_mappings(client).await;
        }
    }
}

#[rstest]
#[case::delegation_and_oauth_false(McpKeyHistoryCohort::DelegationAndOauthFalse)]
#[case::env_var_definitions_array_and_raw_user_values(
    McpKeyHistoryCohort::EnvVarDefinitionsArrayAndRawUserValues
)]
#[case::mcp_user_env_unique_pair_and_indexes(McpKeyHistoryCohort::McpUserEnvUniquePairAndIndexes)]
#[case::nullable_oauth_exchange_issuer_fields(
    McpKeyHistoryCohort::NullableOauthExchangeIssuerFields
)]
#[case::unguarded_nullable_columns_repeat_42701(
    McpKeyHistoryCohort::UnguardedNullableColumnsRepeat42701
)]
#[case::key_type_nullable_both_tables(McpKeyHistoryCohort::KeyTypeNullableBothTables)]
#[case::budget_fallbacks_empty_both_tables(McpKeyHistoryCohort::BudgetFallbacksEmptyBothTables)]
#[case::settings_updated_at_nullable_no_backfill(
    McpKeyHistoryCohort::SettingsUpdatedAtNullableNoBackfill
)]
#[case::oauth_client_raw_json_and_timestamp_defaults(
    McpKeyHistoryCohort::OauthClientRawJsonAndTimestampDefaults
)]
#[case::sso_assertion_raw_text_and_timestamp_defaults(
    McpKeyHistoryCohort::SsoAssertionRawTextAndTimestampDefaults
)]
#[case::jwt_key_delete_now_cascades_only_its_mappings(
    McpKeyHistoryCohort::JwtKeyDeleteNowCascadesOnlyItsMappings
)]
#[tokio::test]
async fn postgres_mcp_key_history(#[case] cohort: McpKeyHistoryCohort) {
    Box::pin(harness(move |[client]| async move {
        apply_migrations_through(&client, mcp_key_history_checkpoint_for(cohort)).await;
        run_mcp_key_history_check(&client, cohort).await;
    }))
    .await;
}
