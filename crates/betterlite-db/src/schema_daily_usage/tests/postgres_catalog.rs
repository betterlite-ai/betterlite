struct ExpectedIndex {
    name: &'static str,
    columns: &'static [&'static str],
    unique: bool,
}

const EXPECTED_INDEXES: &[ExpectedIndex] = &[
    ExpectedIndex {
        name: "LiteLLM_DailyUserSpend_date_idx",
        columns: &["date"],
        unique: false,
    },
    ExpectedIndex {
        name: "LiteLLM_DailyUserSpend_user_id_idx",
        columns: &["user_id"],
        unique: false,
    },
    ExpectedIndex {
        name: "LiteLLM_DailyUserSpend_api_key_idx",
        columns: &["api_key"],
        unique: false,
    },
    ExpectedIndex {
        name: "LiteLLM_DailyUserSpend_model_idx",
        columns: &["model"],
        unique: false,
    },
    ExpectedIndex {
        name: "LiteLLM_DailyUserSpend_user_id_date_api_key_model_custom_ll_key",
        columns: &["user_id", "date", "api_key", "model", "custom_llm_provider"],
        unique: true,
    },
    ExpectedIndex {
        name: "LiteLLM_ManagedFileTable_unified_file_id_key",
        columns: &["unified_file_id"],
        unique: true,
    },
    ExpectedIndex {
        name: "LiteLLM_ManagedFileTable_unified_file_id_idx",
        columns: &["unified_file_id"],
        unique: false,
    },
    ExpectedIndex {
        name: "LiteLLM_DailyTeamSpend_date_idx",
        columns: &["date"],
        unique: false,
    },
    ExpectedIndex {
        name: "LiteLLM_DailyTeamSpend_team_id_idx",
        columns: &["team_id"],
        unique: false,
    },
    ExpectedIndex {
        name: "LiteLLM_DailyTeamSpend_api_key_idx",
        columns: &["api_key"],
        unique: false,
    },
    ExpectedIndex {
        name: "LiteLLM_DailyTeamSpend_model_idx",
        columns: &["model"],
        unique: false,
    },
    ExpectedIndex {
        name: "LiteLLM_DailyTeamSpend_team_id_date_api_key_model_custom_ll_key",
        columns: &["team_id", "date", "api_key", "model", "custom_llm_provider"],
        unique: true,
    },
    ExpectedIndex {
        name: "LiteLLM_DailyTagSpend_date_idx",
        columns: &["date"],
        unique: false,
    },
    ExpectedIndex {
        name: "LiteLLM_DailyTagSpend_tag_idx",
        columns: &["tag"],
        unique: false,
    },
    ExpectedIndex {
        name: "LiteLLM_DailyTagSpend_api_key_idx",
        columns: &["api_key"],
        unique: false,
    },
    ExpectedIndex {
        name: "LiteLLM_DailyTagSpend_model_idx",
        columns: &["model"],
        unique: false,
    },
    ExpectedIndex {
        name: "LiteLLM_DailyTagSpend_tag_date_api_key_model_custom_llm_pro_key",
        columns: &["tag", "date", "api_key", "model", "custom_llm_provider"],
        unique: true,
    },
    ExpectedIndex {
        name: "LiteLLM_SpendLogs_session_id_idx",
        columns: &["session_id"],
        unique: false,
    },
    ExpectedIndex {
        name: "LiteLLM_GuardrailsTable_guardrail_name_key",
        columns: &["guardrail_name"],
        unique: true,
    },
    ExpectedIndex {
        name: "LiteLLM_ManagedObjectTable_unified_object_id_key",
        columns: &["unified_object_id"],
        unique: true,
    },
    ExpectedIndex {
        name: "LiteLLM_ManagedObjectTable_unified_object_id_idx",
        columns: &["unified_object_id"],
        unique: false,
    },
    ExpectedIndex {
        name: "LiteLLM_ManagedObjectTable_model_object_id_key",
        columns: &["model_object_id"],
        unique: true,
    },
    ExpectedIndex {
        name: "LiteLLM_ManagedObjectTable_model_object_id_idx",
        columns: &["model_object_id"],
        unique: false,
    },
    ExpectedIndex {
        name: "LiteLLM_HealthCheckTable_model_name_idx",
        columns: &["model_name"],
        unique: false,
    },
    ExpectedIndex {
        name: "LiteLLM_HealthCheckTable_checked_at_idx",
        columns: &["checked_at"],
        unique: false,
    },
    ExpectedIndex {
        name: "LiteLLM_HealthCheckTable_status_idx",
        columns: &["status"],
        unique: false,
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

#[tokio::test]
async fn postgres_catalog() {
    harness(|[client]| async move {
        client.batch_execute(&migration_sql_all()).await.unwrap();
        assert_expected_indexes(&client, EXPECTED_INDEXES).await;
        assert!(
            !index_exists(&client, "LiteLLM_DailyTagSpend_tag_key").await,
            "the tag-only unique index must be dropped by the final checkpoint"
        );
    })
    .await;
}
