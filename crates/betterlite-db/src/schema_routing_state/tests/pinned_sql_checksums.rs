use sha2::{Digest as _, Sha256};

const EXPECTED_DIRECTORIES: [&str; 36] = [
    "20260418000000_add_adaptive_router_tables",
    "20260421120000_add_memory_table",
    "20260421135425_add_team_membership_total_spend",
    "20260429120000_search_tools_on_object_permission",
    "20260429161855_workflow_runs_tables",
    "20260501195714_managed_resource_team_owner",
    "20260513120000_add_delegate_auth_to_upstream_to_mcp_servers",
    "20260514120000_add_blocked_to_proxy_model_table",
    "20260520120000_add_mcp_env_vars",
    "20260526120000_add_oauth_passthrough_to_mcp_servers",
    "20260604120000_add_oauth2_flow_to_mcp_servers",
    "20260605182307_add_timeout_to_mcp_server_table",
    "20260626120000_add_mcp_tool_search_enabled",
    "20260629000000_add_max_concurrent_requests_to_mcp_server_table",
    "20260630120000_add_token_exchange_to_mcp_servers",
    "20260630190000_add_budget_fallbacks_to_litellm_verification_token",
    "20260703120000_add_token_exchange_profile_to_mcp_servers",
    "20260710000000_add_dcr_bridge_to_mcp_server_table",
    "20260713010000_add_ptu_columns_to_daily_team_spend",
    "20260713230852_add_key_type_to_litellm_verification_token",
    "20260715000000_add_issuer_to_mcp_server_table",
    "20260717000000_add_compression_saved_tokens",
    "20260717000000_add_mcp_server_oauth_client_table",
    "20260717000000_cascade_delete_jwt_key_mapping_on_token_delete",
    "20260718000000_add_savings_spend",
    "20260721000000_add_sso_identity_assertion",
    "20260724000000_add_spend_log_tool_index_start_time_idx",
    "20260725000000_add_daily_tool_spend",
    "20260729000000_add_reload_tracking_to_litellm_config",
    "20260730000000_add_api_key_and_request_tags_to_managed_object_table",
    "20260731000000_add_autorouter_savings_spend",
    "20260803000000_add_daily_gateway_requests",
    "20260804162853_add_budget_window_spend_table",
    "20260805000000_add_autorouter_session_rollup",
    "20260807000000_add_autorouter_session_tier_turns",
    "20260810000000_add_verificationtoken_settings_updated_at",
];

fn assert_catalog_matches(files: &[MigrationFile], expected_directories: &[&str; 36]) {
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
