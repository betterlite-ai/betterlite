#![cfg(test)]

use clap::Parser as _;
use deadpool_postgres::{Manager, Pool};
use secrecy::{ExposeSecret as _, SecretString};
use tokio_postgres::NoTls;

use betterlite_postgres_fixture::connection::PostgresFixture;

use crate::migration_ledger::baseline::{PACKAGED_CATALOGS, catalog_statements};

#[derive(clap::Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

pub(crate) async fn database() -> (PostgresFixture, Pool) {
    let url = Environment::parse_from(["shadow-tables-tests"])
        .url
        .map(SecretString::from);
    let fixture = PostgresFixture::provision_with_statements(
        url.as_ref(),
        catalog_statements(&PACKAGED_CATALOGS).unwrap(),
    )
    .resolve()
    .await
    .unwrap();
    let manager = Manager::new(fixture.options.url.expose_secret().parse().unwrap(), NoTls);
    let pool = Pool::builder(manager).max_size(2).build().unwrap();
    (fixture, pool)
}
