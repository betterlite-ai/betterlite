use betterlite_db::migration_deploy::catalog::Migration;
use betterlite_db::{
    schema_agent_identity, schema_baseline, schema_budget_windows, schema_daily_usage,
    schema_identity_history, schema_managed_resources, schema_policy_resources,
    schema_routing_state,
};

// The schema.prisma the packaged migrations were generated from; betterlite-db's three copies of it are
// byte-identical and this one records the packaged schema SHA256 that baseline detection compares.
pub const SCHEMA_PRISMA: &[u8] =
    include_bytes!("../../../../betterlite-db/src/schema_contract/fixtures/proxy/schema.prisma");

// The migration files the crates ship, in the timestamp order prisma migrate applies them.
pub fn migrations() -> Vec<Migration> {
    let catalogs = [
        schema_baseline::MIGRATION_CATALOG,
        schema_daily_usage::MIGRATION_CATALOG,
        schema_managed_resources::MIGRATION_CATALOG,
        schema_identity_history::MIGRATION_CATALOG,
        schema_policy_resources::MIGRATION_CATALOG,
        schema_routing_state::MIGRATION_CATALOG,
        schema_budget_windows::MIGRATION_CATALOG,
        schema_agent_identity::MIGRATION_CATALOG,
    ];
    let mut migrations: Vec<Migration> = catalogs
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
