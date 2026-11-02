const SPEND_TABLES: [(&str, &str); 6] = [
    ("LiteLLM_DailyUserSpend", "user_id"),
    ("LiteLLM_DailyOrganizationSpend", "organization_id"),
    ("LiteLLM_DailyEndUserSpend", "end_user_id"),
    ("LiteLLM_DailyAgentSpend", "agent_id"),
    ("LiteLLM_DailyTeamSpend", "team_id"),
    ("LiteLLM_DailyTagSpend", "tag"),
];

async fn insert_minimal_spend_row(
    client: &Client,
    table: &str,
    owner_column: &str,
    id: &str,
    spend: f64,
) {
    let sql = format!(
        r#"INSERT INTO "{table}" (id, "{owner_column}", date, api_key, model, spend, updated_at) VALUES ($1, $2, $3, $4, $5, $6, now())"#
    );
    exec_params(
        client,
        &sql,
        &[&id, &"owner-a", &"2026-01-01", &"key-a", &"model-a", &spend],
    )
    .await;
}

#[derive(Clone, Copy)]
enum UsageRollupsCohort {
    TeamPtuZero,
    SixCompressionTokenDefaults,
    SixThreeSavingsDefaultsPreserveExistingSpend,
    SpendToolStartIndex,
    DailyToolZeroCountersAndCompositeKey,
    ConfigRevisionZeroLastRunNull,
    ManagedObjectKeyNullTagsArray,
    GatewayZeroCountersAndDateIndex,
    BudgetWindowKeyExcludesStart,
    AutorouterFourteenIntegerCountersTokensSpendModelsTiers,
}

fn usage_rollups_checkpoint_for(cohort: UsageRollupsCohort) -> &'static str {
    match cohort {
        UsageRollupsCohort::TeamPtuZero => ADD_DAILY_TEAM_PTU.directory,
        UsageRollupsCohort::SixCompressionTokenDefaults => ADD_COMPRESSION_SAVED_TOKENS.directory,
        UsageRollupsCohort::SixThreeSavingsDefaultsPreserveExistingSpend => {
            JWT_KEY_CASCADE_DELETE.directory
        }
        UsageRollupsCohort::SpendToolStartIndex => ADD_SPEND_LOG_TOOL_INDEX.directory,
        UsageRollupsCohort::DailyToolZeroCountersAndCompositeKey => ADD_DAILY_TOOL_SPEND.directory,
        UsageRollupsCohort::ConfigRevisionZeroLastRunNull => ADD_CONFIG_RELOAD_TRACKING.directory,
        UsageRollupsCohort::ManagedObjectKeyNullTagsArray => ADD_MANAGED_OBJECT_TAGS.directory,
        UsageRollupsCohort::GatewayZeroCountersAndDateIndex => ADD_DAILY_GATEWAY_REQUESTS.directory,
        UsageRollupsCohort::BudgetWindowKeyExcludesStart => ADD_BUDGET_WINDOW_SPEND_TABLE.directory,
        UsageRollupsCohort::AutorouterFourteenIntegerCountersTokensSpendModelsTiers => {
            ADD_AUTOROUTER_TIER_TURNS.directory
        }
    }
}

async fn check_team_ptu_zero(client: &Client) {
    insert_minimal_spend_row(
        client,
        "LiteLLM_DailyTeamSpend",
        "team_id",
        "team-spend-ptu",
        1.0,
    )
    .await;
    assert_column(
        client,
        "LiteLLM_DailyTeamSpend",
        (
            "ptu_flat_cost",
            "double precision",
            false,
            Some("0.0"),
            "float8",
            None,
        ),
    )
    .await;
}

async fn seed_spend_tables(
    client: &Client,
    suffix: &str,
    spend: f64,
) -> Vec<(&'static str, String)> {
    let mut seeded = Vec::new();
    for (table, owner_column) in SPEND_TABLES {
        let id = format!("{table}-{suffix}");
        insert_minimal_spend_row(client, table, owner_column, &id, spend).await;
        seeded.push((table, id));
    }
    seeded
}

async fn check_six_compression_token_defaults(client: &Client) {
    for (table, id) in seed_spend_tables(client, "compression", 1.0).await {
        assert_column(client, table, int8_zero("compression_saved_tokens")).await;
        let value: i64 = query_one_row(
            client,
            &format!(r#"SELECT compression_saved_tokens FROM "{table}" WHERE id = $1"#),
            &[&id],
        )
        .await
        .get(0);
        assert_eq!(value, 0);
    }
}

async fn check_six_three_savings_defaults_preserve_existing_spend(client: &Client) {
    let seeded = seed_spend_tables(client, "savings", 7.5_f64).await;
    batch_execute(client, migration_sql_of(ADD_SAVINGS_SPEND.directory)).await;
    for (table, id) in seeded {
        let row = query_one_row(
            client,
            &format!(
                r#"SELECT spend, compression_savings_spend, prompt_caching_savings_spend FROM "{table}" WHERE id = $1"#
            ),
            &[&id],
        )
        .await;
        let spend: f64 = row.get(0);
        let compression_savings_spend: f64 = row.get(1);
        let prompt_caching_savings_spend: f64 = row.get(2);
        assert_eq!(
            spend.to_bits(),
            7.5_f64.to_bits(),
            "{table} spend should stay untouched"
        );
        assert_eq!(compression_savings_spend.to_bits(), 0.0_f64.to_bits());
        assert_eq!(prompt_caching_savings_spend.to_bits(), 0.0_f64.to_bits());
    }
}

async fn check_spend_tool_start_index(client: &Client) {
    assert_index(
        client,
        "LiteLLM_SpendLogToolIndex_start_time_idx",
        &["start_time"],
        false,
    )
    .await;
}

async fn check_daily_tool_zero_counters_and_composite_key(client: &Client) {
    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_DailyToolSpend" (date, tool_name, updated_at) VALUES ($1, $2, now())"#,
        &[&"2026-01-01", &"search"],
    )
    .await;
    assert_columns(
        client,
        "LiteLLM_DailyToolSpend",
        &[int8_zero("total_tokens"), int8_zero("request_count")],
    )
    .await;
    let conflict = exec_params_err(
        client,
        r#"INSERT INTO "LiteLLM_DailyToolSpend" (date, tool_name, updated_at) VALUES ($1, $2, now())"#,
        &[&"2026-01-01", &"search"],
    )
    .await;
    assert_eq!(sql_state(&conflict), SqlState::UNIQUE_VIOLATION.code());
}

async fn check_config_revision_zero_last_run_null(client: &Client) {
    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_Config" (param_name) VALUES ($1)"#,
        &[&"reload-param"],
    )
    .await;
    assert_column(
        client,
        "LiteLLM_Config",
        ("reload_revision", "bigint", false, Some("0"), "int8", None),
    )
    .await;
    let last_run_at = scalar_opt_text(
        client,
        r#"SELECT last_run_at::text FROM "LiteLLM_Config" WHERE param_name = $1"#,
        &[&"reload-param"],
    )
    .await;
    assert_eq!(last_run_at, None);
}

async fn check_managed_object_key_null_tags_array(client: &Client) {
    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_ManagedObjectTable" (id, unified_object_id, model_object_id, file_object, file_purpose, updated_at) VALUES ($1, $2, $3, '{}', 'batch', now())"#,
        &[&"object-tags", &"unified-tags", &"model-object-tags"],
    )
    .await;
    assert_columns(
        client,
        "LiteLLM_ManagedObjectTable",
        &[
            ("api_key", "text", true, None, "text", None),
            (
                "request_tags",
                "jsonb",
                true,
                Some("'[]'::jsonb"),
                "jsonb",
                None,
            ),
        ],
    )
    .await;
    let api_key = scalar_opt_text(
        client,
        r#"SELECT api_key FROM "LiteLLM_ManagedObjectTable" WHERE id = $1"#,
        &[&"object-tags"],
    )
    .await;
    let request_tags = scalar_text(
        client,
        r#"SELECT request_tags::text FROM "LiteLLM_ManagedObjectTable" WHERE id = $1"#,
        &[&"object-tags"],
    )
    .await;
    assert_eq!(api_key, None);
    assert_eq!(request_tags, "[]");
}

async fn check_gateway_zero_counters_and_date_index(client: &Client) {
    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_DailyGatewayRequests" (date, category, route, updated_at) VALUES ($1, $2, $3, now())"#,
        &[&"2026-01-01", &"chat", &"/v1/chat/completions"],
    )
    .await;
    assert_columns(
        client,
        "LiteLLM_DailyGatewayRequests",
        &[
            int8_zero("successful_requests"),
            int8_zero("failed_requests"),
        ],
    )
    .await;
    assert_index(
        client,
        "LiteLLM_DailyGatewayRequests_date_idx",
        &["date"],
        false,
    )
    .await;
}

async fn check_budget_window_key_excludes_start(client: &Client) {
    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_BudgetWindowSpend" (entity_type, entity_id, window_duration, window_start, spend, updated_at) VALUES ($1, $2, $3, '2026-01-01 00:00:00', $4, now())"#,
        &[&"team", &"team-window", &"1h", &1.0_f64],
    )
    .await;
    let conflict = exec_params_err(
        client,
        r#"INSERT INTO "LiteLLM_BudgetWindowSpend" (entity_type, entity_id, window_duration, window_start, spend, updated_at) VALUES ($1, $2, $3, '2026-01-01 01:00:00', $4, now())"#,
        &[&"team", &"team-window", &"1h", &2.0_f64],
    )
    .await;
    assert_eq!(sql_state(&conflict), SqlState::UNIQUE_VIOLATION.code());
}

const AUTOROUTER_SESSION: &str = "LiteLLM_AutoRouterSession";

async fn check_autorouter_fourteen_integer_counters_tokens_spend_models_tiers(client: &Client) {
    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_AutoRouterSession" (api_key, session_id, router_name, router_type, first_turn_at, last_turn_at, last_model) VALUES ($1, $2, $3, $4, now(), now(), $5)"#,
        &[&"key-a", &"session-a", &"router-a", &"adaptive", &"model-a"],
    )
    .await;
    for name in [
        "turns",
        "unordered_turns",
        "covered_turns",
        "cache_hits",
        "same_model_turns",
        "same_model_hits",
        "first_visit_turns",
        "first_visit_hits",
        "return_turns",
        "return_hits",
        "return_expired_misses",
        "return_within_ttl_misses",
        "ttl_5m_turns",
        "ttl_1h_turns",
    ] {
        assert_column(client, AUTOROUTER_SESSION, int4_zero(name)).await;
    }
    assert_column(client, AUTOROUTER_SESSION, int8_zero("total_tokens")).await;
    for name in ["spend", "saved_spend"] {
        assert_column(client, AUTOROUTER_SESSION, float8_default(name, "0")).await;
    }
    for name in ["models", "tier_turns"] {
        assert_column(client, AUTOROUTER_SESSION, empty_jsonb(name)).await;
    }
}

async fn run_usage_rollups_check(client: &Client, cohort: UsageRollupsCohort) {
    match cohort {
        UsageRollupsCohort::TeamPtuZero => check_team_ptu_zero(client).await,
        UsageRollupsCohort::SixCompressionTokenDefaults => {
            check_six_compression_token_defaults(client).await;
        }
        UsageRollupsCohort::SixThreeSavingsDefaultsPreserveExistingSpend => {
            check_six_three_savings_defaults_preserve_existing_spend(client).await;
        }
        UsageRollupsCohort::SpendToolStartIndex => check_spend_tool_start_index(client).await,
        UsageRollupsCohort::DailyToolZeroCountersAndCompositeKey => {
            check_daily_tool_zero_counters_and_composite_key(client).await;
        }
        UsageRollupsCohort::ConfigRevisionZeroLastRunNull => {
            check_config_revision_zero_last_run_null(client).await;
        }
        UsageRollupsCohort::ManagedObjectKeyNullTagsArray => {
            check_managed_object_key_null_tags_array(client).await;
        }
        UsageRollupsCohort::GatewayZeroCountersAndDateIndex => {
            check_gateway_zero_counters_and_date_index(client).await;
        }
        UsageRollupsCohort::BudgetWindowKeyExcludesStart => {
            check_budget_window_key_excludes_start(client).await;
        }
        UsageRollupsCohort::AutorouterFourteenIntegerCountersTokensSpendModelsTiers => {
            check_autorouter_fourteen_integer_counters_tokens_spend_models_tiers(client).await;
        }
    }
}

#[rstest]
#[case::team_ptu_zero(UsageRollupsCohort::TeamPtuZero)]
#[case::six_compression_token_defaults(UsageRollupsCohort::SixCompressionTokenDefaults)]
#[case::six_three_savings_defaults_preserve_existing_spend(
    UsageRollupsCohort::SixThreeSavingsDefaultsPreserveExistingSpend
)]
#[case::spend_tool_start_index(UsageRollupsCohort::SpendToolStartIndex)]
#[case::daily_tool_zero_counters_and_composite_key(
    UsageRollupsCohort::DailyToolZeroCountersAndCompositeKey
)]
#[case::config_revision_zero_last_run_null(UsageRollupsCohort::ConfigRevisionZeroLastRunNull)]
#[case::managed_object_key_null_tags_array(UsageRollupsCohort::ManagedObjectKeyNullTagsArray)]
#[case::gateway_zero_counters_and_date_index(UsageRollupsCohort::GatewayZeroCountersAndDateIndex)]
#[case::budget_window_key_excludes_start(UsageRollupsCohort::BudgetWindowKeyExcludesStart)]
#[case::autorouter_fourteen_integer_counters_tokens_spend_models_tiers(
    UsageRollupsCohort::AutorouterFourteenIntegerCountersTokensSpendModelsTiers
)]
#[tokio::test]
async fn postgres_usage_rollups(#[case] cohort: UsageRollupsCohort) {
    Box::pin(harness(move |[client]| async move {
        apply_migrations_through(&client, usage_rollups_checkpoint_for(cohort)).await;
        run_usage_rollups_check(&client, cohort).await;
    }))
    .await;
}
