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
            bool_false("delegate_auth_to_upstream"),
            bool_false("oauth_passthrough"),
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
            nullable_bool("dcr_bridge"),
            nullable_text("token_exchange_endpoint"),
            nullable_text("audience"),
            nullable_text("subject_token_type"),
            nullable_text("token_exchange_profile"),
            nullable_text("issuer"),
        ],
    )
    .await;
}

async fn check_unguarded_nullable_columns_repeat_42701(client: &Client) {
    assert_batch_execute_err_state(
        client,
        migration_sql_of(ADD_MCP_SERVER_TIMEOUT.directory),
        SqlState::DUPLICATE_COLUMN,
    )
    .await;
}

async fn insert_verification_token_pair(
    client: &Client,
    token: &str,
    deleted_id: &str,
    deleted_token: &str,
) {
    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_VerificationToken" (token) VALUES ($1)"#,
        &[&token],
    )
    .await;
    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_DeletedVerificationToken" (id, token) VALUES ($1, $2)"#,
        &[&deleted_id, &deleted_token],
    )
    .await;
}

async fn check_key_type_nullable_both_tables(client: &Client) {
    insert_verification_token_pair(
        client,
        "token-key-type",
        "deleted-key-type",
        "token-key-type-deleted",
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
    insert_verification_token_pair(
        client,
        "token-budget-fallbacks",
        "deleted-budget-fallbacks",
        "token-budget-fallbacks-deleted",
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
    insert_verification_token_pair(
        client,
        "token-settings-updated-at",
        "deleted-settings-updated-at",
        "token-settings-updated-at-deleted",
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

struct RawValueCheck<'check> {
    insert_sql: &'check str,
    insert_params: &'check [&'check (dyn ToSql + Sync)],
    select_sql: &'check str,
    select_param: &'check str,
    expected: &'check str,
    table: &'check str,
}

async fn check_raw_value_with_timestamps(client: &Client, check: RawValueCheck<'_>) {
    exec_params(client, check.insert_sql, check.insert_params).await;
    let stored = scalar_text(client, check.select_sql, &[&check.select_param]).await;
    assert_eq!(stored, check.expected);
    assert_created_updated_timestamps(client, check.table).await;
}

#[derive(Clone, Copy)]
enum RawValueCohort {
    OauthClient,
    Sso,
}

fn raw_value_check_for(cohort: RawValueCohort) -> RawValueCheck<'static> {
    match cohort {
        RawValueCohort::OauthClient => RawValueCheck {
            insert_sql: r#"INSERT INTO "LiteLLM_MCPServerOAuthClient" (server_id, credentials) VALUES ($1, $2::text::jsonb)"#,
            insert_params: &[&"server-oauth", &r#""raw-opaque-value""#],
            select_sql: r#"SELECT credentials::text FROM "LiteLLM_MCPServerOAuthClient" WHERE server_id = $1"#,
            select_param: "server-oauth",
            expected: r#""raw-opaque-value""#,
            table: "LiteLLM_MCPServerOAuthClient",
        },
        RawValueCohort::Sso => RawValueCheck {
            insert_sql: r#"INSERT INTO "LiteLLM_SSOIdentityAssertion" (user_id, assertion_b64) VALUES ($1, $2)"#,
            insert_params: &[&"user-sso", &"opaque-assertion-bytes"],
            select_sql: r#"SELECT assertion_b64 FROM "LiteLLM_SSOIdentityAssertion" WHERE user_id = $1"#,
            select_param: "user-sso",
            expected: "opaque-assertion-bytes",
            table: "LiteLLM_SSOIdentityAssertion",
        },
    }
}

async fn check_oauth_client_or_sso_raw_value(client: &Client, cohort: RawValueCohort) {
    check_raw_value_with_timestamps(client, raw_value_check_for(cohort)).await;
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
            check_oauth_client_or_sso_raw_value(client, RawValueCohort::OauthClient).await;
        }
        McpKeyHistoryCohort::SsoAssertionRawTextAndTimestampDefaults => {
            check_oauth_client_or_sso_raw_value(client, RawValueCohort::Sso).await;
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
