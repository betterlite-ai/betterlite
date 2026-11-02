use sha2::{Digest as _, Sha256};
const EXPECTED_DIRECTORIES: [&str; 7] = [
    "20260921190000_agent_identity",
    "20260923000000_add_agent_kill_switch",
    "20260925000000_add_mcp_pinned_tools",
    "20260928000000_add_daily_model_usage",
    "20260930000000_agent_engine",
    "20261001000000_lens_run_history",
    "20261001100000_rename_lens",
];
fn assert_catalog_matches(files: &[MigrationFile], expected_directories: &[&str; 7]) {
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
