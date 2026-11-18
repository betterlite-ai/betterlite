use betterlite_db::migration_deploy::catalog::Migration;
use betterlite_db::migration_ledger::baseline::PACKAGED_CATALOGS;

// baseline detection compares this file's SHA256 with the one recorded for the packaged migrations
pub const SCHEMA_PRISMA: &[u8] =
    include_bytes!("../../../../betterlite-db/src/schema_contract/fixtures/proxy/schema.prisma");

// The migration files the crates ship, in the timestamp order prisma migrate applies them.
#[must_use]
pub fn migrations() -> Vec<Migration> {
    let mut migrations: Vec<Migration> = PACKAGED_CATALOGS
        .into_iter()
        .flat_map(|catalog| catalog.files)
        .map(|file| Migration {
            name: file.directory.to_owned(),
            checksum: file.sha256.to_owned(),
            sql: file.bytes.to_vec(),
        })
        .collect();
    migrations.sort_by_key(|migration| migration.name.clone());
    migrations
}
