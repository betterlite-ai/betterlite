use sha2::{Digest as _, Sha256};

const EXPECTED_DIRECTORIES: [&str; 37] = [
    "20260811172448_add_shadow_eval",
    "20260813180408_add_shadow_eval_direction",
    "20260814000000_add_proxy_worker_heartbeat",
    "20260817000000_shadow_eval_multi_key",
    "20260817143646_add_daily_guardrail_usage_units",
    "20260818000000_add_spend_log_timestamps",
    "20260818224500_add_shadow_eval_stopped_by",
    "20260819000000_shadow_eval_max_budget",
    "20260823000000_add_spend_logs_api_key_starttime_index",
    "20260824000000_add_gateway_injected_caching_savings_spend",
    "20260828000000_shadow_eval_cost_comparison",
    "20260829000000_add_model_access_group_budget_table",
    "20260831000000_shadow_eval_typed_targets",
    "20260831120000_add_litellm_call_id_spend_logs",
    "20260831120001_spend_logs_litellm_call_id_index",
    "20260901000000_shadow_eval_multi_router",
    "20260901000001_add_guardrail_usage_units_cost",
    "20260903230000_add_org_id_to_managed_object_table",
    "20260904000000_shadow_eval_model_scope",
    "20260905120000_add_per_server_oauth_discovery_to_mcp_servers",
    "20260907000000_add_autorouter_classifier_cost",
    "20260910000000_add_autorouter_session_baseline_models",
    "20260910000000_skills_on_object_permission",
    "20260913000000_add_tpd_limit",
    "20260915000000_add_autorouter_savings_estimate_coverage",
    "20260915000000_add_daily_global_spend",
    "20260915000000_add_daily_response_time",
    "20260915000000_scope_jwt_key_mapping_by_issuer",
    "20260915010000_add_autorouter_baseline_state",
    "20260916000000_add_key_total_spend",
    "20260917000000_add_agent_access_group_ids",
    "20260917000000_add_budget_temp_increase",
    "20260917055603_add_policy_attachment_priority",
    "20260918000000_add_managed_file_content_table",
    "20260919000000_add_autorouter_user_session_rollup",
    "20260920041500_add_policy_attachment_is_default",
    "20260921000000_add_password_reset_columns",
];

fn assert_catalog_matches(files: &[MigrationFile], expected_directories: &[&str; 37]) {
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
