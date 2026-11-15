#![cfg(test)]

use clap::Parser as _;
use deadpool_postgres::{Manager, Pool};
use secrecy::{ExposeSecret as _, SecretString};
use tokio_postgres::NoTls;

use betterlite_postgres_fixture::connection::PostgresFixture;

// every shadow migration, so a test sees the tables the proxy queries
const SCHEMA: &str = concat!(
    include_str!("../../../migrations/20260811172448_add_shadow_eval/migration.sql"),
    include_str!("../../../migrations/20260813180408_add_shadow_eval_direction/migration.sql"),
    include_str!("../../../migrations/20260817000000_shadow_eval_multi_key/migration.sql"),
    include_str!("../../../migrations/20260818224500_add_shadow_eval_stopped_by/migration.sql"),
    include_str!("../../../migrations/20260819000000_shadow_eval_max_budget/migration.sql"),
    include_str!("../../../migrations/20260828000000_shadow_eval_cost_comparison/migration.sql"),
    include_str!("../../../migrations/20260831000000_shadow_eval_typed_targets/migration.sql"),
    include_str!("../../../migrations/20260901000000_shadow_eval_multi_router/migration.sql"),
    include_str!("../../../migrations/20260904000000_shadow_eval_model_scope/migration.sql"),
);

#[derive(clap::Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

pub(crate) async fn database() -> (PostgresFixture, Pool) {
    let url = Environment::parse_from(["shadow-tables-tests"])
        .url
        .map(SecretString::from);
    let fixture = PostgresFixture::provision_schema(url.as_ref())
        .resolve()
        .await
        .unwrap();
    let manager = Manager::new(fixture.options.url.expose_secret().parse().unwrap(), NoTls);
    let pool = Pool::builder(manager).max_size(2).build().unwrap();
    let client = pool.get().await.unwrap();
    client.batch_execute(SCHEMA).await.unwrap();
    (fixture, pool)
}
