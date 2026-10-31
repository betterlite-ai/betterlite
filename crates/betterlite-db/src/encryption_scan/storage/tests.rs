#![cfg(test)]

use super::scan_covered_tables;
use crate::encryption_scan::ScanError;
use crate::encryption_scan::report::MigrationReport;
use crate::encryption_scan::tests::cipher;
use betterlite_postgres_fixture::connection::PostgresFixture;
use clap::Parser;
use deadpool_postgres::{Client, Manager, Pool};
use secrecy::{ExposeSecret as _, SecretString};
use std::time::Duration;
use tokio_postgres::NoTls;

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

async fn database() -> (PostgresFixture, Pool, Client) {
    let url = Environment::parse_from(["scan-tests"])
        .url
        .map(SecretString::from);
    let fixture = PostgresFixture::provision_schema(url.as_ref())
        .resolve()
        .await
        .unwrap();
    let manager = Manager::new(fixture.options.url.expose_secret().parse().unwrap(), NoTls);
    let pool = Pool::builder(manager).max_size(1).build().unwrap();
    let client = pool.get().await.unwrap();
    client
        .batch_execute(include_str!("schema.sql"))
        .await
        .unwrap();
    (fixture, pool, client)
}

#[tokio::test]
async fn scans_every_column_without_writes() {
    let (fixture, pool, mut client) = database().await;
    client
        .batch_execute(include_str!("fixtures.sql"))
        .await
        .unwrap();
    let before = snapshot(&client).await;
    let transaction = client
        .build_transaction()
        .read_only(true)
        .start()
        .await
        .unwrap();
    let report = scan_covered_tables(&*transaction, &cipher(), Duration::from_secs(10))
        .await
        .unwrap();
    assert_counts(&report);
    transaction.commit().await.unwrap();
    assert_eq!(snapshot(&client).await, before);
    drop(client);
    pool.close();
    fixture.close_async().await.unwrap();
}

async fn snapshot(client: &Client) -> Vec<String> {
    let tables = [
        "LiteLLM_ProxyModelTable",
        "LiteLLM_CredentialsTable",
        "LiteLLM_MCPServerTable",
        "LiteLLM_MCPUserCredentials",
        "LiteLLM_MCPUserEnvVars",
        "LiteLLM_Config",
    ];
    let mut values = Vec::new();
    for table in tables {
        let sql = format!(
            "SELECT row_to_json(rows)::text FROM \"{table}\" rows ORDER BY row_to_json(rows)::text"
        );
        let rows = client.query(&sql, &[]).await.unwrap();
        values.extend(rows.iter().map(|row| row.get::<_, String>(0)));
    }
    values
}

#[tokio::test]
async fn unavailable_tables_do_not_stop_later_scans() {
    let (fixture, pool, client) = database().await;
    client.batch_execute(r#"
        DROP TABLE "LiteLLM_ProxyModelTable";
        DROP TABLE "LiteLLM_CredentialsTable";
        CREATE VIEW "LiteLLM_CredentialsTable" AS SELECT (1 / 0)::text AS credential_values;
        INSERT INTO "LiteLLM_Config" (param_name, param_value) VALUES ('environment_variables', '{"key":"v2:gcm:last"}');
    "#).await.unwrap();
    let report = scan_covered_tables(&**client, &cipher(), Duration::from_secs(10))
        .await
        .unwrap();
    assert_eq!(report.locations.len(), 6);
    assert!(
        report
            .locations
            .iter()
            .take(5)
            .all(|location| location.scanned == 0)
    );
    assert_eq!(report.locations.last().unwrap().already_v2, 1);
    drop(client);
    pool.close();
    fixture.close_async().await.unwrap();
}

#[tokio::test]
async fn unavailable_table_recovers_inside_a_transaction() {
    let (fixture, pool, mut client) = database().await;
    client
        .batch_execute(include_str!("fixtures.sql"))
        .await
        .unwrap();
    client
        .batch_execute(r#"DROP TABLE "LiteLLM_ProxyModelTable";"#)
        .await
        .unwrap();
    let transaction = client
        .build_transaction()
        .read_only(true)
        .start()
        .await
        .unwrap();
    let report = scan_covered_tables(&*transaction, &cipher(), Duration::from_secs(10))
        .await
        .unwrap();
    transaction.commit().await.unwrap();
    assert_eq!(report.locations[0].scanned, 0);
    assert_eq!(
        (
            report.locations[1].scanned,
            report.locations[1].already_v2,
            report.locations[1].legacy
        ),
        (2, 1, 1)
    );
    drop(client);
    pool.close();
    fixture.close_async().await.unwrap();
}

#[tokio::test]
async fn malformed_config_is_empty_and_json_text_is_decoded() {
    let (fixture, pool, client) = database().await;
    client.batch_execute(r#"
        INSERT INTO "LiteLLM_Config" (param_name, param_value) VALUES ('environment_variables', '"{broken"');
        INSERT INTO "LiteLLM_ProxyModelTable" (model_id, model_name, litellm_params, created_by, updated_by)
        VALUES ('one', 'test', '"{\"key\":\"v2:gcm:nested\"}"', 'test', 'test');
    "#).await.unwrap();
    let report = scan_covered_tables(&**client, &cipher(), Duration::from_secs(10))
        .await
        .unwrap();
    assert_eq!(report.locations.first().unwrap().already_v2, 1);
    assert_eq!(report.locations.last().unwrap().scanned, 0);
    drop(client);
    pool.close();
    fixture.close_async().await.unwrap();
}

#[tokio::test]
async fn scan_deadline_is_enforced() {
    let (fixture, pool, client) = database().await;
    client.batch_execute(r#"DROP TABLE "LiteLLM_ProxyModelTable"; CREATE VIEW "LiteLLM_ProxyModelTable" AS SELECT '{}'::jsonb AS litellm_params FROM pg_sleep(0.1);"#).await.unwrap();
    let result = scan_covered_tables(&**client, &cipher(), Duration::from_millis(10)).await;
    assert!(matches!(result, Err(ScanError::Timeout(_))));
    drop(client);
    pool.close();
    fixture.close_async().await.unwrap();
}

fn assert_counts(report: &MigrationReport) {
    let counts: Vec<_> = report
        .locations
        .iter()
        .map(|location| {
            (
                location.location.as_str(),
                location.scanned,
                location.already_v2,
                location.plaintext,
                location.undecryptable,
                location.legacy,
            )
        })
        .collect();
    assert_eq!(
        counts,
        vec![
            ("model_table", 2, 1, 1, 0, 0),
            ("credentials", 2, 1, 0, 0, 1),
            ("mcp_server", 5, 2, 0, 1, 2),
            ("mcp_user_credentials", 2, 1, 1, 0, 0),
            ("mcp_user_env_vars", 1, 1, 0, 0, 0),
            ("config_environment_variables", 1, 1, 0, 0, 0),
        ]
    );
}
