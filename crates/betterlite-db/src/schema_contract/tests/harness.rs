use super::catalog::Attribute;
use super::history::litellm_statements;
use crate::migration_ledger::baseline::packaged_statements;
use betterlite_postgres_fixture::connection::PostgresFixture;
use clap::Parser;
use deadpool_postgres::{Client, Manager, Pool};
use futures_util::FutureExt as _;
use secrecy::SecretString;
use std::future::Future;
use std::panic::{AssertUnwindSafe, resume_unwind};
use std::time::Duration;
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
        for statements in [litellm_statements(), packaged_statements().unwrap()] {
            let fixture = PostgresFixture::provision_with_statements(url.as_ref(), statements)
                .resolve()
                .await
                .unwrap();
            let pool = Pool::builder(Manager::new(fixture.options.config.clone(), NoTls))
                .max_size(1)
                .build()
                .unwrap();
            resources.push((fixture, pool));
        }
        let reference = resources[0].1.get().await.unwrap();
        let candidate = resources[1].1.get().await.unwrap();
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
