#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MigrationFile {
    pub directory: &'static str,
    pub bytes: &'static [u8],
    pub sha256: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MigrationCatalog {
    pub files: &'static [MigrationFile],
}

pub const BASELINE_MIGRATION: MigrationFile = MigrationFile {
    directory: "20250326162113_baseline",
    bytes: include_bytes!("../migrations/20250326162113_baseline/migration.sql"),
    sha256: "35afca4826335f8bd2bc2b733c0540de51c6a199d39c27a2d1910c53e1a02647",
};

pub const MIGRATION_CATALOG: MigrationCatalog = MigrationCatalog {
    files: &[BASELINE_MIGRATION],
};

mod tests;
