#[derive(Clone, Copy)]
enum UsageAndLensCohort {
    PinnedToolsNullableEmptyObject,
    DailyModelFiveCountersZeroRequiredDimensionsAndUpdatedAt,
    EngineZeroVersionRequiredJson,
    WorkerUniqueHashUnchanged,
    EngineRunRequiredCreatedAtWithoutFk,
    LensNamesAndFiveIndexRenames,
    RenamePreservesOidsRowsAndHashBytes,
    RenameAlreadyFinalNoop,
    RenameExistingDestinationError,
    LateIndexCollisionRollsBackDo,
}

async fn assert_lens_renamed(client: &Client) {
    for name in ["LiteLLM_Lens", "LiteLLM_LensRun", "LiteLLM_LensWorker"] {
        assert!(relation_exists(client, name).await, "{name} missing");
    }
    for name in [
        "LiteLLM_Engine",
        "LiteLLM_EngineRun",
        "LiteLLM_EngineWorker",
    ] {
        assert!(!relation_exists(client, name).await, "{name} still present");
    }
    for name in [
        "LiteLLM_Lens_pkey",
        "LiteLLM_LensRun_pkey",
        "LiteLLM_LensWorker_pkey",
        "LiteLLM_LensWorker_token_hash_key",
        "LiteLLM_LensRun_lens_id_created_at_idx",
    ] {
        assert!(relation_exists(client, name).await, "{name} missing");
    }
    assert_column(client, "LiteLLM_LensRun", nullable_text("lens_id")).await;
}

async fn check_pinned_tools_nullable_empty_object(client: &Client) {
    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_MCPServerTable" (server_id, url) VALUES ($1, $2)"#,
        &[&"server-a", &"https://example.test/mcp"],
    )
    .await;
    assert_column(
        client,
        "LiteLLM_MCPServerTable",
        nullable_empty_jsonb("pinned_tools"),
    )
    .await;
}

async fn check_daily_model_five_counters_zero_required_dimensions_and_updated_at(client: &Client) {
    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_DailyModelUsage" (date, model_group, model, custom_llm_provider, task_type, updated_at) VALUES ('date-a', 'group-a', 'model-a', 'openai', 'chat', now())"#,
        &[],
    )
    .await;
    assert_columns(
        client,
        "LiteLLM_DailyModelUsage",
        &[
            float8_zero("spend"),
            int8_zero("prompt_tokens"),
            int8_zero("completion_tokens"),
            int8_zero("request_count"),
            int8_zero("successful_requests"),
            int8_zero("failed_requests"),
            timestamp3_now_default("created_at"),
            timestamp3_required("updated_at"),
        ],
    )
    .await;
}

async fn check_engine_zero_version_required_json(client: &Client) {
    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_Engine" (id, data) VALUES ($1, '{"k":"v"}')"#,
        &[&"engine-a"],
    )
    .await;
    assert_columns(
        client,
        "LiteLLM_Engine",
        &[
            int4_zero("version"),
            ("data", "jsonb", false, None, "jsonb", None),
        ],
    )
    .await;
}

async fn check_worker_unique_hash_unchanged(client: &Client) {
    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_EngineWorker" (id, token_hash, data) VALUES ($1, $2, '{}')"#,
        &[&"worker-a", &"hash-a"],
    )
    .await;
    assert_exec_params_err_state(
        client,
        r#"INSERT INTO "LiteLLM_EngineWorker" (id, token_hash, data) VALUES ($1, $2, '{}')"#,
        &[&"worker-b", &"hash-a"],
        SqlState::UNIQUE_VIOLATION,
    )
    .await;
    let token_hash = scalar_text(
        client,
        r#"SELECT token_hash FROM "LiteLLM_EngineWorker" WHERE id = $1"#,
        &[&"worker-a"],
    )
    .await;
    assert_eq!(token_hash, "hash-a");
}

async fn check_engine_run_required_created_at_without_fk(client: &Client) {
    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_EngineRun" (id, engine_id, created_at, data) VALUES ($1, $2, now(), '{}')"#,
        &[&"run-a", &"engine-never-existed"],
    )
    .await;
    assert_column(
        client,
        "LiteLLM_EngineRun",
        timestamp3_required("created_at"),
    )
    .await;
}

async fn insert_rename_fixture_rows(client: &Client) {
    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_Engine" (id, version, data) VALUES ($1, 2, '{"k":"v"}')"#,
        &[&"engine-b"],
    )
    .await;
    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_EngineWorker" (id, token_hash, data) VALUES ($1, $2, '{}')"#,
        &[&"worker-b", &"hash-b"],
    )
    .await;
    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_EngineRun" (id, engine_id, created_at, data) VALUES ($1, $2, now(), '{"r":1}')"#,
        &[&"run-b", &"engine-b"],
    )
    .await;
}

async fn capture_pre_rename_oids(client: &Client) -> (String, String, String, String) {
    (
        relation_oid(client, "LiteLLM_Engine").await,
        relation_oid(client, "LiteLLM_EngineRun").await,
        relation_oid(client, "LiteLLM_EngineWorker").await,
        relation_oid(client, "LiteLLM_EngineWorker_token_hash_key").await,
    )
}

async fn assert_rename_preserved_oids(client: &Client, before: (String, String, String, String)) {
    assert_eq!(before.0, relation_oid(client, "LiteLLM_Lens").await);
    assert_eq!(before.1, relation_oid(client, "LiteLLM_LensRun").await);
    assert_eq!(before.2, relation_oid(client, "LiteLLM_LensWorker").await);
    assert_eq!(
        before.3,
        relation_oid(client, "LiteLLM_LensWorker_token_hash_key").await
    );
}

async fn assert_rename_preserved_rows(client: &Client) {
    let version: i32 = query_one_row(
        client,
        r#"SELECT version FROM "LiteLLM_Lens" WHERE id = $1"#,
        &[&"engine-b"],
    )
    .await
    .get(0);
    assert_eq!(version, 2_i32);
    let row = query_one_row(
        client,
        r#"SELECT lens_id, data::text FROM "LiteLLM_LensRun" WHERE id = $1"#,
        &[&"run-b"],
    )
    .await;
    let lens_id: String = row.get(0);
    let data: String = row.get(1);
    assert_eq!(lens_id, "engine-b");
    assert_eq!(data, r#"{"r": 1}"#);
    let token_hash = scalar_text(
        client,
        r#"SELECT token_hash FROM "LiteLLM_LensWorker" WHERE id = $1"#,
        &[&"worker-b"],
    )
    .await;
    assert_eq!(token_hash, "hash-b");
}

async fn check_rename_preserves_oids_rows_and_hash_bytes(client: &Client) {
    insert_rename_fixture_rows(client).await;
    let before = capture_pre_rename_oids(client).await;
    batch_execute(client, migration_sql_of(RENAME_LENS.directory)).await;
    assert_rename_preserved_oids(client, before).await;
    assert_rename_preserved_rows(client).await;
}

async fn check_rename_already_final_noop(client: &Client) {
    batch_execute(client, migration_sql_of(RENAME_LENS.directory)).await;
    let before = catalog_snapshot(client).await;
    batch_execute(client, migration_sql_of(RENAME_LENS.directory)).await;
    assert_eq!(before, catalog_snapshot(client).await);
}

async fn check_rename_existing_destination_error(client: &Client) {
    batch_execute(client, r#"CREATE TABLE "LiteLLM_Lens" (id INT);"#).await;
    batch_execute_err(client, migration_sql_of(RENAME_LENS.directory)).await;
    assert!(relation_exists(client, "LiteLLM_Engine").await);
    assert!(!relation_exists(client, "LiteLLM_LensRun").await);
}

async fn check_late_index_collision_rolls_back_do(client: &Client) {
    batch_execute(
        client,
        r#"CREATE TABLE "scratch_collision" (token_hash TEXT);
           CREATE UNIQUE INDEX "LiteLLM_LensWorker_token_hash_key" ON "scratch_collision" (token_hash);"#,
    )
    .await;
    batch_execute_err(client, migration_sql_of(RENAME_LENS.directory)).await;
    assert!(relation_exists(client, "LiteLLM_Engine").await);
    assert!(relation_exists(client, "LiteLLM_EngineRun").await);
    assert!(relation_exists(client, "LiteLLM_EngineWorker").await);
    assert!(!relation_exists(client, "LiteLLM_Lens").await);
}

fn usage_and_lens_checkpoint(cohort: UsageAndLensCohort) -> &'static str {
    use UsageAndLensCohort::*;
    match cohort {
        PinnedToolsNullableEmptyObject => ADD_MCP_PINNED_TOOLS.directory,
        DailyModelFiveCountersZeroRequiredDimensionsAndUpdatedAt => ADD_DAILY_MODEL_USAGE.directory,
        EngineZeroVersionRequiredJson | WorkerUniqueHashUnchanged => AGENT_ENGINE.directory,
        LensNamesAndFiveIndexRenames => RENAME_LENS.directory,
        EngineRunRequiredCreatedAtWithoutFk
        | RenamePreservesOidsRowsAndHashBytes
        | RenameAlreadyFinalNoop
        | RenameExistingDestinationError
        | LateIndexCollisionRollsBackDo => LENS_RUN_HISTORY.directory,
    }
}

async fn run_usage_and_lens_check(client: &Client, cohort: UsageAndLensCohort) {
    use UsageAndLensCohort::*;
    apply_migrations_through(client, usage_and_lens_checkpoint(cohort)).await;
    match cohort {
        PinnedToolsNullableEmptyObject => check_pinned_tools_nullable_empty_object(client).await,
        DailyModelFiveCountersZeroRequiredDimensionsAndUpdatedAt => {
            check_daily_model_five_counters_zero_required_dimensions_and_updated_at(client).await;
        }
        EngineZeroVersionRequiredJson => check_engine_zero_version_required_json(client).await,
        WorkerUniqueHashUnchanged => check_worker_unique_hash_unchanged(client).await,
        EngineRunRequiredCreatedAtWithoutFk => {
            check_engine_run_required_created_at_without_fk(client).await;
        }
        LensNamesAndFiveIndexRenames => assert_lens_renamed(client).await,
        RenamePreservesOidsRowsAndHashBytes => {
            check_rename_preserves_oids_rows_and_hash_bytes(client).await;
        }
        RenameAlreadyFinalNoop => check_rename_already_final_noop(client).await,
        RenameExistingDestinationError => check_rename_existing_destination_error(client).await,
        LateIndexCollisionRollsBackDo => check_late_index_collision_rolls_back_do(client).await,
    }
}

#[rstest]
#[case::pinned_tools_nullable_empty_object(UsageAndLensCohort::PinnedToolsNullableEmptyObject)]
#[case::daily_model_five_counters_zero_required_dimensions_and_updated_at(
    UsageAndLensCohort::DailyModelFiveCountersZeroRequiredDimensionsAndUpdatedAt
)]
#[case::engine_zero_version_required_json(UsageAndLensCohort::EngineZeroVersionRequiredJson)]
#[case::worker_unique_hash_unchanged(UsageAndLensCohort::WorkerUniqueHashUnchanged)]
#[case::engine_run_required_created_at_without_fk(
    UsageAndLensCohort::EngineRunRequiredCreatedAtWithoutFk
)]
#[case::lens_names_and_five_index_renames(UsageAndLensCohort::LensNamesAndFiveIndexRenames)]
#[case::rename_preserves_oids_rows_and_hash_bytes(
    UsageAndLensCohort::RenamePreservesOidsRowsAndHashBytes
)]
#[case::rename_already_final_noop(UsageAndLensCohort::RenameAlreadyFinalNoop)]
#[case::rename_existing_destination_error(UsageAndLensCohort::RenameExistingDestinationError)]
#[case::late_index_collision_rolls_back_do(UsageAndLensCohort::LateIndexCollisionRollsBackDo)]
#[tokio::test]
async fn postgres_usage_and_lens(#[case] cohort: UsageAndLensCohort) {
    Box::pin(harness(move |[client]| async move {
        run_usage_and_lens_check(&client, cohort).await;
    }))
    .await;
}
