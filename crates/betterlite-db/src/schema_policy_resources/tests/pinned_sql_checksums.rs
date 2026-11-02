use sha2::{Digest as _, Sha256};

const EXPECTED_DIRECTORIES: [&str; 32] = [
    "20260219181415_baseline_diff",
    "20260220124742_add_spec_path_to_mcp_servers",
    "20260220153844_add_composite_index_aggregate_tables",
    "20260221000000_ensure_project_id_verification_token",
    "20260221183800_add_policy_versioning",
    "20260222000000_add_batch_processed_to_managed_object_table",
    "20260224201417_spend_logs_request_duration",
    "20260224203854_add_agent_object_permissions_table",
    "20260226000000_add_blocked_tools_to_object_permission",
    "20260226120000_add_spend_log_tool_index",
    "20260226202727_add_agent_id_to_delete_keys",
    "20260228000000_add_claude_code_plugin_table",
    "20260228100000_add_spend_logs_composite_index",
    "20260228110000_mcp_default_public_internet_true",
    "20260228170127_support_team_based_guardrails",
    "20260303000000_update_tool_table_policies",
    "20260304175016_add_spend_to_agent_table",
    "20260305000000_add_agent_headers",
    "20260305000000_add_rate_limits_to_agents",
    "20260306175056_add_configs_override_table",
    "20260306233848_schema_sync",
    "20260309000000_add_mcp_approval_status",
    "20260309000001_add_mcp_source_url",
    "20260312124619_schema_sync",
    "20260318140652_add_index_to_team_table",
    "20260319000000_restore_mcp_approval_fields",
    "20260321000000_add_mcp_toolsets",
    "20260331000000_add_prompt_environment_and_created_by",
    "20260401000000_add_budget_limits",
    "20260401000000_add_team_member_model_scope",
    "20260414140000_add_mcp_server_instructions",
    "20260415120000_health_check_latest_per_model_index",
];

fn assert_catalog_matches(files: &[MigrationFile], expected_directories: &[&str; 32]) {
    for (file, expected_directory) in files.iter().zip(expected_directories.iter().copied()) {
        assert_eq!(file.directory, expected_directory);
        let digest = hex::encode(Sha256::digest(file.bytes));
        assert_eq!(
            digest, file.sha256,
            "checksum mismatch for {expected_directory}"
        );
    }
}

#[test]
fn pinned_sql_checksums() {
    assert_eq!(MIGRATION_CATALOG.files.len(), EXPECTED_DIRECTORIES.len());
    assert_catalog_matches(MIGRATION_CATALOG.files, &EXPECTED_DIRECTORIES);

    let mut sorted: Vec<&str> = EXPECTED_DIRECTORIES.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(
        sorted.len(),
        EXPECTED_DIRECTORIES.len(),
        "two entries share a full directory name despite equal timestamp prefixes"
    );

    assert_eq!(
        PINNED_MIGRATION_CATALOG.files.len(),
        EXPECTED_DIRECTORIES.len()
    );
    assert_catalog_matches(PINNED_MIGRATION_CATALOG.files, &EXPECTED_DIRECTORIES);
}
