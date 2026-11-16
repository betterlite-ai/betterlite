use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use betterlite_db::migration_deploy::catalog::Migration;
use betterlite_db::migration_recovery::RecoveryError;
use tokio::sync::Notify;
use tokio::task::JoinSet;

use super::postgres::{
    NoJitter, SchemaDatabase, catalog_request_log_indexes, ledger_rows, migration,
    present_request_log_indexes, row_count, scalar, table_exists, with_schema,
};
use crate::sections::database_startup::job::{
    BuildFuture, IndexBuild, JobOutcome, RequestLogIndexBuild, SetupFailure, classify,
    run_migration_job,
};
use crate::sections::database_startup::packaged;
use crate::sections::database_startup::startup::{
    StartupSettings, StartupSwitch, build_request_log_indexes, setup_database,
};

// The signals a test reads while the job's build blocks: start, release and completion.
#[derive(Default)]
struct BuildSignals {
    started: Arc<Notify>,
    release: Arc<Notify>,
    finished: Arc<AtomicBool>,
}

// The migration job's build port: it signals when it starts, then blocks until the test releases it.
struct JobBuild {
    signals: BuildSignals,
    answer: bool,
    wait: bool,
}

impl JobBuild {
    fn immediate(answer: bool) -> Self {
        Self {
            signals: BuildSignals::default(),
            answer,
            wait: false,
        }
    }

    fn waiting() -> Self {
        Self {
            wait: true,
            ..Self::immediate(true)
        }
    }
}

impl IndexBuild for JobBuild {
    fn build(&self) -> BuildFuture<'_> {
        self.signals.started.notify_one();
        let release = Arc::clone(&self.signals.release);
        let finished = Arc::clone(&self.signals.finished);
        let answer = self.answer;
        let wait = self.wait;
        Box::pin(async move {
            if wait {
                release.notified().await;
            }
            finished.store(true, Ordering::SeqCst);
            answer
        })
    }
}

#[tokio::test]
async fn db_push_is_refused_before_connecting() {
    with_schema(|database| async move {
        let push = StartupSettings {
            use_prisma_db_push: StartupSwitch(true),
            ..database.settings()
        };
        let error = setup_database(&push, &[], packaged::SCHEMA_PRISMA, &NoJitter)
            .await
            .expect_err("a db push request is refused");
        assert!(matches!(error, RecoveryError::DbPushRequested), "{error}");

        let no_url = StartupSettings {
            database_url: None,
            direct_url: None,
            ..database.settings()
        };
        let missing_url = setup_database(&no_url, &[], packaged::SCHEMA_PRISMA, &NoJitter)
            .await
            .expect_err("a migration without DATABASE_URL is refused");
        assert!(
            matches!(missing_url, RecoveryError::DatabaseUrlRequired),
            "{missing_url}"
        );
    })
    .await;
}

#[tokio::test]
async fn existing_schema_no_rewrite() {
    with_schema(|database| async move {
        let settings = database.settings();
        let catalog = [migration(
            "20260101000000_b051_existing",
            "CREATE TABLE \"B051_Existing\" (id integer); \
             INSERT INTO \"B051_Existing\" (id) VALUES (7);",
        )];
        setup_database(&settings, &catalog, packaged::SCHEMA_PRISMA, &NoJitter)
            .await
            .expect("the first startup applies the migration");
        let first = ledger_rows(&database).await;
        assert_eq!(first.len(), 1);
        assert!(first[0].finished);
        assert_eq!(row_count(&database, "B051_Existing").await, 1);

        // a startup against a schema that is already current applies nothing and rewrites nothing
        setup_database(&settings, &catalog, packaged::SCHEMA_PRISMA, &NoJitter)
            .await
            .expect("the second startup finds the schema current");
        let second = ledger_rows(&database).await;
        assert_eq!(second.len(), 1);
        assert_eq!(second[0].checksum, first[0].checksum);
        assert!(second[0].finished);
        assert!(!second[0].rolled_back);
        assert_eq!(row_count(&database, "B051_Existing").await, 1);
    })
    .await;
}

#[tokio::test]
async fn migration_job_waits() {
    with_schema(|database| async move {
        let build = JobBuild::waiting();
        let started = Arc::clone(&build.signals.started);
        let finished = Arc::clone(&build.signals.finished);
        let release = Arc::clone(&build.signals.release);
        let catalog = [migration(
            "20260101000000_b051_job",
            "CREATE TABLE \"B051_Job\" (id integer);",
        )];
        let job_catalog = catalog.clone();
        let settings = database.settings();
        let mut tasks = JoinSet::new();
        tasks.spawn(async move {
            run_migration_job(
                &settings,
                &job_catalog,
                packaged::SCHEMA_PRISMA,
                &NoJitter,
                &build,
            )
            .await
        });
        started.notified().await;
        assert!(
            !finished.load(Ordering::SeqCst),
            "the job waits for the build"
        );
        assert!(tasks.try_join_next().is_none(), "the job is still running");
        release.notify_one();
        let outcome = tasks
            .join_next()
            .await
            .expect("the job task")
            .expect("the job joins")
            .expect("the job completes");
        assert_eq!(outcome, JobOutcome::Completed);
        assert_failed_index_build_keeps_the_migration(&database, &catalog).await;
    })
    .await;
}

// a failed index build does not undo the applied migration; it reports the index exit
async fn assert_failed_index_build_keeps_the_migration(
    database: &SchemaDatabase,
    catalog: &[Migration],
) {
    let failing = JobBuild::immediate(false);
    let settings = database.settings();
    let outcome = run_migration_job(
        &settings,
        catalog,
        packaged::SCHEMA_PRISMA,
        &NoJitter,
        &failing,
    )
    .await
    .expect("the job reports a failed build");
    assert_eq!(outcome, JobOutcome::IndexJobFailed);
    assert!(table_exists(database, "B051_Job").await);
    assert_eq!(ledger_rows(database).await.len(), 1);
}

#[tokio::test]
async fn postgres_pinned_rows() {
    with_schema(|database| async move {
        let settings = database.settings();
        setup_database(
            &settings,
            &packaged::migrations(),
            packaged::SCHEMA_PRISMA,
            &NoJitter,
        )
        .await
        .expect("the packaged migrations apply");
        assert_packaged_ledger(&database).await;

        insert_spend_log(&database).await;
        setup_database(
            &settings,
            &packaged::migrations(),
            packaged::SCHEMA_PRISMA,
            &NoJitter,
        )
        .await
        .expect("a second startup keeps the schema");
        assert_eq!(row_count(&database, "LiteLLM_SpendLogs").await, 1);
        assert!(build_request_log_indexes(&settings).await);
        assert_eq!(
            present_request_log_indexes(&database).await,
            catalog_request_log_indexes(),
            "the build created every request-log index"
        );
        assert_eq!(spend_log_replica_identity(&database).await, "d");
        assert_replica_identity_full(&database, &settings).await;
    })
    .await;
}

async fn assert_packaged_ledger(database: &SchemaDatabase) {
    let catalog = packaged::migrations();
    assert!(
        catalog
            .iter()
            .any(|migration| migration.name == "20250326162113_baseline"),
        "the packaged catalog carries the baseline migration"
    );
    let rows = ledger_rows(database).await;
    let ledger: Vec<(String, String, bool, bool)> = rows
        .iter()
        .map(|row| {
            (
                row.name.clone(),
                row.checksum.clone(),
                row.finished,
                row.rolled_back,
            )
        })
        .collect();
    let pinned: Vec<(String, String, bool, bool)> = catalog
        .iter()
        .map(|migration| {
            (
                migration.name.clone(),
                migration.checksum.clone(),
                true,
                false,
            )
        })
        .collect();
    assert_eq!(
        ledger, pinned,
        "the ledger records every packaged migration"
    );
}

async fn insert_spend_log(database: &SchemaDatabase) {
    let client = database.pool.get().await.expect("a fixture connection");
    let inserted = client
        .execute(
            "INSERT INTO \"LiteLLM_SpendLogs\" (\"request_id\", \"call_type\", \"startTime\", \"endTime\") \
             VALUES ('b051-row', 'acompletion', CURRENT_TIMESTAMP, CURRENT_TIMESTAMP)",
            &[],
        )
        .await
        .expect("a pinned spend-log row inserts");
    assert_eq!(inserted, 1);
}

async fn spend_log_replica_identity(database: &SchemaDatabase) -> String {
    scalar(
        database,
        "SELECT c.relreplident::text FROM pg_class c \
         JOIN pg_namespace n ON n.oid = c.relnamespace \
         WHERE n.nspname = $1 AND c.relname = 'LiteLLM_SpendLogs'",
        &[&database.schema],
    )
    .await
}

async fn assert_replica_identity_full(database: &SchemaDatabase, settings: &StartupSettings) {
    let mut full_identity = settings.clone();
    full_identity.set_replica_identity_full = StartupSwitch(true);
    setup_database(
        &full_identity,
        &packaged::migrations(),
        packaged::SCHEMA_PRISMA,
        &NoJitter,
    )
    .await
    .expect("the opt-in identity pass runs");
    assert_eq!(spend_log_replica_identity(database).await, "f");
}

#[tokio::test]
async fn postgres_failure_effects() {
    with_schema(|database| async move {
        let settings = database.settings();
        let catalog = failing_catalog();
        let recovery = setup_database(&settings, &catalog, packaged::SCHEMA_PRISMA, &NoJitter)
            .await
            .expect_err("the failing migration stops setup");
        assert_eq!(classify(&recovery), SetupFailure::Unrecoverable);
        assert!(
            recovery.to_string().contains("20260101000002_b051_second"),
            "the failure names its migration: {recovery}"
        );

        // the preceding migration committed, and the failing migration's transaction left nothing
        assert!(table_exists(&database, "B051_First").await);
        assert_eq!(row_count(&database, "B051_First").await, 1);
        assert!(!table_exists(&database, "B051_Second").await);
        assert_failed_ledger_row(&database).await;
        assert_unfinished_history_stops_the_job(&database, &catalog).await;
    })
    .await;
}

fn failing_catalog() -> [Migration; 2] {
    [
        migration(
            "20260101000001_b051_first",
            "CREATE TABLE \"B051_First\" (id integer); \
             INSERT INTO \"B051_First\" (id) VALUES (1);",
        ),
        migration(
            "20260101000002_b051_second",
            "CREATE TABLE \"B051_Second\" (id integer); \
             INSERT INTO \"B051_Missing\" (id) VALUES (1);",
        ),
    ]
}

async fn assert_failed_ledger_row(database: &SchemaDatabase) {
    let rows = ledger_rows(database).await;
    assert_eq!(rows.len(), 2);
    assert!(rows[0].finished);
    assert!(!rows[0].rolled_back);
    assert!(!rows[1].finished);
    assert!(!rows[1].rolled_back);
    let logs = rows[1].logs.as_deref().expect("the failure is logged");
    assert!(logs.contains("B051_Missing"), "{logs}");
}

// the unfinished row blocks the job like LiteLLM's P3018 until an operator resolves it
async fn assert_unfinished_history_stops_the_job(database: &SchemaDatabase, catalog: &[Migration]) {
    let build = JobBuild::immediate(true);
    let settings = database.settings();
    let error = run_migration_job(
        &settings,
        catalog,
        packaged::SCHEMA_PRISMA,
        &NoJitter,
        &build,
    )
    .await
    .expect_err("the unfinished history stops the job");
    assert_eq!(classify(&error), SetupFailure::Unrecoverable);
    assert!(
        error.to_string().contains("could not be verified"),
        "{error}"
    );
    assert_eq!(ledger_rows(database).await.len(), 2);
}

#[tokio::test]
async fn native_betterlite_db_admin_migrate_replaces_python_migrations_run_py_and_prisma_migration()
{
    with_schema(|database| async move {
        let mut settings = database.settings();
        settings.bootstrap_inputs = vec!["PRISMA_OFFLINE_MODE", "LITELLM_MIGRATION_DIR"];
        let build = RequestLogIndexBuild {
            settings: &settings,
        };
        let outcome = run_migration_job(
            &settings,
            &packaged::migrations(),
            packaged::SCHEMA_PRISMA,
            &NoJitter,
            &build,
        )
        .await
        .expect("the native migration job completes");
        assert_eq!(outcome, JobOutcome::Completed);
        let rows = ledger_rows(&database).await;
        assert_eq!(rows.len(), packaged::migrations().len());
        assert!(rows.iter().all(|row| row.finished && !row.rolled_back));

        assert_eq!(
            present_request_log_indexes(&database).await,
            catalog_request_log_indexes(),
            "the migration job built every request-log index"
        );

        // a failed index build reports the index-job exit instead of reverting the migrations
        let failing = JobBuild::immediate(false);
        let index_failure = run_migration_job(
            &settings,
            &packaged::migrations(),
            packaged::SCHEMA_PRISMA,
            &NoJitter,
            &failing,
        )
        .await
        .expect("an index failure is not a migration failure");
        assert_eq!(index_failure, JobOutcome::IndexJobFailed);
    })
    .await;
}
