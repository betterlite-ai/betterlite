#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_db::database_tls::translation::strip_prisma_query_params;
use betterlite_db::migration_drift::MigrationDriftCheck;
use betterlite_db::migration_ledger::environment::prisma_schema_param;
use secrecy::{ExposeSecret as _, SecretString};

use super::Settings;
use crate::loader::ConfigLoader;

fn variables(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn load(pairs: &[(&str, &str)]) -> Settings {
    ConfigLoader {
        lookup: variables(pairs),
    }
    .load("model_list: []\n")
    .expect("config should load")
    .sections
    .migration_drift
}

fn check(settings: &Settings) -> MigrationDriftCheck {
    MigrationDriftCheck {
        database_url: settings.inner.database_url.clone(),
        direct_url: settings.inner.direct_url.clone(),
    }
}

fn shown(value: Option<&SecretString>) -> Option<&str> {
    value.map(SecretString::expose_secret)
}

fn repair_url(settings: &MigrationDriftCheck) -> String {
    let database = settings.database_url.as_ref().expect("DATABASE_URL is set");
    let chosen = settings.direct_url.as_ref().unwrap_or(database);
    strip_prisma_query_params(chosen.expose_secret()).expect("the database URL parses")
}

fn schema_of(settings: &MigrationDriftCheck) -> Option<String> {
    let database = settings.database_url.as_ref()?;
    prisma_schema_param(database.expose_secret()).unwrap_or(None)
}

#[test]
fn loads_settings() {
    let database = "postgresql://writer/db?schema=tenant_a&sslmode=require";
    let direct = "postgresql://direct:6543/db?pgbouncer=true&schema=pooled&connection_limit=5";
    let loaded = load(&[("DATABASE_URL", database), ("DIRECT_URL", direct)]);
    assert_eq!(shown(loaded.inner.database_url.as_ref()), Some(database));
    assert_eq!(shown(loaded.inner.direct_url.as_ref()), Some(direct));

    let settings = check(&loaded);
    assert_eq!(repair_url(&settings), "postgresql://direct:6543/db");
    assert_eq!(schema_of(&settings).as_deref(), Some("tenant_a"));
}

#[test]
fn defaults_and_precedence() {
    let absent = load(&[]);
    let absent_settings = check(&absent);
    assert!(absent_settings.database_url.is_none());
    assert!(absent_settings.direct_url.is_none());

    let database = "postgresql://writer/db?schema=tenant_a&sslmode=require&pool_timeout=30";
    let direct = "postgresql://direct:6543/db?pgbouncer=true";
    let both = load(&[("DATABASE_URL", database), ("DIRECT_URL", direct)]);
    let both_settings = check(&both);
    assert_eq!(repair_url(&both_settings), "postgresql://direct:6543/db");
    assert_eq!(schema_of(&both_settings).as_deref(), Some("tenant_a"));

    let writer = load(&[("DATABASE_URL", database)]);
    let writer_settings = check(&writer);
    assert!(writer_settings.direct_url.is_none());
    assert_eq!(
        repair_url(&writer_settings),
        "postgresql://writer/db?sslmode=require"
    );
}

#[test]
fn invalid_value_fallback() {
    let blank = load(&[("DATABASE_URL", ""), ("DIRECT_URL", "")]);
    assert!(blank.inner.database_url.is_none());
    assert!(blank.inner.direct_url.is_none());

    let malformed = load(&[("DATABASE_URL", "postgresql://[::1/db"), ("DIRECT_URL", "")]);
    assert_eq!(
        shown(malformed.inner.database_url.as_ref()),
        Some("postgresql://[::1/db")
    );
    assert!(malformed.inner.direct_url.is_none());
    let malformed_settings = check(&malformed);
    let database = malformed_settings
        .database_url
        .as_ref()
        .expect("DATABASE_URL is set");
    let chosen = malformed_settings.direct_url.as_ref().unwrap_or(database);
    strip_prisma_query_params(chosen.expose_secret()).unwrap_err();
}
