struct ExpectedIndex {
    name: &'static str,
    columns: &'static [&'static str],
    unique: bool,
}

const EXPECTED_INDEXES: &[ExpectedIndex] = &[
    ExpectedIndex {
        name: "LiteLLM_DailyTagSpend_mcp_namespaced_tool_name_idx",
        columns: &["mcp_namespaced_tool_name"],
        unique: false,
    },
    ExpectedIndex {
        name: "LiteLLM_DailyTagSpend_tag_date_api_key_model_custom_llm_pro_key",
        columns: &[
            "tag",
            "date",
            "api_key",
            "model",
            "custom_llm_provider",
            "mcp_namespaced_tool_name",
        ],
        unique: true,
    },
    ExpectedIndex {
        name: "LiteLLM_DailyTeamSpend_mcp_namespaced_tool_name_idx",
        columns: &["mcp_namespaced_tool_name"],
        unique: false,
    },
    ExpectedIndex {
        name: "LiteLLM_DailyTeamSpend_team_id_date_api_key_model_custom_ll_key",
        columns: &[
            "team_id",
            "date",
            "api_key",
            "model",
            "custom_llm_provider",
            "mcp_namespaced_tool_name",
        ],
        unique: true,
    },
    ExpectedIndex {
        name: "LiteLLM_DailyUserSpend_mcp_namespaced_tool_name_idx",
        columns: &["mcp_namespaced_tool_name"],
        unique: false,
    },
    ExpectedIndex {
        name: "LiteLLM_DailyUserSpend_user_id_date_api_key_model_custom_ll_key",
        columns: &[
            "user_id",
            "date",
            "api_key",
            "model",
            "custom_llm_provider",
            "mcp_namespaced_tool_name",
        ],
        unique: true,
    },
    ExpectedIndex {
        name: "LiteLLM_SearchToolsTable_search_tool_name_key",
        columns: &["search_tool_name"],
        unique: true,
    },
    ExpectedIndex {
        name: "LiteLLM_ManagedVectorStoreIndexTable_index_name_key",
        columns: &["index_name"],
        unique: true,
    },
    ExpectedIndex {
        name: "LiteLLM_AgentsTable_agent_name_key",
        columns: &["agent_name"],
        unique: true,
    },
    ExpectedIndex {
        name: "LiteLLM_PromptTable_prompt_id_idx",
        columns: &["prompt_id"],
        unique: false,
    },
    ExpectedIndex {
        name: "LiteLLM_PromptTable_prompt_id_version_key",
        columns: &["prompt_id", "version"],
        unique: true,
    },
    ExpectedIndex {
        name: "LiteLLM_DailyOrganizationSpend_date_idx",
        columns: &["date"],
        unique: false,
    },
    ExpectedIndex {
        name: "LiteLLM_DailyOrganizationSpend_organization_id_idx",
        columns: &["organization_id"],
        unique: false,
    },
    ExpectedIndex {
        name: "LiteLLM_DailyOrganizationSpend_api_key_idx",
        columns: &["api_key"],
        unique: false,
    },
    ExpectedIndex {
        name: "LiteLLM_DailyOrganizationSpend_model_idx",
        columns: &["model"],
        unique: false,
    },
    ExpectedIndex {
        name: "LiteLLM_DailyOrganizationSpend_mcp_namespaced_tool_name_idx",
        columns: &["mcp_namespaced_tool_name"],
        unique: false,
    },
    ExpectedIndex {
        name: "LiteLLM_DailyOrganizationSpend_organization_id_date_api_key_key",
        columns: &[
            "organization_id",
            "date",
            "api_key",
            "model",
            "custom_llm_provider",
            "mcp_namespaced_tool_name",
        ],
        unique: true,
    },
    ExpectedIndex {
        name: "LiteLLM_DailyEndUserSpend_date_idx",
        columns: &["date"],
        unique: false,
    },
    ExpectedIndex {
        name: "LiteLLM_DailyEndUserSpend_end_user_id_idx",
        columns: &["end_user_id"],
        unique: false,
    },
    ExpectedIndex {
        name: "LiteLLM_DailyEndUserSpend_api_key_idx",
        columns: &["api_key"],
        unique: false,
    },
    ExpectedIndex {
        name: "LiteLLM_DailyEndUserSpend_model_idx",
        columns: &["model"],
        unique: false,
    },
    ExpectedIndex {
        name: "LiteLLM_DailyEndUserSpend_mcp_namespaced_tool_name_idx",
        columns: &["mcp_namespaced_tool_name"],
        unique: false,
    },
    ExpectedIndex {
        name: "LiteLLM_DailyEndUserSpend_end_user_id_date_api_key_model_cu_key",
        columns: &[
            "end_user_id",
            "date",
            "api_key",
            "model",
            "custom_llm_provider",
            "mcp_namespaced_tool_name",
        ],
        unique: true,
    },
    ExpectedIndex {
        name: "LiteLLM_DailyAgentSpend_date_idx",
        columns: &["date"],
        unique: false,
    },
    ExpectedIndex {
        name: "LiteLLM_DailyAgentSpend_agent_id_idx",
        columns: &["agent_id"],
        unique: false,
    },
    ExpectedIndex {
        name: "LiteLLM_DailyAgentSpend_api_key_idx",
        columns: &["api_key"],
        unique: false,
    },
    ExpectedIndex {
        name: "LiteLLM_DailyAgentSpend_model_idx",
        columns: &["model"],
        unique: false,
    },
    ExpectedIndex {
        name: "LiteLLM_DailyAgentSpend_mcp_namespaced_tool_name_idx",
        columns: &["mcp_namespaced_tool_name"],
        unique: false,
    },
    ExpectedIndex {
        name: "LiteLLM_DailyAgentSpend_agent_id_date_api_key_model_custom__key",
        columns: &[
            "agent_id",
            "date",
            "api_key",
            "model",
            "custom_llm_provider",
            "mcp_namespaced_tool_name",
        ],
        unique: true,
    },
];

async fn assert_expected_indexes(client: &Client, expected_indexes: &[ExpectedIndex]) {
    for expected in expected_indexes {
        let facts = index_facts(client, expected.name).await;
        assert_eq!(
            facts.columns, expected.columns,
            "columns for {}",
            expected.name
        );
        assert_eq!(
            facts.unique, expected.unique,
            "uniqueness for {}",
            expected.name
        );
    }
}

async fn assert_foreign_keys(client: &Client) {
    assert_rule(client, "LiteLLM_TagTable_budget_id_fkey", 'n').await;
    assert_rule(client, "LiteLLM_ProjectTable_team_id_fkey", 'n').await;
    assert_rule(client, "LiteLLM_ProjectTable_budget_id_fkey", 'n').await;
    assert_rule(
        client,
        "LiteLLM_ProjectTable_object_permission_id_fkey",
        'n',
    )
    .await;
    assert_rule(client, "LiteLLM_VerificationToken_project_id_fkey", 'n').await;
}

#[tokio::test]
async fn postgres_catalog() {
    harness(|[client]| async move {
        batch_execute(&client, &migration_sql_all()).await;
        assert_expected_indexes(&client, EXPECTED_INDEXES).await;
        assert_foreign_keys(&client).await;
        assert!(
            !index_exists(&client, "LiteLLM_PromptTable_prompt_id_key").await,
            "the prompt_id-only unique index must be dropped once versioning lands"
        );
        assert!(
            !column_exists(&client, "LiteLLM_MCPServerTable", "spec_version").await,
            "spec_version must stay dropped"
        );
    })
    .await;
}
