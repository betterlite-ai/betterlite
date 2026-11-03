const CACHE_TOKENS_FOR_USER: &str =
    "20250415151647_add_cache_read_write_tokens_daily_spend_transactions";
const ADD_TAG_TABLE: &str = "20250416115320_add_tag_table_to_db";
const DROP_TAG_UNIQUENESS: &str = "20250416151339_drop_tag_uniqueness_requirement";
const ADD_STATUS_TO_SPENDLOGS: &str = "20250508072103_add_status_to_spendlogs";
const USE_BIG_INT: &str = "20250509141545_use_big_int_for_daily_spend_tables";
const ALLOW_NULL_ENTITY_ID: &str = "20250526154401_allow_null_entity_id";

const SEVEN_COUNTERS: [&str; 7] = [
    "prompt_tokens",
    "completion_tokens",
    "api_requests",
    "successful_requests",
    "failed_requests",
    "cache_creation_input_tokens",
    "cache_read_input_tokens",
];

async fn assert_counters_are_integer_default_zero(client: &Client, table: &str) {
    for name in SEVEN_COUNTERS {
        let info = column(client, table, name).await;
        assert_eq!(info.data_type, "integer", "{table}.{name}");
        assert!(!info.is_nullable, "{table}.{name}");
        assert!(info.default.unwrap().contains('0'), "{table}.{name}");
    }
}

async fn insert_user_row(client: &Client, id: &str, custom_llm_provider: Option<&str>) {
    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_DailyUserSpend" (id, user_id, date, api_key, model, custom_llm_provider, updated_at)
           VALUES ($1, 'u1', 'd1', 'k1', 'm1', $2, NOW())"#,
        &[&id, &custom_llm_provider],
    )
    .await;
}

#[tokio::test]
async fn user_counter_additions() {
    harness(|[client]| async move {
        client
            .batch_execute(&migration_sql_through(CACHE_TOKENS_FOR_USER))
            .await
            .unwrap();
        assert_counters_are_integer_default_zero(&client, "LiteLLM_DailyUserSpend").await;
    })
    .await;
}

#[tokio::test]
async fn team_counter_additions() {
    harness(|[client]| async move {
        client
            .batch_execute(&migration_sql_through(ADD_TAG_TABLE))
            .await
            .unwrap();
        assert_counters_are_integer_default_zero(&client, "LiteLLM_DailyTeamSpend").await;
    })
    .await;
}

#[tokio::test]
async fn tag_counter_defaults() {
    harness(|[client]| async move {
        client
            .batch_execute(&migration_sql_through(ADD_TAG_TABLE))
            .await
            .unwrap();
        assert_counters_are_integer_default_zero(&client, "LiteLLM_DailyTagSpend").await;
    })
    .await;
}

#[rstest]
#[case::user_counters("LiteLLM_DailyUserSpend", "user_id", "u1")]
#[case::team_counters("LiteLLM_DailyTeamSpend", "team_id", "t1")]
#[case::tag_counters("LiteLLM_DailyTagSpend", "tag", "tag1")]
#[tokio::test]
async fn seven_counters_widen_preserving_values(
    #[case] table: &str,
    #[case] entity_column: &str,
    #[case] entity_id: &str,
) {
    harness(|[client]| async move {
        client.batch_execute(&migration_sql_through(ADD_STATUS_TO_SPENDLOGS)).await.unwrap();
        let insert = format!(
            "INSERT INTO \"{table}\" (id, \"{entity_column}\", date, api_key, model, updated_at) VALUES ('row-widen', $1, 'd1', 'k1', 'm1', NOW())"
        );
        exec_params(&client, &insert, &[&entity_id]).await;
        let update = format!(
            "UPDATE \"{table}\" SET prompt_tokens = 11, completion_tokens = 22, api_requests = 33, successful_requests = 44, failed_requests = 55, cache_creation_input_tokens = 66, cache_read_input_tokens = 77 WHERE id = 'row-widen'"
        );
        client.batch_execute(&update).await.unwrap();
        client.batch_execute(migration_sql_of(USE_BIG_INT)).await.unwrap();
        let types = client.query(
            "SELECT data_type FROM information_schema.columns WHERE table_schema = 'public' AND table_name = $1 AND column_name = ANY($2) ORDER BY array_position($2, column_name)",
            &[&table, &&SEVEN_COUNTERS[..]],
        ).await.unwrap();
        assert_eq!(types.iter().map(|row| row.get::<_, String>(0)).collect::<Vec<_>>(), vec!["bigint"; 7]);
        let select = format!("SELECT {} FROM \"{table}\" WHERE id = 'row-widen'", SEVEN_COUNTERS.join(", "));
        let values = client.query_one(&select, &[]).await.unwrap();
        assert_counter_values(&values, table);
    }).await;
}

#[tokio::test]
async fn values_above_i32_after_widen() {
    harness(|[client]| async move {
        client
            .batch_execute(&migration_sql_through(USE_BIG_INT))
            .await
            .unwrap();
        insert_user_row(&client, "row-big", Some("openai")).await;
        let big: i64 = 5_000_000_000;
        exec_params(
            &client,
            r#"UPDATE "LiteLLM_DailyUserSpend" SET api_requests = $1 WHERE id = $2"#,
            &[&big, &"row-big"],
        )
        .await;
        let value = scalar_i64(
            &client,
            r#"SELECT api_requests FROM "LiteLLM_DailyUserSpend" WHERE id = $1"#,
            &[&"row-big"],
        )
        .await;
        assert_eq!(value, big);
    })
    .await;
}

#[tokio::test]
async fn entity_date_key_model_provider_uniqueness() {
    harness(|[client]| async move {
        client
            .batch_execute(&migration_sql_through(ADD_DAILY_USER_TABLE))
            .await
            .unwrap();
        insert_user_row(&client, "row-a", Some("openai")).await;
        let error = exec_params_err(
            &client,
            r#"INSERT INTO "LiteLLM_DailyUserSpend" (id, user_id, date, api_key, model, custom_llm_provider, updated_at)
               VALUES ('row-b', 'u1', 'd1', 'k1', 'm1', 'openai', NOW())"#,
            &[],
        )
        .await;
        assert_eq!(error.code().unwrap().code(), "23505");
        insert_user_row(&client, "row-c", Some("anthropic")).await;
    })
    .await;
}

#[tokio::test]
async fn tag_global_unique_then_dropped() {
    harness(|[client]| async move {
        client
            .batch_execute(&migration_sql_through(ADD_TAG_TABLE))
            .await
            .unwrap();
        assert!(index_exists(&client, "LiteLLM_DailyTagSpend_tag_key").await);
        exec_params(
            &client,
            r#"INSERT INTO "LiteLLM_DailyTagSpend" (id, tag, date, api_key, model, updated_at)
               VALUES ('tag-a', 't1', 'd1', 'k1', 'm1', NOW())"#,
            &[],
        )
        .await;
        let error = exec_params_err(
            &client,
            r#"INSERT INTO "LiteLLM_DailyTagSpend" (id, tag, date, api_key, model, updated_at)
               VALUES ('tag-b', 't1', 'd2', 'k2', 'm2', NOW())"#,
            &[],
        )
        .await;
        assert_eq!(error.code().unwrap().code(), "23505");
        client
            .batch_execute(migration_sql_of(DROP_TAG_UNIQUENESS))
            .await
            .unwrap();
        assert!(!index_exists(&client, "LiteLLM_DailyTagSpend_tag_key").await);
        exec_params(
            &client,
            r#"INSERT INTO "LiteLLM_DailyTagSpend" (id, tag, date, api_key, model, updated_at)
               VALUES ('tag-c', 't1', 'd2', 'k2', 'm2', NOW())"#,
            &[],
        )
        .await;
    })
    .await;
}

#[rstest]
#[case::null_user_id("LiteLLM_DailyUserSpend", "user_id")]
#[case::null_team_id("LiteLLM_DailyTeamSpend", "team_id")]
#[case::null_tag("LiteLLM_DailyTagSpend", "tag")]
#[tokio::test]
async fn nullable_entities_and_distinct_null_uniqueness(
    #[case] table: &str,
    #[case] entity_column: &str,
) {
    harness(|[client]| async move {
        client
            .batch_execute(&migration_sql_through(ALLOW_NULL_ENTITY_ID))
            .await
            .unwrap();
        let info = column(&client, table, entity_column).await;
        assert!(info.is_nullable, "{table}.{entity_column}");
        insert_null_entity_row(&client, table, entity_column, "null-a").await;
        insert_null_entity_row(&client, table, entity_column, "null-b").await;
    })
    .await;
}

async fn assert_required_updated_at(client: &Client, tables: &[&str]) {
    for table in tables {
        let info = column(client, table, "updated_at").await;
        assert!(!info.is_nullable, "{table}.updated_at");
        assert_eq!(info.default, None, "{table}.updated_at");
    }
}

#[tokio::test]
async fn required_updated_at_without_default() {
    harness(|[client]| async move {
        client
            .batch_execute(&migration_sql_through(ADD_TAG_TABLE))
            .await
            .unwrap();
        assert_required_updated_at(
            &client,
            &[
                "LiteLLM_DailyUserSpend",
                "LiteLLM_DailyTeamSpend",
                "LiteLLM_DailyTagSpend",
            ],
        )
        .await;
    })
    .await;
}
