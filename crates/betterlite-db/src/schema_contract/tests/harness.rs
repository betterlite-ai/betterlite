use super::catalog::Attribute;
use crate::{
    schema_agent_identity, schema_baseline, schema_budget_windows, schema_daily_usage,
    schema_identity_history, schema_managed_resources, schema_policy_resources,
    schema_routing_state,
};
use betterlite_postgres_fixture::connection::{PostgresFixture, RandomDatabaseName};
use betterlite_postgres_fixture::seed::FixtureSeed;
use clap::Parser;
use deadpool_postgres::{Client, Manager, Pool};
use futures_util::FutureExt as _;
use secrecy::SecretString;
use std::future::Future;
use std::panic::{AssertUnwindSafe, resume_unwind};
use std::str;
use std::time::Duration;
use tokio::fs as async_fs;
use tokio_postgres::NoTls;

use super::comparison::Change;
use super::{Fact, ObjectKind, SchemaContract};

const LIMIT: Duration = Duration::from_secs(30);

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

async fn harness<F, Fut>(body: F)
where
    F: FnOnce([Client; 2]) -> Fut,
    Fut: Future<Output = ()>,
{
    let url = Environment::parse_from(["schema-contract"])
        .url
        .map(SecretString::from);
    let mut resources = Vec::new();
    let outcome = AssertUnwindSafe(async {
        for _ in 0..2_usize {
            let mut task = PostgresFixture::provision_with_name_generator(
                url.as_ref(),
                &FixtureSeed::default(),
                &mut RandomDatabaseName,
            );
            let fixture = task.resolve().await.unwrap();
            let pool = Pool::builder(Manager::new(fixture.options.config.clone(), NoTls))
                .max_size(1)
                .build()
                .unwrap();
            resources.push((fixture, pool));
        }
        let reference = resources[0].1.get().await.unwrap();
        let candidate = resources[1].1.get().await.unwrap();
        apply_histories(&reference, &candidate).await;
        body([reference, candidate]).await;
    })
    .catch_unwind()
    .await;
    for (fixture, pool) in resources {
        pool.close();
        fixture.close_async().await.unwrap();
    }
    if let Err(payload) = outcome {
        resume_unwind(payload);
    }
}

async fn apply_histories(reference: &Client, candidate: &Client) {
    let manifest = include_str!("../fixtures/sql.sha256");
    assert_eq!(
        hex::encode(Sha256::digest(manifest)),
        "c2a1e005f73eda0bb5f5496f90299688872f1e56e71f749b52d81009eae0d5ee"
    );
    for line in manifest.lines() {
        let (digest, file) = line.split_once("  ").unwrap();
        let sql = async_fs::read(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("src/schema_contract/fixtures/sql")
                .join(file),
        )
        .await
        .unwrap();
        assert_eq!(hex::encode(Sha256::digest(&sql)), digest);
        reference
            .batch_execute(str::from_utf8(&sql).unwrap())
            .await
            .unwrap();
    }
    apply_candidate(candidate).await;
}

async fn apply_candidate(candidate: &Client) {
    candidate
        .batch_execute("DROP SCHEMA public CASCADE; CREATE SCHEMA public")
        .await
        .unwrap();
    for catalog in [
        schema_baseline::MIGRATION_CATALOG,
        schema_daily_usage::MIGRATION_CATALOG,
        schema_managed_resources::MIGRATION_CATALOG,
        schema_identity_history::MIGRATION_CATALOG,
        schema_policy_resources::MIGRATION_CATALOG,
        schema_routing_state::MIGRATION_CATALOG,
        schema_budget_windows::MIGRATION_CATALOG,
        schema_agent_identity::MIGRATION_CATALOG,
    ] {
        for migration in catalog.files {
            candidate
                .batch_execute(str::from_utf8(migration.bytes).unwrap())
                .await
                .unwrap();
        }
    }
}

async fn snapshot(client: &Client) -> SchemaContract {
    SchemaContract::read(&***client, "public", LIMIT)
        .await
        .unwrap()
}

fn value<'contract>(
    contract: &'contract SchemaContract,
    kind: ObjectKind,
    relation: &str,
    member: &str,
    attribute: &str,
) -> Option<&'contract str> {
    contract.facts[&Fact {
        kind,
        relation: relation.to_owned(),
        member: member.to_owned(),
        attribute: attribute.parse().unwrap(),
    }]
        .as_deref()
}
