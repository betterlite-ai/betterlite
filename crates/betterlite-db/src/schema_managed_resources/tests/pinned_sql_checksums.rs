use sha2::{Digest as _, Sha256};

const EXPECTED_DIRECTORIES: [&str; 29] = [
    "20250707230009_add_mcp_namespaced_tool_name",
    "20250711220620_add_stdio_mcp",
    "20250718125714_add_litellm_params_to_vector_stores",
    "20250802162330_prompt_table",
    "20250806095134_rename_alias_to_server_name_mcp_table",
    "20250918083359_drop_spec_version_column_from_mcp_table",
    "20250926194702_unnamed_migration",
    "20251003165142_add_allowed_tools_to_mcp",
    "20251003190954_extra_headers_to_mcp_table",
    "20251006143948_add_mcp_tool_permissions",
    "20251011084309_add_tag_table",
    "20251023141814_add_search_tool_table",
    "20251031181430_add_cache_config_table",
    "20251101131415_add_managed_vector_store_index_table",
    "20251103072422_add_static_headers",
    "20251104220043_add_credentials_to_mcp_servers",
    "20251113000000_add_project_table",
    "20251113000001_add_project_fields",
    "20251114173537_add_request_id_to_daily_tag_spend",
    "20251114180624_Add_org_usage_table",
    "20251114182247_agents_table",
    "20251119131227_add_prompt_versioning",
    "20251122125322_Add organization_id to spend logs",
    "20251204124859_add_end_user_spend_table",
    "20251204142718_add_agent_permissions",
    "20251209112246_add_ui_settings_table",
    "20251210125210_add_storage_backend_to_managed_files",
    "20251210205007_add_daily_agent_spend_table",
    "20251211100212_schema_sync",
];

fn assert_catalog_matches(files: &[MigrationFile], expected_directories: &[&str; 29]) {
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
