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
    assert_column(client, "LiteLLM_LensRun", required_text("lens_id")).await;
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
            timestamp3("created_at", Some("CURRENT_TIMESTAMP")),
            timestamp3("updated_at", None),
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
    assert_column(client, "LiteLLM_EngineRun", timestamp3("created_at", None)).await;
}
fn usage_and_lens_checkpoint(cohort: UsageAndLensCohort) -> &'static str {
    use UsageAndLensCohort::{
        DailyModelFiveCountersZeroRequiredDimensionsAndUpdatedAt,
        EngineRunRequiredCreatedAtWithoutFk, EngineZeroVersionRequiredJson,
        LateIndexCollisionRollsBackDo, LensNamesAndFiveIndexRenames,
        PinnedToolsNullableEmptyObject, RenameAlreadyFinalNoop, RenameExistingDestinationError,
        RenamePreservesOidsRowsAndHashBytes, WorkerUniqueHashUnchanged,
    };
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
    use UsageAndLensCohort::{
        DailyModelFiveCountersZeroRequiredDimensionsAndUpdatedAt,
        EngineRunRequiredCreatedAtWithoutFk, EngineZeroVersionRequiredJson,
        LateIndexCollisionRollsBackDo, LensNamesAndFiveIndexRenames,
        PinnedToolsNullableEmptyObject, RenameAlreadyFinalNoop, RenameExistingDestinationError,
        RenamePreservesOidsRowsAndHashBytes, WorkerUniqueHashUnchanged,
    };
    apply_migrations_through(client, usage_and_lens_checkpoint(cohort)).await;
    let check: Pin<Box<dyn Future<Output = ()> + Send + '_>> = match cohort {
        PinnedToolsNullableEmptyObject => {
            Box::pin(check_pinned_tools_nullable_empty_object(client))
        }
        DailyModelFiveCountersZeroRequiredDimensionsAndUpdatedAt => Box::pin(
            check_daily_model_five_counters_zero_required_dimensions_and_updated_at(client),
        ),
        EngineZeroVersionRequiredJson => Box::pin(check_engine_zero_version_required_json(client)),
        WorkerUniqueHashUnchanged => Box::pin(check_worker_unique_hash_unchanged(client)),
        EngineRunRequiredCreatedAtWithoutFk => {
            Box::pin(check_engine_run_required_created_at_without_fk(client))
        }
        LensNamesAndFiveIndexRenames => Box::pin(assert_lens_renamed(client)),
        RenamePreservesOidsRowsAndHashBytes => {
            Box::pin(check_rename_preserves_oids_rows_and_hash_bytes(client))
        }
        RenameAlreadyFinalNoop => Box::pin(check_rename_already_final_noop(client)),
        RenameExistingDestinationError => Box::pin(check_rename_existing_destination_error(client)),
        LateIndexCollisionRollsBackDo => Box::pin(check_late_index_collision_rolls_back_do(client)),
    };
    check.await;
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
