use sha2::{Digest as _, Sha256};

const EXPECTED_DIRECTORIES: [&str; 25] = [
    "20251219110931_add_deleted_keys_and_deleted_teams_tables",
    "20251220144550_schema_update",
    "20260102131258_add_metadata_urls_to_mcp_servers",
    "20260105151539_add_allow_all_keys_to_mcp_servers",
    "20260106155622_add_endpoint_to_daily_activity_tables",
    "20260107111013_add_router_settings_to_keys_teams",
    "20260108_add_user_email_lower_idx",
    "20260116142756_update_deleted_keys_teams_table_routing_settings",
    "20260123131407_add_policy_tables_and_policies_field",
    "20260131150814_add_team_user_to_vector_stores",
    "20260203120000_add_deprecated_verification_token_table",
    "20260205091235_allow_team_guardrail_config",
    "20260205144610_add_soft_budget_to_team_table",
    "20260207093506_add_available_on_public_internet_to_mcp_servers",
    "20260207110613_add_soft_budget_to_deleted_teams_table",
    "20260209085821_add_verificationtoken_indexes",
    "20260212103349_adjust_tags_policy_table",
    "20260212143306_add_access_group_table",
    "20260213105436_add_managed_vector_store_table",
    "20260213170952_access_group_change_to_model_name",
    "20260214094754_schema_sync",
    "20260214163027_add_pipeline_to_policy_table",
    "20260214185341_object_permissions_for_end_users",
    "20260218231534_add_last_active_to_key_table",
    "20260219105005_add_project_id_to_deleted_keys",
];

fn assert_catalog_matches(files: &[MigrationFile], expected_directories: &[&str; 25]) {
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
}
