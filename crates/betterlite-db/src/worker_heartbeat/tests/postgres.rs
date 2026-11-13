#![cfg(test)]

use std::sync::atomic::Ordering;

use clap::Parser as _;
use deadpool_postgres::{Client, Manager, Pool};
use secrecy::{ExposeSecret as _, SecretString};
use tokio_postgres::NoTls;

use betterlite_postgres_fixture::connection::PostgresFixture;

use super::support::RecordingIdentity;
use crate::database_replica::availability::Availability;
use crate::database_replica::pinned::WriterPinnedClient;
use crate::database_replica::routing::RoutingDatabase;
use crate::worker_heartbeat::{PostgresDatabase, ProxyWorkerHeartbeat, count_live_proxy_workers};

// the Prisma column is timestamp(3); the boundary tests widen it so a frozen NOW() lands exactly on the cutoff
const SCHEMA: &str = concat!(
    include_str!("../../../migrations/20260814000000_add_proxy_worker_heartbeat/migration.sql"),
    r#"ALTER TABLE "LiteLLM_ProxyWorkerHeartbeat" ALTER COLUMN "last_heartbeat_at" TYPE timestamptz USING "last_heartbeat_at" AT TIME ZONE 'UTC';"#,
);

#[derive(clap::Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

async fn database() -> (PostgresFixture, Pool) {
    let url = Environment::parse_from(["heartbeat-tests"])
        .url
        .map(SecretString::from);
    let fixture = PostgresFixture::provision_schema(url.as_ref())
        .resolve()
        .await
        .unwrap();
    let manager = Manager::new(fixture.options.url.expose_secret().parse().unwrap(), NoTls);
    let pool = Pool::builder(manager).max_size(4).build().unwrap();
    let client = pool.get().await.unwrap();
    client.batch_execute(SCHEMA).await.unwrap();
    (fixture, pool)
}

fn heartbeat(worker_id: Option<&str>, hostname: &'static str) -> ProxyWorkerHeartbeat {
    let identity = RecordingIdentity::new(hostname);
    ProxyWorkerHeartbeat::new(worker_id, &identity).unwrap()
}

#[tokio::test]
async fn postgres_clock_ignores_process_skew() {
    let (fixture, pool) = database().await;
    let mut client = pool.get().await.unwrap();
    let transaction = client.transaction().await.unwrap();
    transaction
        .batch_execute(
            r#"INSERT INTO "LiteLLM_ProxyWorkerHeartbeat" (worker_id, hostname, last_heartbeat_at)
               VALUES ('sentinel', 'host', NOW() - interval '179 seconds')"#,
        )
        .await
        .unwrap();
    let database = PostgresDatabase {
        client: transaction,
    };
    let service = heartbeat(Some("worker-a"), "host-a");
    service.beat(&database).await;
    let row = database
        .client
        .query_one(
            r#"SELECT last_heartbeat_at = NOW() FROM "LiteLLM_ProxyWorkerHeartbeat"
               WHERE worker_id = 'worker-a'"#,
            &[],
        )
        .await
        .unwrap();
    assert!(row.get::<_, bool>(0));
    assert_eq!(
        count_live_proxy_workers(WriterPinnedClient::Writer(&database)).await,
        Some(2)
    );
    database.client.commit().await.unwrap();
    drop(client);
    pool.close();
    fixture.close_async().await.unwrap();
}

#[tokio::test]
async fn postgres_conflict_keeps_hostname() {
    let (fixture, pool) = database().await;
    let client = pool.get().await.unwrap();
    client
        .execute(
            r#"INSERT INTO "LiteLLM_ProxyWorkerHeartbeat" (worker_id, hostname, last_heartbeat_at)
               VALUES ('worker-a', 'original-host', NOW() - interval '1 hour')"#,
            &[],
        )
        .await
        .unwrap();
    let database = PostgresDatabase { client };
    heartbeat(Some("worker-a"), "new-host")
        .beat(&database)
        .await;
    let row = database
        .client
        .query_one(
            r#"SELECT hostname, last_heartbeat_at > NOW() - interval '60 seconds'
               FROM "LiteLLM_ProxyWorkerHeartbeat" WHERE worker_id = 'worker-a'"#,
            &[],
        )
        .await
        .unwrap();
    assert_eq!(row.get::<_, String>(0), "original-host");
    assert!(row.get::<_, bool>(1));
    drop(database);
    pool.close();
    fixture.close_async().await.unwrap();
}

#[tokio::test]
async fn postgres_upsert_then_prune() {
    let (fixture, pool) = database().await;
    let client = pool.get().await.unwrap();
    client
        .batch_execute(
            r#"INSERT INTO "LiteLLM_ProxyWorkerHeartbeat" (worker_id, hostname, last_heartbeat_at) VALUES
                   ('stale', 'host', NOW() - interval '3601 seconds'),
                   ('kept', 'host', NOW() - interval '3599 seconds')"#,
        )
        .await
        .unwrap();
    let database = PostgresDatabase { client };
    heartbeat(Some("worker-a"), "host-a").beat(&database).await;
    let rows = database
        .client
        .query(
            r#"SELECT worker_id FROM "LiteLLM_ProxyWorkerHeartbeat" ORDER BY worker_id"#,
            &[],
        )
        .await
        .unwrap();
    let names: Vec<String> = rows.iter().map(|row| row.get(0)).collect();
    assert_eq!(names, vec!["kept", "worker-a"]);
    drop(database);
    pool.close();
    fixture.close_async().await.unwrap();
}

#[tokio::test]
async fn postgres_prune_failure_keeps_upsert() {
    let (fixture, pool) = database().await;
    let first = pool.get().await.unwrap();
    first
        .batch_execute(
            r#"CREATE FUNCTION fail_prune() RETURNS trigger LANGUAGE plpgsql AS
               $$ BEGIN RAISE EXCEPTION 'prune blocked'; END $$;
               CREATE TRIGGER fail_prune BEFORE DELETE ON "LiteLLM_ProxyWorkerHeartbeat"
               FOR EACH ROW EXECUTE FUNCTION fail_prune();
               INSERT INTO "LiteLLM_ProxyWorkerHeartbeat" (worker_id, hostname, last_heartbeat_at)
               VALUES ('stale', 'host', NOW() - interval '2 hours')"#,
        )
        .await
        .unwrap();
    let database = PostgresDatabase { client: first };
    let service = heartbeat(Some("worker-a"), "host-a");
    service.beat(&database).await;
    let observer = pool.get().await.unwrap();
    assert!(heartbeat_exists(&observer, "worker-a").await);
    assert!(heartbeat_exists(&observer, "stale").await);
    observer
        .batch_execute(
            r#"DROP TRIGGER fail_prune ON "LiteLLM_ProxyWorkerHeartbeat";
               DROP FUNCTION fail_prune()"#,
        )
        .await
        .unwrap();
    service.beat(&database).await;
    assert!(!heartbeat_exists(&observer, "stale").await);
    assert!(heartbeat_exists(&observer, "worker-a").await);
    drop(observer);
    drop(database);
    pool.close();
    fixture.close_async().await.unwrap();
}

async fn heartbeat_exists(client: &Client, worker_id: &str) -> bool {
    client
        .query_one(
            r#"SELECT EXISTS (SELECT 1 FROM "LiteLLM_ProxyWorkerHeartbeat" WHERE worker_id = $1)"#,
            &[&worker_id],
        )
        .await
        .unwrap()
        .get(0)
}

#[tokio::test]
async fn postgres_live_and_prune_strict_boundaries() {
    let (fixture, pool) = database().await;
    let mut client = pool.get().await.unwrap();
    let transaction = client.transaction().await.unwrap();
    transaction
        .batch_execute(
            r#"INSERT INTO "LiteLLM_ProxyWorkerHeartbeat" (worker_id, hostname, last_heartbeat_at) VALUES
                   ('boundary-live', 'host', NOW() - interval '179.999999 seconds'),
                   ('boundary-dead', 'host', NOW() - interval '180 seconds'),
                   ('prune-kept', 'host', NOW() - interval '3600 seconds'),
                   ('prune-dead', 'host', NOW() - interval '3600.000001 seconds')"#,
        )
        .await
        .unwrap();
    let database = PostgresDatabase {
        client: transaction,
    };
    assert_eq!(
        count_live_proxy_workers(WriterPinnedClient::Writer(&database)).await,
        Some(1)
    );
    heartbeat(Some("worker-a"), "host-a").beat(&database).await;
    let rows = database
        .client
        .query(
            r#"SELECT worker_id FROM "LiteLLM_ProxyWorkerHeartbeat" ORDER BY worker_id"#,
            &[],
        )
        .await
        .unwrap();
    let names: Vec<String> = rows.iter().map(|row| row.get(0)).collect();
    assert_eq!(
        names,
        vec!["boundary-dead", "boundary-live", "prune-kept", "worker-a"]
    );
    assert_eq!(
        count_live_proxy_workers(WriterPinnedClient::Writer(&database)).await,
        Some(2)
    );
    database.client.commit().await.unwrap();
    drop(client);
    pool.close();
    fixture.close_async().await.unwrap();
}

#[tokio::test]
async fn postgres_deregister_idempotent() {
    let (fixture, pool) = database().await;
    let client = pool.get().await.unwrap();
    let database = PostgresDatabase { client };
    let worker_a = heartbeat(Some("worker-a"), "host-a");
    worker_a.beat(&database).await;
    heartbeat(Some("worker-b"), "host-b").beat(&database).await;
    worker_a.deregister(&database).await;
    worker_a.deregister(&database).await;
    let rows = database
        .client
        .query(
            r#"SELECT worker_id FROM "LiteLLM_ProxyWorkerHeartbeat" ORDER BY worker_id"#,
            &[],
        )
        .await
        .unwrap();
    let names: Vec<String> = rows.iter().map(|row| row.get(0)).collect();
    assert_eq!(names, vec!["worker-b"]);
    drop(database);
    pool.close();
    fixture.close_async().await.unwrap();
}

#[tokio::test]
async fn postgres_count_uses_writer_not_reader() {
    let (writer_fixture, writer_pool) = database().await;
    let (reader_fixture, reader_pool) = database().await;
    let writer_client = writer_pool.get().await.unwrap();
    let reader_client = reader_pool.get().await.unwrap();
    insert_live_rows(&writer_client, 2).await;
    insert_live_rows(&reader_client, 5).await;
    let routing = RoutingDatabase {
        writer: PostgresDatabase {
            client: writer_client,
        },
        reader: PostgresDatabase {
            client: reader_client,
        },
        availability: Availability::default(),
        reader_url: None,
    };
    assert_eq!(
        count_live_proxy_workers(WriterPinnedClient::Routed(&routing)).await,
        Some(2)
    );
    routing
        .availability
        .reader_unavailable
        .store(true, Ordering::SeqCst);
    routing
        .availability
        .writer_unavailable
        .store(true, Ordering::SeqCst);
    assert_eq!(
        count_live_proxy_workers(WriterPinnedClient::Routed(&routing)).await,
        Some(2)
    );
    drop(routing);
    writer_pool.close();
    reader_pool.close();
    writer_fixture.close_async().await.unwrap();
    reader_fixture.close_async().await.unwrap();
}

async fn insert_live_rows(client: &Client, count: i32) {
    client
        .execute(
            r#"INSERT INTO "LiteLLM_ProxyWorkerHeartbeat" (worker_id, hostname, last_heartbeat_at)
               SELECT 'worker-' || value, 'host', NOW() FROM generate_series(1, $1::int) AS value"#,
            &[&count],
        )
        .await
        .unwrap();
}
