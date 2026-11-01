async fn insert_budget(client: &Client, budget_id: &str) {
    exec_params(
        client,
        "INSERT INTO \"LiteLLM_BudgetTable\" (budget_id, created_by, updated_by) \
         VALUES ($1, 'tester', 'tester')",
        &[&budget_id],
    )
    .await;
}

async fn insert_team(client: &Client, team_id: &str) {
    exec_params(
        client,
        "INSERT INTO \"LiteLLM_TeamTable\" (team_id) VALUES ($1)",
        &[&team_id],
    )
    .await;
}

async fn insert_object_permission(client: &Client, object_permission_id: &str) {
    exec_params(
        client,
        "INSERT INTO \"LiteLLM_ObjectPermissionTable\" (object_permission_id) VALUES ($1)",
        &[&object_permission_id],
    )
    .await;
}

async fn insert_project(
    client: &Client,
    project_id: &str,
    team_id: &str,
    budget_id: &str,
    object_permission_id: &str,
) {
    exec_params(
        client,
        "INSERT INTO \"LiteLLM_ProjectTable\" \
         (project_id, team_id, budget_id, object_permission_id, created_by, updated_by) \
         VALUES ($1, $2, $3, $4, 'tester', 'tester')",
        &[&project_id, &team_id, &budget_id, &object_permission_id],
    )
    .await;
}

async fn assert_no_default_nullable(client: &Client, table: &str, name: &str) {
    let info = column(client, table, name).await;
    assert!(info.is_nullable, "{name} should be nullable");
    assert!(info.default.is_none(), "{name} should have no default");
}

#[tokio::test]
async fn rotation_false_zero_and_nullable_times() {
    harness(|[client]| async move {
        batch_execute(
            &client,
            &migration_sql_through("20250926194702_unnamed_migration"),
        )
        .await;
        let auto_rotate = column(&client, "LiteLLM_VerificationToken", "auto_rotate").await;
        assert!(auto_rotate.is_nullable);
        assert!(auto_rotate.default.unwrap().contains("false"));
        let rotation_count = column(&client, "LiteLLM_VerificationToken", "rotation_count").await;
        assert!(rotation_count.is_nullable);
        assert!(rotation_count.default.unwrap().contains('0'));
        assert_no_default_nullable(&client, "LiteLLM_VerificationToken", "key_rotation_at").await;
        assert_no_default_nullable(&client, "LiteLLM_VerificationToken", "last_rotation_at").await;
        assert_no_default_nullable(&client, "LiteLLM_VerificationToken", "rotation_interval").await;
    })
    .await;
}

#[tokio::test]
async fn prompt_initial_unique_id() {
    harness(|[client]| async move {
        batch_execute(
            &client,
            &migration_sql_through("20250802162330_prompt_table"),
        )
        .await;
        exec_params(
            &client,
            "INSERT INTO \"LiteLLM_PromptTable\" (id, prompt_id, litellm_params, updated_at) \
             VALUES ($1, $2, '{}', CURRENT_TIMESTAMP)",
            &[&"prompt-row-1", &"prompt-1"],
        )
        .await;
        let error = exec_params_err(
            &client,
            "INSERT INTO \"LiteLLM_PromptTable\" (id, prompt_id, litellm_params, updated_at) \
             VALUES ($1, $2, '{}', CURRENT_TIMESTAMP)",
            &[&"prompt-row-2", &"prompt-1"],
        )
        .await;
        assert_eq!(error.code().unwrap().code(), "23505");
    })
    .await;
}

#[tokio::test]
async fn prompt_existing_version_one() {
    harness(|[client]| async move {
        batch_execute(
            &client,
            &migration_sql_through("20250802162330_prompt_table"),
        )
        .await;
        exec_params(
            &client,
            "INSERT INTO \"LiteLLM_PromptTable\" (id, prompt_id, litellm_params, updated_at) \
             VALUES ($1, $2, '{}', CURRENT_TIMESTAMP)",
            &[&"prompt-row-1", &"prompt-1"],
        )
        .await;
        batch_execute(
            &client,
            migration_sql_of("20251119131227_add_prompt_versioning"),
        )
        .await;
        let version = scalar_text(
            &client,
            "SELECT version::TEXT FROM \"LiteLLM_PromptTable\" WHERE id = $1",
            &[&"prompt-row-1"],
        )
        .await;
        assert_eq!(version, "1");
    })
    .await;
}

#[tokio::test]
async fn prompt_different_versions_allowed() {
    harness(|[client]| async move {
        batch_execute(&client, &migration_sql_through("20251119131227_add_prompt_versioning")).await;
        exec_params(
            &client,
            "INSERT INTO \"LiteLLM_PromptTable\" (id, prompt_id, litellm_params, updated_at, version) \
             VALUES ($1, $2, '{}', CURRENT_TIMESTAMP, 1)",
            &[&"prompt-row-1", &"prompt-1"],
        )
        .await;
        exec_params(
            &client,
            "INSERT INTO \"LiteLLM_PromptTable\" (id, prompt_id, litellm_params, updated_at, version) \
             VALUES ($1, $2, '{}', CURRENT_TIMESTAMP, 2)",
            &[&"prompt-row-2", &"prompt-1"],
        )
        .await;
        let count = scalar_text(
            &client,
            "SELECT count(*)::TEXT FROM \"LiteLLM_PromptTable\" WHERE prompt_id = $1",
            &[&"prompt-1"],
        )
        .await;
        assert_eq!(count, "2");
    })
    .await;
}

#[tokio::test]
async fn prompt_duplicate_version_rejected() {
    harness(|[client]| async move {
        batch_execute(&client, &migration_sql_through("20251119131227_add_prompt_versioning")).await;
        exec_params(
            &client,
            "INSERT INTO \"LiteLLM_PromptTable\" (id, prompt_id, litellm_params, updated_at, version) \
             VALUES ($1, $2, '{}', CURRENT_TIMESTAMP, 1)",
            &[&"prompt-row-1", &"prompt-1"],
        )
        .await;
        let error = exec_params_err(
            &client,
            "INSERT INTO \"LiteLLM_PromptTable\" (id, prompt_id, litellm_params, updated_at, version) \
             VALUES ($1, $2, '{}', CURRENT_TIMESTAMP, 1)",
            &[&"prompt-row-2", &"prompt-1"],
        )
        .await;
        assert_eq!(error.code().unwrap().code(), "23505");
    })
    .await;
}

#[tokio::test]
async fn search_tool_required_params_and_unique_name() {
    harness(|[client]| async move {
        batch_execute(&client, &migration_sql_through("20251023141814_add_search_tool_table")).await;
        assert!(!column(&client, "LiteLLM_SearchToolsTable", "litellm_params").await.is_nullable);
        assert!(column(&client, "LiteLLM_SearchToolsTable", "search_tool_info").await.is_nullable);
        let missing_params = exec_params_err(
            &client,
            "INSERT INTO \"LiteLLM_SearchToolsTable\" (search_tool_id, search_tool_name, updated_at) \
             VALUES ($1, $2, CURRENT_TIMESTAMP)",
            &[&"tool-1", &"tool-name-1"],
        )
        .await;
        assert_eq!(missing_params.code().unwrap().code(), "23502");
        exec_params(
            &client,
            "INSERT INTO \"LiteLLM_SearchToolsTable\" (search_tool_id, search_tool_name, litellm_params, updated_at) \
             VALUES ($1, $2, '{}', CURRENT_TIMESTAMP)",
            &[&"tool-1", &"tool-name-1"],
        )
        .await;
        let duplicate_name = exec_params_err(
            &client,
            "INSERT INTO \"LiteLLM_SearchToolsTable\" (search_tool_id, search_tool_name, litellm_params, updated_at) \
             VALUES ($1, $2, '{}', CURRENT_TIMESTAMP)",
            &[&"tool-2", &"tool-name-1"],
        )
        .await;
        assert_eq!(duplicate_name.code().unwrap().code(), "23505");
    })
    .await;
}

#[tokio::test]
async fn vector_index_unique_name_and_actors() {
    harness(|[client]| async move {
        batch_execute(
            &client,
            &migration_sql_through("20251101131415_add_managed_vector_store_index_table"),
        )
        .await;
        assert!(column(&client, "LiteLLM_ManagedVectorStoreIndexTable", "created_by").await.is_nullable);
        assert!(column(&client, "LiteLLM_ManagedVectorStoreIndexTable", "updated_by").await.is_nullable);
        exec_params(
            &client,
            "INSERT INTO \"LiteLLM_ManagedVectorStoreIndexTable\" (id, index_name, litellm_params, updated_at) \
             VALUES ($1, $2, '{}', CURRENT_TIMESTAMP)",
            &[&"index-row-1", &"index-1"],
        )
        .await;
        let error = exec_params_err(
            &client,
            "INSERT INTO \"LiteLLM_ManagedVectorStoreIndexTable\" (id, index_name, litellm_params, updated_at) \
             VALUES ($1, $2, '{}', CURRENT_TIMESTAMP)",
            &[&"index-row-2", &"index-1"],
        )
        .await;
        assert_eq!(error.code().unwrap().code(), "23505");
    })
    .await;
}

#[tokio::test]
async fn vector_store_nullable_params() {
    harness(|[client]| async move {
        batch_execute(
            &client,
            &migration_sql_through("20250718125714_add_litellm_params_to_vector_stores"),
        )
        .await;
        let info = column(
            &client,
            "LiteLLM_ManagedVectorStoresTable",
            "litellm_params",
        )
        .await;
        assert!(info.is_nullable);
        assert!(info.default.is_none());
    })
    .await;
}

#[tokio::test]
async fn managed_file_storage_fields_no_backfill() {
    harness(|[client]| async move {
        batch_execute(
            &client,
            &migration_sql_through("20251210125210_add_storage_backend_to_managed_files"),
        )
        .await;
        exec_params(
            &client,
            "INSERT INTO \"LiteLLM_ManagedFileTable\" \
             (id, unified_file_id, file_object, model_mappings, updated_at) \
             VALUES ($1, $2, '{}', '{}', CURRENT_TIMESTAMP)",
            &[&"file-row-1", &"unified-1"],
        )
        .await;
        let backend = scalar_opt_text(
            &client,
            "SELECT storage_backend FROM \"LiteLLM_ManagedFileTable\" WHERE id = $1",
            &[&"file-row-1"],
        )
        .await;
        assert_eq!(backend, None);
        let url = scalar_opt_text(
            &client,
            "SELECT storage_url FROM \"LiteLLM_ManagedFileTable\" WHERE id = $1",
            &[&"file-row-1"],
        )
        .await;
        assert_eq!(url, None);
    })
    .await;
}

#[tokio::test]
async fn tag_defaults_and_budget_set_null() {
    harness(|[client]| async move {
        batch_execute(
            &client,
            &migration_sql_through("20251011084309_add_tag_table"),
        )
        .await;
        let spend = column(&client, "LiteLLM_TagTable", "spend").await;
        assert!(!spend.is_nullable);
        assert!(spend.default.unwrap().contains("0.0"));
        insert_budget(&client, "budget-1").await;
        exec_params(
            &client,
            "INSERT INTO \"LiteLLM_TagTable\" (tag_name, budget_id) VALUES ($1, $2)",
            &[&"tag-1", &"budget-1"],
        )
        .await;
        exec_params(
            &client,
            "DELETE FROM \"LiteLLM_BudgetTable\" WHERE budget_id = $1",
            &[&"budget-1"],
        )
        .await;
        let budget_id = scalar_opt_text(
            &client,
            "SELECT budget_id FROM \"LiteLLM_TagTable\" WHERE tag_name = $1",
            &[&"tag-1"],
        )
        .await;
        assert_eq!(budget_id, None);
    })
    .await;
}

#[tokio::test]
async fn singleton_sso_cache_ui_ids_and_required_json() {
    harness(|[client]| async move {
        batch_execute(&client, &migration_sql_through("20251209112246_add_ui_settings_table")).await;
        exec_params(
            &client,
            "INSERT INTO \"LiteLLM_SSOConfig\" (sso_settings, updated_at) VALUES ('{}', CURRENT_TIMESTAMP)",
            &[],
        )
        .await;
        let sso_id = scalar_text(&client, "SELECT id FROM \"LiteLLM_SSOConfig\"", &[]).await;
        assert_eq!(sso_id, "sso_config");
        exec_params(
            &client,
            "INSERT INTO \"LiteLLM_CacheConfig\" (cache_settings, updated_at) VALUES ('{}', CURRENT_TIMESTAMP)",
            &[],
        )
        .await;
        let cache_id = scalar_text(&client, "SELECT id FROM \"LiteLLM_CacheConfig\"", &[]).await;
        assert_eq!(cache_id, "cache_config");
        exec_params(
            &client,
            "INSERT INTO \"LiteLLM_UISettings\" (ui_settings, updated_at) VALUES ('{}', CURRENT_TIMESTAMP)",
            &[],
        )
        .await;
        let ui_id = scalar_text(&client, "SELECT id FROM \"LiteLLM_UISettings\"", &[]).await;
        assert_eq!(ui_id, "ui_settings");
        let missing_settings = exec_params_err(
            &client,
            "INSERT INTO \"LiteLLM_CacheConfig\" (id, updated_at) VALUES ('other', CURRENT_TIMESTAMP)",
            &[],
        )
        .await;
        assert_eq!(missing_settings.code().unwrap().code(), "23502");
    })
    .await;
}

#[tokio::test]
async fn project_defaults_and_model_limit_objects() {
    harness(|[client]| async move {
        batch_execute(
            &client,
            &migration_sql_through("20251113000001_add_project_fields"),
        )
        .await;
        exec_params(
            &client,
            "INSERT INTO \"LiteLLM_ProjectTable\" (project_id, created_by, updated_by) \
             VALUES ($1, 'tester', 'tester')",
            &[&"project-1"],
        )
        .await;
        let row = query_one_row(
            &client,
            "SELECT metadata = '{}'::jsonb, model_spend = '{}'::jsonb, blocked = false, \
             spend = 0.0, model_rpm_limit = '{}'::jsonb, model_tpm_limit = '{}'::jsonb \
             FROM \"LiteLLM_ProjectTable\" WHERE project_id = $1",
            &[&"project-1"],
        )
        .await;
        let defaults_hold: Vec<bool> = (0..6).map(|idx| row.get(idx)).collect();
        assert!(defaults_hold.iter().all(|&holds| holds));
    })
    .await;
}

#[tokio::test]
async fn project_three_set_null_references() {
    harness(|[client]| async move {
        batch_execute(
            &client,
            &migration_sql_through("20251113000000_add_project_table"),
        )
        .await;
        insert_team(&client, "team-1").await;
        insert_budget(&client, "budget-1").await;
        insert_object_permission(&client, "permission-1").await;
        insert_project(&client, "project-1", "team-1", "budget-1", "permission-1").await;
        batch_execute(
            &client,
            "DELETE FROM \"LiteLLM_TeamTable\"; \
             DELETE FROM \"LiteLLM_BudgetTable\"; \
             DELETE FROM \"LiteLLM_ObjectPermissionTable\";",
        )
        .await;
        let row = query_one_row(
            &client,
            "SELECT team_id IS NULL, budget_id IS NULL, object_permission_id IS NULL \
             FROM \"LiteLLM_ProjectTable\" WHERE project_id = $1",
            &[&"project-1"],
        )
        .await;
        let references_cleared: Vec<bool> = (0..3).map(|idx| row.get(idx)).collect();
        assert!(references_cleared.iter().all(|&cleared| cleared));
    })
    .await;
}

#[tokio::test]
async fn key_project_set_null_reference() {
    harness(|[client]| async move {
        batch_execute(
            &client,
            &migration_sql_through("20251113000000_add_project_table"),
        )
        .await;
        exec_params(
            &client,
            "INSERT INTO \"LiteLLM_ProjectTable\" (project_id, created_by, updated_by) \
             VALUES ($1, 'tester', 'tester')",
            &[&"project-1"],
        )
        .await;
        exec_params(
            &client,
            "INSERT INTO \"LiteLLM_VerificationToken\" (token, project_id) VALUES ($1, $2)",
            &[&"token-1", &"project-1"],
        )
        .await;
        exec_params(
            &client,
            "DELETE FROM \"LiteLLM_ProjectTable\" WHERE project_id = $1",
            &[&"project-1"],
        )
        .await;
        let project_id = scalar_opt_text(
            &client,
            "SELECT project_id FROM \"LiteLLM_VerificationToken\" WHERE token = $1",
            &[&"token-1"],
        )
        .await;
        assert_eq!(project_id, None);
    })
    .await;
}

#[tokio::test]
async fn agent_required_card_and_actor_defaults() {
    harness(|[client]| async move {
        batch_execute(
            &client,
            &migration_sql_through("20251114182247_agents_table"),
        )
        .await;
        let card = column(&client, "LiteLLM_AgentsTable", "agent_card_params").await;
        assert!(!card.is_nullable);
        let litellm_params = column(&client, "LiteLLM_AgentsTable", "litellm_params").await;
        assert!(litellm_params.is_nullable);
        let created_by = column(&client, "LiteLLM_AgentsTable", "created_by").await;
        assert!(!created_by.is_nullable);
        assert!(created_by.default.is_none());
        let updated_by = column(&client, "LiteLLM_AgentsTable", "updated_by").await;
        assert!(!updated_by.is_nullable);
        assert!(updated_by.default.is_none());
        let created_at = column(&client, "LiteLLM_AgentsTable", "created_at").await;
        assert!(!created_at.is_nullable);
        assert!(created_at.default.is_some());
        let updated_at = column(&client, "LiteLLM_AgentsTable", "updated_at").await;
        assert!(!updated_at.is_nullable);
        assert!(updated_at.default.is_some());
        let missing_card = exec_params_err(
            &client,
            "INSERT INTO \"LiteLLM_AgentsTable\" (agent_id, agent_name, created_by, updated_by) \
             VALUES ($1, $2, 'tester', 'tester')",
            &[&"agent-1", &"agent-name-1"],
        )
        .await;
        assert_eq!(missing_card.code().unwrap().code(), "23502");
    })
    .await;
}
