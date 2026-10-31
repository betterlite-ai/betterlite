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
    sha256: "0bf3d5cabe50d5d7249eb505cf2ec6fe2d8b49dc8b496dceb2179a357ed4725e",
};

pub const MIGRATION_CATALOG: MigrationCatalog = MigrationCatalog {
    files: &[BASELINE_MIGRATION],
};

mod tests;
