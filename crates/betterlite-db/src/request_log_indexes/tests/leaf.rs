use std::time::Duration;

use crate::migration_ledger::lock::MIGRATION_LOCK_KEY;
use crate::request_log_index_catalog::catalog::IndexState;
use crate::request_log_indexes::database::IndexRef;
use crate::request_log_indexes::locks::HANDOVER;
use crate::request_log_indexes::report::report_second_copies;

use super::support::{
    API_KEY_INDEX, CALL_ID_INDEX, FakeDatabase, FixedJitter, PostgresTest, RecordingDatabase,
    RecordingPause, SPEND_LOGS, create_spend_logs, duplicate_api_key_rows, index_builder,
    index_by_name, index_columns, index_dependencies, index_oid, index_state_of, informed,
    install_logs, invalid_index, leave_invalid_index, oid_in_schema, other_schema, plain_table,
    relation_kind, run_leaf, valid_index, warned,
};

#[tokio::test]
async fn postgres_coordinator_lock_held_skips_leaf() {
    let (logs, _guard) = install_logs();
    let test = PostgresTest::new().await;
    create_spend_logs(&test).await;
    let (holder, mut drivers) = test.another_client().await;
    holder
        .query_one("SELECT pg_advisory_lock($1)", &[&MIGRATION_LOCK_KEY])
        .await
        .unwrap();
    let recorder = RecordingDatabase::new(test.service().await);
    let jitter = FixedJitter::new(0);
    let pause = RecordingPause::standalone();
    let dependencies = index_dependencies(&jitter, &pause);
    let builder = index_builder(&recorder, &test.schema, &dependencies);
    let built = run_leaf(
        &builder,
        SPEND_LOGS,
        API_KEY_INDEX,
        index_by_name(API_KEY_INDEX),
    )
    .await;
    assert!(!built);
    assert!(recorder.statements().is_empty());
    assert!(informed(
        &logs.contents(),
        "Another process holds the migration lock, leaving the request-log indexes to the next index build"
    ));
    holder
        .query_one("SELECT pg_advisory_unlock($1)", &[&MIGRATION_LOCK_KEY])
        .await
        .unwrap();
    drivers.shutdown().await;
    test.finish().await;
}

#[tokio::test]
async fn postgres_coordinator_released_after_leaf() {
    let test = PostgresTest::new().await;
    create_spend_logs(&test).await;
    let recorder = RecordingDatabase::new(test.service().await);
    let jitter = FixedJitter::new(0);
    let pause = RecordingPause::standalone();
    let dependencies = index_dependencies(&jitter, &pause);
    let builder = index_builder(&recorder, &test.schema, &dependencies);
    assert!(
        run_leaf(
            &builder,
            SPEND_LOGS,
            API_KEY_INDEX,
            index_by_name(API_KEY_INDEX)
        )
        .await
    );
    let (probe, mut drivers) = test.another_client().await;
    let acquired: bool = probe
        .query_one(
            "SELECT pg_try_advisory_lock($1) AS acquired",
            &[&MIGRATION_LOCK_KEY],
        )
        .await
        .unwrap()
        .get("acquired");
    assert!(acquired);
    probe
        .query_one("SELECT pg_advisory_unlock($1)", &[&MIGRATION_LOCK_KEY])
        .await
        .unwrap();
    drivers.shutdown().await;
    test.finish().await;
}

async fn assert_leaf_repair_state(
    test: &PostgresTest,
    invalid_oid: u32,
    copy_oid: u32,
    adoption_oid: u32,
) {
    assert!(index_state_of(&test.client, API_KEY_INDEX).await.unwrap().1);
    assert!(index_state_of(&test.client, CALL_ID_INDEX).await.unwrap().1);
    assert_ne!(
        index_oid(&test.client, API_KEY_INDEX).await.unwrap(),
        invalid_oid
    );
    assert_eq!(
        index_oid(&test.client, "api_key_copy").await.unwrap(),
        copy_oid
    );
    assert_eq!(
        index_oid(&test.client, CALL_ID_INDEX).await.unwrap(),
        adoption_oid
    );
}

fn assert_repair_statements(recorder: &RecordingDatabase, schema: &str) {
    assert_eq!(
        recorder.statements(),
        [
            format!("DROP INDEX CONCURRENTLY \"{schema}\".\"{API_KEY_INDEX}\""),
            format!(
                "CREATE INDEX CONCURRENTLY IF NOT EXISTS \"{API_KEY_INDEX}\" ON \"{schema}\".\"{SPEND_LOGS}\" (\"api_key\", \"startTime\")"
            ),
            format!("ALTER INDEX \"{schema}\".\"call_id_copy\" RENAME TO \"{CALL_ID_INDEX}\""),
        ]
    );
}

fn assert_repair_logs(logs: &str) {
    assert!(informed(
        logs,
        "Dropping the invalid index LiteLLM_SpendLogs_api_key_startTime_idx left by an interrupted build on LiteLLM_SpendLogs"
    ));
    assert!(informed(
        logs,
        "Building index LiteLLM_SpendLogs_api_key_startTime_idx on LiteLLM_SpendLogs concurrently"
    ));
    assert!(informed(
        logs,
        "Renaming the equivalent index call_id_copy on LiteLLM_SpendLogs to LiteLLM_SpendLogs_litellm_call_id_idx instead of building a second one"
    ));
}

async fn create_invalid_fixture(test: &PostgresTest, extras: &str) -> (u32, u32) {
    create_spend_logs(test).await;
    duplicate_api_key_rows(&test.client, SPEND_LOGS).await;
    leave_invalid_index(
        &test.client,
        API_KEY_INDEX,
        SPEND_LOGS,
        "\"api_key\", \"startTime\"",
    )
    .await;
    let setup = format!(
        "CREATE INDEX api_key_copy ON \"{SPEND_LOGS}\" (\"api_key\", \"startTime\"); {extras}"
    );
    test.client.batch_execute(&setup).await.unwrap();
    let invalid_oid = index_oid(&test.client, API_KEY_INDEX).await.unwrap();
    let copy_oid = index_oid(&test.client, "api_key_copy").await.unwrap();
    (invalid_oid, copy_oid)
}

#[tokio::test]
async fn postgres_leaf_repair_and_adoption() {
    let (logs, _guard) = install_logs();
    let test = PostgresTest::new().await;
    let extras = format!("CREATE INDEX call_id_copy ON \"{SPEND_LOGS}\" (litellm_call_id);");
    let (invalid_oid, copy_oid) = create_invalid_fixture(&test, &extras).await;
    let adoption_oid = index_oid(&test.client, "call_id_copy").await.unwrap();
    let recorder = RecordingDatabase::new(test.service().await);
    let jitter = FixedJitter::new(0);
    let pause = RecordingPause::standalone();
    let dependencies = index_dependencies(&jitter, &pause);
    let builder = index_builder(&recorder, &test.schema, &dependencies);
    assert!(
        run_leaf(
            &builder,
            SPEND_LOGS,
            API_KEY_INDEX,
            index_by_name(API_KEY_INDEX)
        )
        .await
    );
    assert!(
        run_leaf(
            &builder,
            SPEND_LOGS,
            CALL_ID_INDEX,
            index_by_name(CALL_ID_INDEX)
        )
        .await
    );
    assert_leaf_repair_state(&test, invalid_oid, copy_oid, adoption_oid).await;
    assert_repair_statements(&recorder, &test.schema);
    assert_repair_logs(&logs.contents());
    test.finish().await;
}

#[tokio::test]
async fn invalid_equivalent_is_never_adopted() {
    let (logs, _guard) = install_logs();
    let test = PostgresTest::new().await;
    create_spend_logs(&test).await;
    leave_invalid_equivalent(&test, "api_key_broken").await;
    let invalid_oid = index_oid(&test.client, "api_key_broken").await.unwrap();
    assert!(
        !index_state_of(&test.client, "api_key_broken")
            .await
            .unwrap()
            .1
    );
    let recorder = RecordingDatabase::new(test.service().await);
    let jitter = FixedJitter::new(0);
    let pause = RecordingPause::standalone();
    let dependencies = index_dependencies(&jitter, &pause);
    let builder = index_builder(&recorder, &test.schema, &dependencies);
    assert!(
        run_leaf(
            &builder,
            SPEND_LOGS,
            API_KEY_INDEX,
            index_by_name(API_KEY_INDEX)
        )
        .await
    );
    assert_built_not_adopted(&test, invalid_oid, &recorder).await;
    assert!(warned(
        &logs.contents(),
        "Index api_key_broken on LiteLLM_SpendLogs is a second copy of LiteLLM_SpendLogs_api_key_startTime_idx"
    ));
    test.finish().await;
}

async fn assert_built_not_adopted(
    test: &PostgresTest,
    invalid_oid: u32,
    recorder: &RecordingDatabase,
) {
    assert!(index_state_of(&test.client, API_KEY_INDEX).await.unwrap().1);
    assert_eq!(
        index_oid(&test.client, "api_key_broken").await.unwrap(),
        invalid_oid
    );
    let statements = recorder.statements();
    assert!(
        statements.iter().any(|statement| statement.contains(
            "CREATE INDEX CONCURRENTLY IF NOT EXISTS \"LiteLLM_SpendLogs_api_key_startTime_idx\""
        )),
        "{statements:?}"
    );
    assert!(
        !statements
            .iter()
            .any(|statement| statement.starts_with("ALTER INDEX")),
        "{statements:?}"
    );
}

// A read-only transaction pins the snapshot, so the concurrent build times out and leaves its index invalid
async fn leave_invalid_equivalent(test: &PostgresTest, name: &str) {
    let (mut holder, mut drivers) = test.another_client().await;
    let pin = holder.transaction().await.unwrap();
    pin.query(&format!("SELECT * FROM \"{SPEND_LOGS}\""), &[])
        .await
        .unwrap();
    test.client
        .batch_execute("SET statement_timeout = '100ms'")
        .await
        .unwrap();
    test.client
        .batch_execute(&format!(
            "CREATE INDEX CONCURRENTLY \"{name}\" ON \"{SPEND_LOGS}\" (\"api_key\", \"startTime\")"
        ))
        .await
        .unwrap_err();
    test.client
        .batch_execute("SET statement_timeout = 0")
        .await
        .unwrap();
    pin.rollback().await.unwrap();
    drivers.shutdown().await;
}

#[tokio::test]
async fn valid_expected_definition_not_rechecked() {
    let test = PostgresTest::new().await;
    create_spend_logs(&test).await;
    test.client
        .batch_execute(&format!(
            "CREATE INDEX \"{API_KEY_INDEX}\" ON \"{SPEND_LOGS}\" (litellm_call_id)"
        ))
        .await
        .unwrap();
    let recorder = RecordingDatabase::new(test.service().await);
    let jitter = FixedJitter::new(0);
    let pause = RecordingPause::standalone();
    let dependencies = index_dependencies(&jitter, &pause);
    let builder = index_builder(&recorder, &test.schema, &dependencies);
    assert!(
        run_leaf(
            &builder,
            SPEND_LOGS,
            API_KEY_INDEX,
            index_by_name(API_KEY_INDEX)
        )
        .await
    );
    assert!(recorder.statements().is_empty());
    assert_eq!(
        index_columns(&test.client, API_KEY_INDEX).await,
        ["litellm_call_id"]
    );
    test.finish().await;
}

#[tokio::test]
async fn same_relname_other_namespace_accepted() {
    let test = PostgresTest::new().await;
    create_spend_logs(&test).await;
    let other = other_schema(&test.schema);
    test.client
        .batch_execute(&format!(
            "CREATE SCHEMA \"{other}\"; CREATE TABLE \"{other}\".\"{SPEND_LOGS}\" (api_key text, \"startTime\" timestamptz, litellm_call_id text); CREATE INDEX \"{API_KEY_INDEX}\" ON \"{other}\".\"{SPEND_LOGS}\" (\"api_key\", \"startTime\");"
        ))
        .await
        .unwrap();
    let other_oid = oid_in_schema(&test.client, &other, API_KEY_INDEX)
        .await
        .unwrap();
    let recorder = RecordingDatabase::new(test.service().await);
    let jitter = FixedJitter::new(0);
    let pause = RecordingPause::standalone();
    let dependencies = index_dependencies(&jitter, &pause);
    let builder = index_builder(&recorder, &test.schema, &dependencies);
    assert!(
        run_leaf(
            &builder,
            SPEND_LOGS,
            API_KEY_INDEX,
            index_by_name(API_KEY_INDEX)
        )
        .await
    );
    assert!(index_state_of(&test.client, API_KEY_INDEX).await.unwrap().1);
    assert_eq!(
        oid_in_schema(&test.client, &other, API_KEY_INDEX)
            .await
            .unwrap(),
        other_oid
    );
    test.client
        .batch_execute(&format!("DROP SCHEMA \"{other}\" CASCADE"))
        .await
        .unwrap();
    test.finish().await;
}

#[tokio::test]
async fn wrong_table_collision_untouched() {
    let (logs, _guard) = install_logs();
    let test = PostgresTest::new().await;
    create_spend_logs(&test).await;
    test.client
        .batch_execute(&format!(
            "CREATE TABLE \"LiteLLM_ErrorLogs\" (request_id text); CREATE INDEX \"{CALL_ID_INDEX}\" ON \"LiteLLM_ErrorLogs\" (request_id);"
        ))
        .await
        .unwrap();
    let collision_oid = index_oid(&test.client, CALL_ID_INDEX).await.unwrap();
    let recorder = RecordingDatabase::new(test.service().await);
    let jitter = FixedJitter::new(0);
    let pause = RecordingPause::standalone();
    let dependencies = index_dependencies(&jitter, &pause);
    let builder = index_builder(&recorder, &test.schema, &dependencies);
    assert!(
        !run_leaf(
            &builder,
            SPEND_LOGS,
            CALL_ID_INDEX,
            index_by_name(CALL_ID_INDEX)
        )
        .await
    );
    assert_eq!(
        index_oid(&test.client, CALL_ID_INDEX).await.unwrap(),
        collision_oid
    );
    assert!(recorder.statements().is_empty());
    assert!(pause.delays().is_empty());
    assert!(warned(
        &logs.contents(),
        "Index LiteLLM_SpendLogs_litellm_call_id_idx already exists on LiteLLM_ErrorLogs rather than LiteLLM_SpendLogs, leaving it alone"
    ));
    test.finish().await;
}

#[tokio::test]
async fn invalid_leaf_drops_before_create_without_adoption() {
    let (logs, _guard) = install_logs();
    let test = PostgresTest::new().await;
    let (invalid_oid, copy_oid) = create_invalid_fixture(&test, "").await;
    let recorder = RecordingDatabase::new(test.service().await);
    let jitter = FixedJitter::new(0);
    let pause = RecordingPause::standalone();
    let dependencies = index_dependencies(&jitter, &pause);
    let builder = index_builder(&recorder, &test.schema, &dependencies);
    assert!(
        run_leaf(
            &builder,
            SPEND_LOGS,
            API_KEY_INDEX,
            index_by_name(API_KEY_INDEX)
        )
        .await
    );
    assert_rebuild_without_adoption(&recorder, &test, invalid_oid, copy_oid).await;
    assert_eq!(pause.delays(), [HANDOVER]);
    assert!(warned(
        &logs.contents(),
        &format!(
            "Index api_key_copy on {SPEND_LOGS} is a second copy of {API_KEY_INDEX} and only costs writes and disk; remove it with: DROP INDEX CONCURRENTLY \"{}\".\"api_key_copy\"",
            test.schema
        )
    ));
    test.finish().await;
}

async fn assert_rebuild_without_adoption(
    recorder: &RecordingDatabase,
    test: &PostgresTest,
    invalid_oid: u32,
    copy_oid: u32,
) {
    let schema = &test.schema;
    assert_eq!(
        recorder.statements(),
        [
            format!("DROP INDEX CONCURRENTLY \"{schema}\".\"{API_KEY_INDEX}\""),
            format!(
                "CREATE INDEX CONCURRENTLY IF NOT EXISTS \"{API_KEY_INDEX}\" ON \"{schema}\".\"{SPEND_LOGS}\" (\"api_key\", \"startTime\")"
            ),
        ]
    );
    assert_ne!(
        index_oid(&test.client, API_KEY_INDEX).await.unwrap(),
        invalid_oid
    );
    assert_eq!(
        index_oid(&test.client, "api_key_copy").await.unwrap(),
        copy_oid
    );
}

#[tokio::test]
async fn nonindex_name_collision() {
    let (logs, _guard) = install_logs();
    let test = PostgresTest::new().await;
    create_spend_logs(&test).await;
    test.client
        .batch_execute(&format!("CREATE TABLE \"{API_KEY_INDEX}\" (x int)"))
        .await
        .unwrap();
    let recorder = RecordingDatabase::new(test.service().await);
    let jitter = FixedJitter::new(0);
    let pause = RecordingPause::standalone();
    let dependencies = index_dependencies(&jitter, &pause);
    let builder = index_builder(&recorder, &test.schema, &dependencies);
    assert!(
        !run_leaf(
            &builder,
            SPEND_LOGS,
            API_KEY_INDEX,
            index_by_name(API_KEY_INDEX)
        )
        .await
    );
    assert_eq!(relation_kind(&test.client, API_KEY_INDEX).await, "r");
    assert!(informed(
        &logs.contents(),
        "Building index LiteLLM_SpendLogs_api_key_startTime_idx on LiteLLM_SpendLogs concurrently"
    ));
    test.finish().await;
}

#[tokio::test]
async fn stale_read_rechecked_under_lock() {
    let fake = FakeDatabase::default().working();
    fake.configure(|state| {
        state
            .tables
            .insert(SPEND_LOGS.to_owned(), Ok(Some(plain_table())));
        state
            .index_state_sequence
            .push_back(Ok(Some(invalid_index(SPEND_LOGS))));
        state
            .index_state_sequence
            .push_back(Ok(Some(valid_index(SPEND_LOGS))));
    });
    let jitter = FixedJitter::new(0);
    let pause = RecordingPause::new(&fake);
    let dependencies = index_dependencies(&jitter, &pause);
    let builder = index_builder(&fake, "public", &dependencies);
    assert!(
        run_leaf(
            &builder,
            SPEND_LOGS,
            API_KEY_INDEX,
            index_by_name(API_KEY_INDEX)
        )
        .await
    );
    assert!(fake.state().statements.is_empty());
    assert_eq!(pause.delays(), [HANDOVER]);
    assert_eq!(
        fake.state().index_states_asked,
        [API_KEY_INDEX, API_KEY_INDEX]
    );
}

async fn run_leaf_with_states(states: &[IndexState]) -> (Vec<Duration>, usize) {
    let fake = FakeDatabase::default().working();
    fake.configure(|state| {
        state
            .tables
            .insert(SPEND_LOGS.to_owned(), Ok(Some(plain_table())));
        for status in states {
            state
                .index_state_sequence
                .push_back(Ok(Some(status.clone())));
        }
    });
    let jitter = FixedJitter::new(0);
    let pause = RecordingPause::new(&fake);
    let dependencies = index_dependencies(&jitter, &pause);
    let builder = index_builder(&fake, "public", &dependencies);
    assert!(
        run_leaf(
            &builder,
            SPEND_LOGS,
            API_KEY_INDEX,
            index_by_name(API_KEY_INDEX)
        )
        .await
    );
    (pause.delays(), fake.state().statements.len())
}

#[tokio::test]
async fn leaf_handover_only_after_locked_success() {
    let (valid_delays, valid_statements) = run_leaf_with_states(&[valid_index(SPEND_LOGS)]).await;
    assert!(valid_delays.is_empty());
    assert_eq!(valid_statements, 0);
    let states = [
        invalid_index(SPEND_LOGS),
        invalid_index(SPEND_LOGS),
        valid_index(SPEND_LOGS),
    ];
    let (rebuilt_delays, rebuilt_statements) = run_leaf_with_states(&states).await;
    assert_eq!(rebuilt_delays, [Duration::from_secs(2)]);
    assert_eq!(rebuilt_statements, 2);
}

#[tokio::test]
async fn duplicate_warnings_leave_all_copies() {
    let (logs, _guard) = install_logs();
    let test = PostgresTest::new().await;
    create_spend_logs(&test).await;
    test.client
        .batch_execute(&format!(
            "CREATE INDEX \"{API_KEY_INDEX}\" ON \"{SPEND_LOGS}\" (\"api_key\", \"startTime\"); CREATE INDEX copy_a ON \"{SPEND_LOGS}\" (\"api_key\", \"startTime\"); CREATE INDEX copy_b ON \"{SPEND_LOGS}\" (\"api_key\", \"startTime\");"
        ))
        .await
        .unwrap();
    let copy_a = index_oid(&test.client, "copy_a").await.unwrap();
    let copy_b = index_oid(&test.client, "copy_b").await.unwrap();
    let recorder = RecordingDatabase::new(test.service().await);
    let jitter = FixedJitter::new(0);
    let pause = RecordingPause::standalone();
    let dependencies = index_dependencies(&jitter, &pause);
    let builder = index_builder(&recorder, &test.schema, &dependencies);
    assert!(
        run_leaf(
            &builder,
            SPEND_LOGS,
            API_KEY_INDEX,
            index_by_name(API_KEY_INDEX)
        )
        .await
    );
    assert!(recorder.statements().is_empty());
    assert_eq!(index_oid(&test.client, "copy_a").await.unwrap(), copy_a);
    assert_eq!(index_oid(&test.client, "copy_b").await.unwrap(), copy_b);
    assert_second_copy_warnings(&logs.contents(), &test.schema);
    test.finish().await;
}

fn assert_second_copy_warnings(logs: &str, schema: &str) {
    for copy in ["copy_a", "copy_b"] {
        assert!(warned(
            logs,
            &format!(
                "Index {copy} on {SPEND_LOGS} is a second copy of {API_KEY_INDEX} and only costs writes and disk; remove it with: DROP INDEX CONCURRENTLY \"{schema}\".\"{copy}\""
            )
        ));
    }
}

#[tokio::test]
async fn parent_report_uses_plain_drop() {
    let (logs, _guard) = install_logs();
    let test = PostgresTest::new().await;
    test.client
        .batch_execute(&format!(
            "CREATE TABLE \"{SPEND_LOGS}\" (api_key text, \"startTime\" timestamptz, litellm_call_id text) PARTITION BY RANGE (\"startTime\"); CREATE TABLE parent_leaf PARTITION OF \"{SPEND_LOGS}\" FOR VALUES FROM ('2026-01-01') TO ('2026-02-01'); CREATE INDEX copied_parent ON ONLY \"{SPEND_LOGS}\" (\"api_key\", \"startTime\");"
        ))
        .await
        .unwrap();
    let recorder = RecordingDatabase::new(test.service().await);
    let jitter = FixedJitter::new(0);
    let pause = RecordingPause::standalone();
    let dependencies = index_dependencies(&jitter, &pause);
    let builder = index_builder(&recorder, &test.schema, &dependencies);
    let target = IndexRef {
        schema: &test.schema,
        table: SPEND_LOGS,
        name: API_KEY_INDEX,
    };
    report_second_copies(&builder, target, index_by_name(API_KEY_INDEX), false)
        .await
        .unwrap();
    assert!(warned(
        &logs.contents(),
        &format!(
            "Index copied_parent on {SPEND_LOGS} is a second copy of {API_KEY_INDEX} and only costs writes and disk; remove it with: DROP INDEX \"{}\".\"copied_parent\"",
            test.schema
        )
    ));
    test.finish().await;
}
