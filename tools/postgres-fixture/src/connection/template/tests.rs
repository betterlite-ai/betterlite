#![cfg(test)]

use super::template_name;
use crate::connection::{PostgresFixture, RandomDatabaseName};
use crate::seed::FixtureSeed;
use crate::seed_sql::schema_statements;
use crate::tests::{connect_client, postgres_url};

#[tokio::test]
async fn concurrent_fixtures_clone_one_published_template() {
    let admin_url = postgres_url();
    let seed = FixtureSeed::default();
    let mut first_task = PostgresFixture::provision_with_name_generator(
        Some(&admin_url),
        &seed,
        &mut RandomDatabaseName,
    );
    let mut second_task = PostgresFixture::provision_with_name_generator(
        Some(&admin_url),
        &seed,
        &mut RandomDatabaseName,
    );
    let first = first_task.resolve().await.unwrap();
    let second = second_task.resolve().await.unwrap();
    let (name, _) = template_name(&schema_statements(&seed).await.unwrap());
    let (admin, _driver) = connect_client(&admin_url).await;
    let row = admin
        .query_one(
            "SELECT datistemplate, datallowconn FROM pg_database WHERE datname = $1",
            &[&name],
        )
        .await
        .unwrap();
    assert!(row.get::<_, bool>(0));
    assert!(!row.get::<_, bool>(1));
    first.close_async().await.unwrap();
    second.close_async().await.unwrap();
}
