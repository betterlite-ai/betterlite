#[derive(Clone, Copy)]
enum RoutingStateCohort {
    AdaptiveRequiredAlphaBeta,
    AdaptiveSevenZeroCounters,
    AdaptiveNegativeLastTurnFalseCreditAndJsonDefaults,
    AdaptiveRepeat42p07,
    MemoryGloballyUniqueKeyAndNullableOwners,
    MemberTotalSpendZero,
    WorkflowPendingAndRequiredUpdatedAt,
    WorkflowSessionUniqueness,
    ChildSequenceUniquenessAndRunDeleteRestrict,
    ChildRunUpdateCascade,
    ManagedTeamOwnerStaysNullAndDescendingIndexes,
    ProxyModelFalseBlocked,
}

fn routing_state_checkpoint_for(cohort: RoutingStateCohort) -> &'static str {
    match cohort {
        RoutingStateCohort::AdaptiveRequiredAlphaBeta
        | RoutingStateCohort::AdaptiveSevenZeroCounters
        | RoutingStateCohort::AdaptiveNegativeLastTurnFalseCreditAndJsonDefaults
        | RoutingStateCohort::AdaptiveRepeat42p07 => ADD_ADAPTIVE_ROUTER_TABLES.directory,
        RoutingStateCohort::MemoryGloballyUniqueKeyAndNullableOwners => ADD_MEMORY_TABLE.directory,
        RoutingStateCohort::MemberTotalSpendZero => ADD_TEAM_TOTAL_SPEND.directory,
        RoutingStateCohort::WorkflowPendingAndRequiredUpdatedAt
        | RoutingStateCohort::WorkflowSessionUniqueness
        | RoutingStateCohort::ChildSequenceUniquenessAndRunDeleteRestrict
        | RoutingStateCohort::ChildRunUpdateCascade
        | RoutingStateCohort::ManagedTeamOwnerStaysNullAndDescendingIndexes => {
            WORKFLOW_RUNS_TABLES.directory
        }
        RoutingStateCohort::ProxyModelFalseBlocked => ADD_PROXY_MODEL_BLOCKED.directory,
    }
}

async fn check_adaptive_seven_zero_counters(client: &Client) {
    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_AdaptiveRouterSession" (session_id, router_name, model_name, classified_type) VALUES ($1, $2, $3, $4)"#,
        &[&"session-a", &"router-a", &"model-a", &"normal"],
    )
    .await;
    for name in [
        "misalignment_count",
        "stagnation_count",
        "disengagement_count",
        "satisfaction_count",
        "failure_count",
        "loop_count",
        "exhaustion_count",
    ] {
        assert_column(client, "LiteLLM_AdaptiveRouterSession", int4_zero(name)).await;
    }
}

async fn check_adaptive_negative_last_turn_false_credit_and_json_defaults(client: &Client) {
    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_AdaptiveRouterSession" (session_id, router_name, model_name, classified_type) VALUES ($1, $2, $3, $4)"#,
        &[&"session-b", &"router-b", &"model-b", &"normal"],
    )
    .await;
    let row = query_one_row(
        client,
        r#"SELECT last_processed_turn, clean_credit_awarded, tool_call_history::text, pending_tool_calls::text FROM "LiteLLM_AdaptiveRouterSession" WHERE session_id = $1"#,
        &[&"session-b"],
    )
    .await;
    let last_processed_turn: i32 = row.get(0);
    let clean_credit_awarded: bool = row.get(1);
    let tool_call_history: String = row.get(2);
    let pending_tool_calls: String = row.get(3);
    assert_eq!(last_processed_turn, -1_i32);
    assert!(!clean_credit_awarded);
    assert_eq!(tool_call_history, "[]");
    assert_eq!(pending_tool_calls, "{}");
}

async fn check_memory_globally_unique_key_and_nullable_owners(client: &Client) {
    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_MemoryTable" (memory_id, key, value) VALUES ($1, $2, $3)"#,
        &[&"memory-a", &"shared-key", &"v1"],
    )
    .await;
    assert_columns(
        client,
        "LiteLLM_MemoryTable",
        &[
            ("user_id", "text", true, None, "text", None),
            ("team_id", "text", true, None, "text", None),
        ],
    )
    .await;
    let conflict = exec_params_err(
        client,
        r#"INSERT INTO "LiteLLM_MemoryTable" (memory_id, key, value, team_id) VALUES ($1, $2, $3, $4)"#,
        &[&"memory-b", &"shared-key", &"v2", &"team-b"],
    )
    .await;
    assert_eq!(sql_state(&conflict), SqlState::UNIQUE_VIOLATION.code());
}

async fn check_workflow_pending_and_required_updated_at(client: &Client) {
    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_WorkflowRun" (run_id, session_id, workflow_type, updated_at) VALUES ($1, $2, $3, now())"#,
        &[&"run-pending", &"session-pending", &"type-a"],
    )
    .await;
    let status = scalar_text(
        client,
        r#"SELECT status FROM "LiteLLM_WorkflowRun" WHERE run_id = $1"#,
        &[&"run-pending"],
    )
    .await;
    assert_eq!(status, "pending");
    let missing_updated_at = exec_params_err(
        client,
        r#"INSERT INTO "LiteLLM_WorkflowRun" (run_id, session_id, workflow_type) VALUES ($1, $2, $3)"#,
        &[&"run-no-updated-at", &"session-no-updated-at", &"type-a"],
    )
    .await;
    assert_eq!(
        sql_state(&missing_updated_at),
        SqlState::NOT_NULL_VIOLATION.code()
    );
}

async fn insert_workflow_run(client: &Client, run_id: &str, session_id: &str, workflow_type: &str) {
    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_WorkflowRun" (run_id, session_id, workflow_type, updated_at) VALUES ($1, $2, $3, now())"#,
        &[&run_id, &session_id, &workflow_type],
    )
    .await;
}

async fn check_child_sequence_uniqueness_and_run_delete_restrict(client: &Client) {
    insert_workflow_run(client, "run-child", "session-child", "type-a").await;
    assert_repeat_insert_unique_violation(
        client,
        r#"INSERT INTO "LiteLLM_WorkflowEvent" (event_id, run_id, event_type, step_name, sequence_number) VALUES ($1, $2, $3, $4, $5)"#,
        &[&"event-a", &"run-child", &"started", &"step-a", &1_i32],
        &[&"event-b", &"run-child", &"started", &"step-a", &1_i32],
    )
    .await;
    let restricted = exec_params_err(
        client,
        r#"DELETE FROM "LiteLLM_WorkflowRun" WHERE run_id = $1"#,
        &[&"run-child"],
    )
    .await;
    assert_eq!(
        sql_state(&restricted),
        SqlState::FOREIGN_KEY_VIOLATION.code()
    );
}

async fn check_child_run_update_cascade(client: &Client) {
    insert_workflow_run(client, "run-old", "session-old", "type-a").await;
    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_WorkflowEvent" (event_id, run_id, event_type, step_name, sequence_number) VALUES ($1, $2, $3, $4, $5)"#,
        &[&"event-cascade", &"run-old", &"started", &"step-a", &1_i32],
    )
    .await;
    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_WorkflowMessage" (message_id, run_id, role, content, sequence_number) VALUES ($1, $2, $3, $4, $5)"#,
        &[&"message-cascade", &"run-old", &"user", &"hello", &1_i32],
    )
    .await;
    exec_params(
        client,
        r#"UPDATE "LiteLLM_WorkflowRun" SET run_id = $1 WHERE run_id = $2"#,
        &[&"run-new", &"run-old"],
    )
    .await;
    let event_run_id = scalar_text(
        client,
        r#"SELECT run_id FROM "LiteLLM_WorkflowEvent" WHERE event_id = $1"#,
        &[&"event-cascade"],
    )
    .await;
    let message_run_id = scalar_text(
        client,
        r#"SELECT run_id FROM "LiteLLM_WorkflowMessage" WHERE message_id = $1"#,
        &[&"message-cascade"],
    )
    .await;
    assert_eq!(event_run_id, "run-new");
    assert_eq!(message_run_id, "run-new");
    assert_rule(client, "LiteLLM_WorkflowEvent_run_id_fkey", 'c', 'r').await;
    assert_rule(client, "LiteLLM_WorkflowMessage_run_id_fkey", 'c', 'r').await;
}

async fn check_managed_team_owner_stays_null_and_descending_indexes(client: &Client) {
    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_ManagedObjectTable" (id, unified_object_id, model_object_id, file_object, file_purpose, updated_at) VALUES ($1, $2, $3, '{}', 'batch', now())"#,
        &[&"object-a", &"unified-a", &"model-object-a"],
    )
    .await;
    batch_execute(
        client,
        migration_sql_of(MANAGED_RESOURCE_TEAM_OWNER.directory),
    )
    .await;
    let team_id = scalar_opt_text(
        client,
        r#"SELECT team_id FROM "LiteLLM_ManagedObjectTable" WHERE id = $1"#,
        &[&"object-a"],
    )
    .await;
    assert_eq!(team_id, None);
    for index_name in [
        "LiteLLM_ManagedFileTable_team_id_created_at_idx",
        "LiteLLM_ManagedObjectTable_team_id_created_at_idx",
        "LiteLLM_ManagedVectorStoreTable_team_id_created_at_idx",
    ] {
        let indexdef = scalar_text(
            client,
            "SELECT indexdef FROM pg_indexes WHERE indexname = $1",
            &[&index_name],
        )
        .await;
        assert!(
            indexdef.contains("DESC"),
            "{index_name} should sort descending"
        );
    }
}

async fn check_proxy_model_false_blocked(client: &Client) {
    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_ProxyModelTable" (model_id, model_name, litellm_params, created_by, updated_by) VALUES ($1, $2, '{}', $3, $3)"#,
        &[&"model-a", &"gpt", &"operator"],
    )
    .await;
    assert_column(client, "LiteLLM_ProxyModelTable", bool_false("blocked")).await;
    let blocked = scalar_bool(
        client,
        r#"SELECT blocked FROM "LiteLLM_ProxyModelTable" WHERE model_id = $1"#,
        &[&"model-a"],
    )
    .await;
    assert!(!blocked);
}

async fn check_adaptive_required_alpha_beta(client: &Client) {
    assert_exec_params_err_state(
        client,
        r#"INSERT INTO "LiteLLM_AdaptiveRouterState" (router_name, request_type, model_name) VALUES ($1, $2, $3)"#,
        &[&"router-a", &"chat", &"model-a"],
        SqlState::NOT_NULL_VIOLATION,
    )
    .await;
}

async fn check_workflow_session_uniqueness(client: &Client) {
    assert_repeat_insert_unique_violation(
        client,
        r#"INSERT INTO "LiteLLM_WorkflowRun" (run_id, session_id, workflow_type, updated_at) VALUES ($1, $2, $3, now())"#,
        &[&"run-first", &"session-shared", &"type-a"],
        &[&"run-second", &"session-shared", &"type-a"],
    )
    .await;
}

async fn check_member_total_spend_zero(client: &Client) {
    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_TeamMembership" (user_id, team_id) VALUES ($1, $2)"#,
        &[&"user-a", &"team-a"],
    )
    .await;
    assert_column(
        client,
        "LiteLLM_TeamMembership",
        (
            "total_spend",
            "double precision",
            false,
            Some("0.0"),
            "float8",
            None,
        ),
    )
    .await;
}

async fn run_routing_state_check(client: &Client, cohort: RoutingStateCohort) {
    match cohort {
        RoutingStateCohort::AdaptiveRequiredAlphaBeta => {
            check_adaptive_required_alpha_beta(client).await;
        }
        RoutingStateCohort::AdaptiveSevenZeroCounters => {
            check_adaptive_seven_zero_counters(client).await;
        }
        RoutingStateCohort::AdaptiveNegativeLastTurnFalseCreditAndJsonDefaults => {
            check_adaptive_negative_last_turn_false_credit_and_json_defaults(client).await;
        }
        RoutingStateCohort::AdaptiveRepeat42p07 => {
            let sql = migration_sql_of(ADD_ADAPTIVE_ROUTER_TABLES.directory);
            assert_batch_execute_err_state(client, sql, SqlState::DUPLICATE_TABLE).await;
        }
        RoutingStateCohort::MemoryGloballyUniqueKeyAndNullableOwners => {
            check_memory_globally_unique_key_and_nullable_owners(client).await;
        }
        RoutingStateCohort::MemberTotalSpendZero => {
            check_member_total_spend_zero(client).await;
        }
        RoutingStateCohort::WorkflowPendingAndRequiredUpdatedAt => {
            check_workflow_pending_and_required_updated_at(client).await;
        }
        RoutingStateCohort::WorkflowSessionUniqueness => {
            check_workflow_session_uniqueness(client).await;
        }
        RoutingStateCohort::ChildSequenceUniquenessAndRunDeleteRestrict => {
            check_child_sequence_uniqueness_and_run_delete_restrict(client).await;
        }
        RoutingStateCohort::ChildRunUpdateCascade => check_child_run_update_cascade(client).await,
        RoutingStateCohort::ManagedTeamOwnerStaysNullAndDescendingIndexes => {
            check_managed_team_owner_stays_null_and_descending_indexes(client).await;
        }
        RoutingStateCohort::ProxyModelFalseBlocked => check_proxy_model_false_blocked(client).await,
    }
}

#[rstest]
#[case::adaptive_required_alpha_beta(RoutingStateCohort::AdaptiveRequiredAlphaBeta)]
#[case::adaptive_seven_zero_counters(RoutingStateCohort::AdaptiveSevenZeroCounters)]
#[case::adaptive_negative_last_turn_false_credit_and_json_defaults(
    RoutingStateCohort::AdaptiveNegativeLastTurnFalseCreditAndJsonDefaults
)]
#[case::adaptive_repeat_42p07(RoutingStateCohort::AdaptiveRepeat42p07)]
#[case::memory_globally_unique_key_and_nullable_owners(
    RoutingStateCohort::MemoryGloballyUniqueKeyAndNullableOwners
)]
#[case::member_total_spend_zero(RoutingStateCohort::MemberTotalSpendZero)]
#[case::workflow_pending_and_required_updated_at(
    RoutingStateCohort::WorkflowPendingAndRequiredUpdatedAt
)]
#[case::workflow_session_uniqueness(RoutingStateCohort::WorkflowSessionUniqueness)]
#[case::child_sequence_uniqueness_and_run_delete_restrict(
    RoutingStateCohort::ChildSequenceUniquenessAndRunDeleteRestrict
)]
#[case::child_run_update_cascade(RoutingStateCohort::ChildRunUpdateCascade)]
#[case::managed_team_owner_stays_null_and_descending_indexes(
    RoutingStateCohort::ManagedTeamOwnerStaysNullAndDescendingIndexes
)]
#[case::proxy_model_false_blocked(RoutingStateCohort::ProxyModelFalseBlocked)]
#[tokio::test]
async fn postgres_routing_state(#[case] cohort: RoutingStateCohort) {
    Box::pin(harness(move |[client]| async move {
        apply_migrations_through(&client, routing_state_checkpoint_for(cohort)).await;
        run_routing_state_check(&client, cohort).await;
    }))
    .await;
}
