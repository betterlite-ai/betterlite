use secrecy::SecretString;

use crate::migration_ledger::lock::MIGRATION_LOCK_KEY;
use crate::request_log_index_catalog::definitions::RequestLogIndex;
use crate::request_log_indexes::ensure::ensure_request_log_indexes;

use super::support::{
    API_KEY_INDEX, CALL_ID_INDEX, FakeDatabase, FakeFault, FixedJitter, PostgresTest,
    RecordingPause, SPEND_LOGS, create_spend_logs, index_by_name, index_state_of, informed,
    install_logs, plain_table, run_ensure, valid_index, warned,
};

#[tokio::test]
async fn ensure_request_log_indexes_all_in_place_on_empty_list() {
    let (logs, _guard) = install_logs();
    let test = PostgresTest::new().await;
    assert!(ensure_request_log_indexes(&test.url, &test.schema, &[]).await);
    assert!(informed(
        &logs.contents(),
        "Request-log indexes are all in place"
    ));
    test.finish().await;
}

#[tokio::test]
async fn held_coordinator_lock_leaves_the_index_unbuilt() {
    let (logs, _guard) = install_logs();
    let test = PostgresTest::new().await;
    create_spend_logs(&test).await;
    let (holder, mut drivers) = test.another_client().await;
    holder
        .batch_execute(&format!("SELECT pg_advisory_lock({MIGRATION_LOCK_KEY})"))
        .await
        .unwrap();
    assert!(
        !ensure_request_log_indexes(&test.url, &test.schema, &[index_by_name(API_KEY_INDEX)]).await
    );
    assert!(index_state_of(&test.client, API_KEY_INDEX).await.is_none());
    assert!(informed(
        &logs.contents(),
        "Another process holds the migration lock, leaving the request-log indexes to the next index build"
    ));
    drivers.shutdown().await;
    test.finish().await;
}

#[tokio::test]
async fn ensure_request_log_indexes_connection_failure_warns() {
    let (logs, _guard) = install_logs();
    let url = SecretString::from(
        "postgresql://betterlite:betterlite@127.0.0.1:1/betterlite?sslmode=disable",
    );
    assert!(!ensure_request_log_indexes(&url, "public", &[]).await);
    assert!(warned(
        &logs.contents(),
        "Could not build the request-log indexes, leaving them for the next index build"
    ));
}

#[tokio::test]
async fn empty_index_list_succeeds() {
    let (logs, _guard) = install_logs();
    let fake = FakeDatabase::default();
    let jitter = FixedJitter::new(0);
    let pause = RecordingPause::new(&fake);
    assert!(run_ensure(&fake, &[], &jitter, &pause).await.unwrap());
    assert!(informed(
        &logs.contents(),
        "Request-log indexes are all in place"
    ));
    assert!(fake.state().table_kinds_asked.is_empty());
}

fn configure_collision(fake: &FakeDatabase) {
    fake.configure(|state| {
        state.table_kind_sequence.push_back(Ok(Some(plain_table())));
        state.table_kind_sequence.push_back(Ok(None));
        state.index_states.insert(
            API_KEY_INDEX.to_owned(),
            Ok(Some(valid_index("LiteLLM_ErrorLogs"))),
        );
    });
}

#[tokio::test]
async fn false_first_index_still_attempts_second() {
    let (logs, _guard) = install_logs();
    let fake = FakeDatabase::default().working();
    configure_collision(&fake);
    let indexes = [index_by_name(API_KEY_INDEX), index_by_name(CALL_ID_INDEX)];
    let jitter = FixedJitter::new(0);
    let pause = RecordingPause::new(&fake);
    assert!(!run_ensure(&fake, &indexes, &jitter, &pause).await.unwrap());
    assert_collision_logs(&logs.contents());
    assert_eq!(fake.state().table_kinds_asked, [SPEND_LOGS, SPEND_LOGS]);
}

fn assert_collision_logs(logs: &str) {
    assert!(warned(
        logs,
        "Index LiteLLM_SpendLogs_api_key_startTime_idx already exists on LiteLLM_ErrorLogs rather than LiteLLM_SpendLogs, leaving it alone"
    ));
    assert!(informed(
        logs,
        "Table LiteLLM_SpendLogs does not exist yet, skipping index LiteLLM_SpendLogs_litellm_call_id_idx"
    ));
    assert!(warned(
        logs,
        "Some request-log indexes are not in place yet, leaving them for the next index build"
    ));
}

#[tokio::test]
async fn database_error_stops_remaining_indexes() {
    let (logs, _guard) = install_logs();
    let fake = FakeDatabase::default();
    fake.configure(|state| {
        state.table_kind_sequence.push_back(Ok(None));
        state
            .table_kind_sequence
            .push_back(Err(FakeFault::Database));
    });
    let third = RequestLogIndex {
        table: "LiteLLM_Third",
        name: "LiteLLM_Third_idx",
        definition: "(\"id\")",
    };
    let indexes = [
        index_by_name(API_KEY_INDEX),
        index_by_name(CALL_ID_INDEX),
        third,
    ];
    let jitter = FixedJitter::new(0);
    let pause = RecordingPause::new(&fake);
    assert!(!run_ensure(&fake, &indexes, &jitter, &pause).await.unwrap());
    assert_eq!(fake.state().table_kinds_asked, [SPEND_LOGS, SPEND_LOGS]);
    assert!(warned(
        &logs.contents(),
        "Could not build the request-log indexes, leaving them for the next index build"
    ));
}

#[tokio::test]
async fn non_database_error_propagates() {
    let (logs, _guard) = install_logs();
    let fake = FakeDatabase::default();
    fake.configure(|state| {
        state
            .table_kind_sequence
            .push_back(Err(FakeFault::NotDatabase));
    });
    let jitter = FixedJitter::new(0);
    let pause = RecordingPause::new(&fake);
    let error = run_ensure(&fake, &[index_by_name(API_KEY_INDEX)], &jitter, &pause)
        .await
        .unwrap_err();
    assert_eq!(error.fault, FakeFault::NotDatabase);
    assert!(!warned(
        &logs.contents(),
        "Could not build the request-log indexes"
    ));
}

#[tokio::test]
async fn missing_table_is_successful_skip() {
    let (logs, _guard) = install_logs();
    let fake = FakeDatabase::default();
    fake.configure(|state| state.table_kind_sequence.push_back(Ok(None)));
    let jitter = FixedJitter::new(0);
    let pause = RecordingPause::new(&fake);
    assert!(
        run_ensure(&fake, &[index_by_name(API_KEY_INDEX)], &jitter, &pause)
            .await
            .unwrap()
    );
    let captured = logs.contents();
    assert!(informed(
        &captured,
        "Table LiteLLM_SpendLogs does not exist yet, skipping index LiteLLM_SpendLogs_api_key_startTime_idx"
    ));
    assert!(informed(&captured, "Request-log indexes are all in place"));
}
