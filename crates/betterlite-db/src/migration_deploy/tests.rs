#![cfg(test)]

use super::DeployError;
use super::fixtures::{checkpoint, database};
use std::time::Duration;
use tokio::fs;
use tokio::time::{Instant, sleep};
use tokio_postgres::error::SqlState;
use uuid::Uuid;

#[tokio::test]
async fn unfinished_row_stops_deploy() {
    database(async |db| {
        db.sql("INSERT INTO _prisma_migrations (id, checksum, migration_name) VALUES ('old', 'checksum', 'unlisted')").await;
        assert!(matches!(db.run(&[("new", "INSERT INTO effects VALUES ('wrong')")]).await, Err(DeployError::Unfinished { .. })));
        assert_eq!(db.count("SELECT count(*) FROM effects").await, 0);
        assert_eq!(db.rows().await.len(), 1);
    }).await;
}

#[tokio::test]
async fn explicit_transaction_failure() {
    database(async |db| {
        let result = db
            .run(&[(
                "bad",
                "BEGIN; INSERT INTO effects VALUES ('rolled back'); SELECT 1/0; COMMIT;",
            )])
            .await;
        assert!(matches!(result, Err(DeployError::Migration { .. })));
        let rows = db.rows().await;
        assert_eq!(rows.len(), 1);
        assert!(!rows[0].get::<_, bool>("finished"));
        assert!(rows[0].get::<_, Option<String>>("logs").is_some());
        assert_eq!(db.count("SELECT count(*) FROM effects").await, 0);
    })
    .await;
}

#[tokio::test]
async fn recorded_checksum() {
    database(async |db| {
        db.run(&[("quote'", "SELECT 1;\r\n")]).await.unwrap();
        let rows = db.rows().await;
        assert_eq!(
            rows[0].get::<_, String>("checksum"),
            "d3cd5042f97738960d802ad6b3a548dfa18152215118ba18f04493bc6944b0e4"
        );
        assert_eq!(rows[0].get::<_, String>("migration_name"), "quote'");
        let id = Uuid::parse_str(&rows[0].get::<_, String>("id")).unwrap();
        assert_eq!(id.get_version_num(), 4_usize);
    })
    .await;
}

#[tokio::test]
async fn restart_skips_applied() {
    database(async |db| {
        db.sql("INSERT INTO _prisma_migrations (id, checksum, migration_name, finished_at, logs) VALUES ('original', 'original checksum', 'done', '2020-01-01', 'original logs')").await;
        db.run(&[("done", "SELECT 1/0")]).await.unwrap();
        let rows = db.rows().await;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].get::<_, String>("id"), "original");
        assert_eq!(rows[0].get::<_, String>("checksum"), "original checksum");
        assert_eq!(rows[0].get::<_, String>("logs"), "original logs");
        assert_eq!(rows[0].get::<_, i32>("applied_steps_count"), 0_i32);
        assert_eq!(db.count("SELECT count(*) FROM _prisma_migrations WHERE finished_at = '2020-01-01'").await, 1);
    }).await;
}

#[tokio::test]
async fn postgres_pinned_rows() {
    database(async |db| {
        let expected_columns = db.columns().await;
        db.sql("DROP TABLE _prisma_migrations").await;
        db.run(&[("first", "INSERT INTO effects VALUES ('a;quoted');")])
            .await
            .unwrap();
        let rows = db.rows().await;
        assert_eq!(rows.len(), 1);
        assert!(rows[0].get::<_, bool>("started"));
        assert!(rows[0].get::<_, bool>("finished"));
        assert!(!rows[0].get::<_, bool>("rolled_back"));
        assert_eq!(rows[0].get::<_, Option<String>>("logs"), None);
        assert_eq!(rows[0].get::<_, i32>("applied_steps_count"), 1_i32);
        assert_eq!(
            db.count("SELECT count(*) FROM effects WHERE value = 'a;quoted'")
                .await,
            1
        );
        assert_eq!(db.columns().await, expected_columns);
    })
    .await;
}

#[tokio::test]
async fn rollback_rows_allow_new_attempt() {
    database(async |db| {
        db.sql("INSERT INTO _prisma_migrations (id, checksum, migration_name, rolled_back_at, logs) VALUES ('old', 'old', 'retry', now(), 'failed')").await;
        db.run(&[("retry", "SELECT 1")]).await.unwrap();
        let rows = db.rows().await;
        assert_eq!(rows.len(), 2);
        assert!(rows[0].get::<_, bool>("rolled_back"));
        assert_eq!(rows[0].get::<_, String>("logs"), "failed");
        assert!(rows[1].get::<_, bool>("finished"));
    }).await;
}

#[tokio::test]
async fn failed_ledger_unfinished() {
    database(async |db| {
        db.run(&[("failed", "SELECT 1/0")]).await.unwrap_err();
        let rows = db.rows().await;
        assert_eq!(rows[0].get::<_, i32>("applied_steps_count"), 0_i32);
        assert!(!rows[0].get::<_, bool>("finished"));
        assert!(!rows[0].get::<_, bool>("rolled_back"));
        assert!(!rows[0].get::<_, String>("logs").trim().is_empty());
        assert!(matches!(
            db.run(&[("next", "SELECT 1")]).await,
            Err(DeployError::Unfinished { .. })
        ));
    })
    .await;
}

#[tokio::test]
async fn postgres_failure_effects() {
    database(async |db| {
        db.run(&[("partial", "BEGIN; INSERT INTO effects VALUES ('committed'); COMMIT; INSERT INTO effects VALUES ('rollback'); SELECT 1/0;")]).await.unwrap_err();
        assert_eq!(db.count("SELECT count(*) FROM effects WHERE value='committed'").await, 1);
        assert_eq!(db.count("SELECT count(*) FROM effects WHERE value='rollback'").await, 0);
        let rows = db.rows().await;
        assert_eq!(rows[0].get::<_, i32>("applied_steps_count"), 0_i32);
        assert!(!rows[0].get::<_, bool>("finished"));
        assert!(rows[0].get::<_, String>("logs").contains("division by zero"));
    }).await;
}

#[tokio::test]
async fn earlier_migrations_survive() {
    database(async |db| {
        db.run(&[
            ("first", "INSERT INTO effects VALUES ('first')"),
            (
                "second",
                "INSERT INTO effects VALUES ('second'); SELECT 1/0",
            ),
            ("third", "INSERT INTO effects VALUES ('third')"),
        ])
        .await
        .unwrap_err();
        assert_eq!(db.count("SELECT count(*) FROM effects").await, 1);
        let rows = db.rows().await;
        assert_eq!(rows.len(), 2);
        assert!(rows[0].get::<_, bool>("finished"));
        assert!(!rows[1].get::<_, bool>("finished"));
    })
    .await;
}

#[tokio::test]
async fn concurrent_index_outside_transaction() {
    database(async |db| {
        db.sql("CREATE TABLE \"LiteLLM_HealthCheckTable\" (model_id text, model_name text, checked_at timestamptz)").await;
        db.run(&[("20260415120000_health_check_latest_per_model_index", include_str!("fixtures/health.sql"))]).await.unwrap();
        assert_eq!(db.count("SELECT count(*) FROM pg_index WHERE indrelid='\"LiteLLM_HealthCheckTable\"'::regclass AND indisvalid").await, 1);
        let rows = db.rows().await;
        assert_eq!(rows[0].get::<_, i32>("applied_steps_count"), 1_i32);
        assert!(rows[0].get::<_, bool>("finished"));
    }).await;
}

use super::catalog::{CatalogError, MigrationCatalog, migration_files};
use super::fixtures::CatalogDirectory;

#[tokio::test]
async fn catalog_lexical_order() {
    let root = CatalogDirectory::create().await;
    root.write("migrations/2/migration.sql", b"second").await;
    root.write("migrations/10/migration.sql", b"first").await;
    let catalog = migration_files(&root.0).await.unwrap();
    assert_eq!(
        catalog
            .iter()
            .map(|file| file.name.as_str())
            .collect::<Vec<_>>(),
        ["10", "2"]
    );
}

#[tokio::test]
async fn catalog_one_directory_level() {
    let root = CatalogDirectory::create().await;
    root.write("migrations/direct/migration.sql", b"direct")
        .await;
    root.write("migrations/direct/nested/migration.sql", b"nested")
        .await;
    root.write("migrations/migration.sql", b"top").await;
    root.write("migrations/another/other.sql", b"other").await;
    let catalog = migration_files(&root.0).await.unwrap();
    assert_eq!(catalog.len(), 1);
    assert_eq!(catalog[0].name, "direct");
}

#[tokio::test]
async fn catalog_empty() {
    let root = CatalogDirectory::create().await;
    assert!(migration_files(&root.0).await.unwrap().is_empty());
    assert!(
        migration_files(&root.0.join("absent"))
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn checksum_exact_bytes() {
    let root = CatalogDirectory::create().await;
    root.write("migrations/raw/migration.sql", b"SELECT 1;\r\n")
        .await;
    let catalog = migration_files(&root.0).await.unwrap();
    assert_eq!(catalog[0].sql, b"SELECT 1;\r\n");
    assert_eq!(
        catalog[0].checksum,
        "d3cd5042f97738960d802ad6b3a548dfa18152215118ba18f04493bc6944b0e4"
    );
}

#[tokio::test]
async fn catalog_io_error() {
    let root = CatalogDirectory::create().await;
    fs::create_dir_all(root.0.join("migrations/unreadable/migration.sql"))
        .await
        .unwrap();
    migration_files(&root.0).await.unwrap_err();
}

#[tokio::test]
async fn started_row_before_sql() {
    database(async |db| {
        let blocker = db.pool.get().await.unwrap();
        blocker
            .batch_execute("SELECT pg_advisory_lock(874123::bigint)")
            .await
            .unwrap();
        let mut deploy =
            Box::pin(db.run(&[("blocked", "SELECT pg_advisory_lock(874123::bigint)")]));
        checkpoint(
            deploy.as_mut(),
            db.wait_for("SELECT count(*) FROM _prisma_migrations"),
        )
        .await;
        let rows = db.rows().await;
        assert!(rows[0].get::<_, bool>("started"));
        assert_eq!(rows[0].get::<_, i32>("applied_steps_count"), 0_i32);
        assert_eq!(rows[0].get::<_, Option<String>>("logs"), None);
        assert!(!rows[0].get::<_, bool>("finished"));
        blocker
            .batch_execute("SELECT pg_advisory_unlock(874123::bigint)")
            .await
            .unwrap();
        deploy.await.unwrap();
    })
    .await;
}

#[tokio::test]
async fn step_before_finish() {
    database(async |db| {
        db.sql("CREATE FUNCTION interrupt_finish() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF NEW.finished_at IS NOT NULL THEN RAISE EXCEPTION 'interrupted finish'; END IF; RETURN NEW; END $$; CREATE TRIGGER interrupt BEFORE UPDATE ON _prisma_migrations FOR EACH ROW EXECUTE FUNCTION interrupt_finish()").await;
        db.run(&[("checkpoint", "INSERT INTO effects VALUES ('durable')")]).await.unwrap_err();
        let rows = db.rows().await;
        assert_eq!(rows[0].get::<_, i32>("applied_steps_count"), 1_i32);
        assert!(!rows[0].get::<_, bool>("finished"));
        assert!(!rows[0].get::<_, bool>("rolled_back"));
        assert_eq!(rows[0].get::<_, Option<String>>("logs"), None);
        assert_eq!(rows[0].get::<_, String>("checksum"), "cff14ca3a9a0a08eb095dc3de087e112817819db5a37e60d121b077b37d465d1");
        assert_eq!(db.count("SELECT count(*) FROM effects").await, 1);
        assert!(matches!(db.run(&[("checkpoint", "SELECT 1")]).await, Err(DeployError::Unfinished { .. })));
    }).await;
}

#[tokio::test]
async fn lock_released_on_cancel() {
    database(async |db| {
        let blocker = db.pool.get().await.unwrap();
        blocker.batch_execute("SELECT pg_advisory_lock(874124::bigint)").await.unwrap();
        let mut deploy = Box::pin(db.run(&[("cancel", "SELECT pg_advisory_lock(874124::bigint)")]));
        checkpoint(deploy.as_mut(), db.wait_for("SELECT count(*) FROM _prisma_migrations")).await;
        drop(deploy);
        blocker.batch_execute("SELECT pg_advisory_unlock(874124::bigint)").await.unwrap();
        db.wait_for("SELECT CASE WHEN EXISTS (SELECT 1 FROM pg_locks WHERE locktype='advisory' AND objid=72707369 AND granted) THEN 0::bigint ELSE 1::bigint END").await;
        let rows = db.rows().await;
        assert!(!rows[0].get::<_, bool>("finished"));
        assert_eq!(rows[0].get::<_, i32>("applied_steps_count"), 0_i32);
    }).await;
}

#[tokio::test]
async fn prisma_lock_serializes_cutover() {
    database(async |db| {
        db.lock_conflict("SELECT pg_advisory_lock(72707369::bigint)")
            .await;
    })
    .await;
}

#[tokio::test]
async fn coordinator_transaction_lock_conflict() {
    database(async |db| {
        let blocker = db.pool.get().await.unwrap();
        blocker
            .batch_execute("BEGIN; SELECT pg_advisory_xact_lock(72707369::bigint)")
            .await
            .unwrap();
        let client = db.pool.get().await.unwrap();
        let service = db.service(&[("locked", "SELECT 1")]);
        let mut deployment = Box::pin(service.run(
            client,
            Instant::now().checked_add(Duration::from_secs(10)).unwrap(),
        ));
        checkpoint(deployment.as_mut(), sleep(Duration::from_millis(100))).await;
        assert!(db.rows().await.is_empty());
        drop(deployment);
        blocker
            .batch_execute("ROLLBACK; SELECT pg_advisory_unlock_all()")
            .await
            .unwrap();
        db.wait_for(
            "SELECT CASE WHEN EXISTS (SELECT 1 FROM pg_locks WHERE locktype='advisory' AND objid=72707369) THEN 0::bigint ELSE 1::bigint END",
        )
        .await;
        db.run(&[("locked", "SELECT 1")]).await.unwrap();
    })
    .await;
}

#[tokio::test]
async fn custom_schema_quoted() {
    database(async |db| {
        let quoted = format!("\"{}\"", db.schema.replace('"', "\"\""));
        let custom = format!("{}\"';--", db.schema);
        let custom_quote = format!("\"{}\"", custom.replace('"', "\"\""));
        db.sql(&format!("ALTER SCHEMA {quoted} RENAME TO {custom_quote}"))
            .await;
        let mut service = db.service(&[("quote'", "CREATE TABLE created(value text)")]);
        service.schema = custom;
        let outcome = service.run(db.pool.get().await.unwrap(), Instant::now().checked_add(Duration::from_secs(10)).unwrap()).await;
        db.sql(&format!("ALTER SCHEMA {custom_quote} RENAME TO {quoted}"))
            .await;
        outcome.unwrap();
        assert_eq!(db.rows().await.len(), 1);
        assert_eq!(db.count("SELECT count(*) FROM information_schema.tables WHERE table_schema=current_schema() AND table_name='created'").await, 1);
    })
    .await;
}

#[tokio::test]
async fn invalid_sql_logs_failure() {
    database(async |db| {
        let mut service = db.service(&[("invalid", "SELECT 1")]);
        service.catalog[0].sql = vec![255];
        let result = service
            .run(
                db.pool.get().await.unwrap(),
                Instant::now().checked_add(Duration::from_secs(10)).unwrap(),
            )
            .await;
        assert!(matches!(result, Err(DeployError::InvalidSql { .. })));
        let rows = db.rows().await;
        assert!(!rows[0].get::<_, bool>("finished"));
        assert_eq!(rows[0].get::<_, i32>("applied_steps_count"), 0_i32);
        assert!(!rows[0].get::<_, String>("logs").is_empty());
    })
    .await;
}

#[tokio::test]
async fn deadline_releases_session() {
    database(async |db| {
        let service = db.service(&[("timeout", "SELECT pg_sleep(10)")]);
        let client = db.pool.get().await.unwrap();
        let pid: i32 = client.query_one("SELECT pg_backend_pid()", &[]).await.unwrap().get(0);
        let deadline = Instant::now().checked_add(Duration::from_secs(1)).unwrap();
        service.run(client, deadline).await.unwrap_err();
        db.wait_for(&format!("SELECT CASE WHEN EXISTS (SELECT 1 FROM pg_stat_activity WHERE pid={pid}) THEN 0::bigint ELSE 1::bigint END")).await;
        let rows = db.rows().await;
        assert_eq!(rows.len(), 1);
        assert!(!rows[0].get::<_, bool>("finished"));
        assert_eq!(rows[0].get::<_, i32>("applied_steps_count"), 0_i32);
    }).await;
}

#[tokio::test]
async fn catalog_types_are_native() {
    let root = CatalogDirectory::create().await;
    root.write("migrations/one/migration.sql", b"SELECT 1;\r\n")
        .await;
    let catalog: MigrationCatalog = migration_files(&root.0).await.unwrap();
    assert_eq!(catalog.len(), 1);
    assert_eq!(catalog[0].name, "one");
    assert_eq!(catalog[0].sql, b"SELECT 1;\r\n");
    assert_eq!(
        catalog[0].checksum,
        "d3cd5042f97738960d802ad6b3a548dfa18152215118ba18f04493bc6944b0e4"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn catalog_invalid_name() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt as _;

    let root = CatalogDirectory::create().await;
    let directory = root
        .0
        .join("migrations")
        .join(OsString::from_vec(vec![255]));
    fs::create_dir(&directory).await.unwrap();
    fs::write(directory.join("migration.sql"), b"SELECT 1")
        .await
        .unwrap();
    let error = migration_files(&root.0).await.unwrap_err();
    assert!(
        matches!(error, CatalogError::Name { path } if path == directory.join("migration.sql"))
    );
}

#[rstest::rstest]
#[case::sql(b"SELECT 1/0", "division by zero")]
#[case::invalid_utf8(&[255], "invalid utf-8")]
#[tokio::test]
async fn log_failure_keeps_cause(#[case] sql: &'static [u8], #[case] message: &'static str) {
    use std::error::Error as _;

    database(async move |db| {
        db.sql("CREATE FUNCTION reject_logs() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'logs rejected'; END $$; CREATE TRIGGER reject BEFORE UPDATE ON _prisma_migrations FOR EACH ROW EXECUTE FUNCTION reject_logs()").await;
        let mut service = db.service(&[("original", "")]);
        service.catalog[0].sql = sql.to_vec();
        let error = service.run(db.pool.get().await.unwrap(), Instant::now().checked_add(Duration::from_secs(10)).unwrap()).await.unwrap_err();
        assert!(error.to_string().contains("original"), "{error:?}");
        let source = error.source().unwrap();
        let detail = source.downcast_ref::<tokio_postgres::Error>().and_then(tokio_postgres::Error::as_db_error).map_or_else(|| source.to_string(), |database_error| database_error.message().to_owned());
        assert!(detail.contains(message), "{error:?}");
        let (DeployError::MigrationLog { ledger, .. } | DeployError::InvalidSqlLog { ledger, .. }) = error else {
            panic!("expected both errors, got {error:?}");
        };
        assert_eq!(ledger.as_db_error().unwrap().message(), "logs rejected");
        let rows = db.rows().await;
        assert!(!rows[0].get::<_, bool>("finished"));
        assert_eq!(rows[0].get::<_, Option<String>>("logs"), None);
    }).await;
}

#[tokio::test]
async fn rollback_failure_keeps_cause() {
    database(async |db| {
        let error = db
            .run(&[(
                "disconnected",
                "SELECT pg_terminate_backend(pg_backend_pid())",
            )])
            .await
            .unwrap_err();
        assert!(error.to_string().contains("disconnected"), "{error:?}");
        let DeployError::MigrationLog {
            name,
            source,
            ledger,
        } = error
        else {
            panic!("expected migration and rollback errors");
        };
        assert_eq!(name, "disconnected");
        assert_eq!(source.code(), Some(&SqlState::ADMIN_SHUTDOWN));
        assert!(ledger.is_closed());
        let rows = db.rows().await;
        assert!(!rows[0].get::<_, bool>("finished"));
        assert_eq!(rows[0].get::<_, Option<String>>("logs"), None);
    })
    .await;
}
