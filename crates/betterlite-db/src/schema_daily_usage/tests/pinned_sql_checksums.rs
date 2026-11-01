use sha2::{Digest as _, Sha256};

const EXPECTED_DIRECTORIES: [&str; 28] = [
    "20250326171002_add_daily_user_table",
    "20250327180120_add_api_requests_to_daily_user_table",
    "20250329084805_new_cron_job_table",
    "20250331215456_track_success_and_failed_requests_daily_agg_table",
    "20250411215431_add_managed_file_table",
    "20250412081753_team_member_permissions",
    "20250415151647_add_cache_read_write_tokens_daily_spend_transactions",
    "20250415191926_add_daily_team_table",
    "20250416115320_add_tag_table_to_db",
    "20250416151339_drop_tag_uniqueness_requirement",
    "20250416185146_add_allowed_routes_litellm_verification_token",
    "20250425182129_add_session_id",
    "20250430193429_add_managed_vector_stores",
    "20250507161526_add_mcp_table_to_db",
    "20250507161527_add_health_check_fields_to_mcp_servers",
    "20250507184818_add_mcp_key_team_permission_mgmt",
    "20250508072103_add_status_to_spendlogs",
    "20250509141545_use_big_int_for_daily_spend_tables",
    "20250510142544_add_session_id_index_spend_logs",
    "20250514142245_add_guardrails_table",
    "20250522223020_managed_object_table",
    "20250526154401_allow_null_entity_id",
    "20250528185438_add_vector_stores_to_object_permissions",
    "20250603210143_cascade_budget_changes",
    "20250618225828_add_health_check_table",
    "20250625145206_cascade_budget_and_loosen_managed_file_json",
    "20250625213625_add_status_to_managed_object_table",
    "20250707212517_add_mcp_info_column_mcp_servers",
];

fn assert_catalog_matches(files: &[MigrationFile], expected_directories: &[&str; 28]) {
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
