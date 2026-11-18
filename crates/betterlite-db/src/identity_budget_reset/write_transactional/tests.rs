#![cfg(test)]

use rust_decimal::Decimal;

use super::write_resets;
use crate::database_faults::ErrorType;
use crate::identity_budget_reset::phase::IdentityPhase;
use crate::identity_budget_reset::tests::support::{
    CountingReconnect, ReleasingReconnect, assert_spend, close_store, finish, moment, naive, past,
    reset_at_of, schema, serial_pool, write,
};
use crate::postgres_store::session::PostgresStore;

const REJECT_TRIGGER: &str = r#"
CREATE FUNCTION reject_key_update() RETURNS trigger AS $$ BEGIN RAISE EXCEPTION 'commit rejected'; END; $$ LANGUAGE plpgsql;
CREATE CONSTRAINT TRIGGER reject_key_update AFTER UPDATE ON "LiteLLM_VerificationToken" DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION reject_key_update();
"#;

const CUT_TRIGGER: &str = r#"
CREATE FUNCTION cut_key_connection() RETURNS trigger AS $$ BEGIN PERFORM pg_terminate_backend(pg_backend_pid()); RETURN NULL; END; $$ LANGUAGE plpgsql;
CREATE CONSTRAINT TRIGGER cut_key_connection AFTER UPDATE ON "LiteLLM_VerificationToken" DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION cut_key_connection();
"#;

const FAIL_TRIGGER: &str = r#"
CREATE FUNCTION fail_key_update() RETURNS trigger AS $$ BEGIN RAISE EXCEPTION 'write rejected'; END; $$ LANGUAGE plpgsql;
CREATE TRIGGER fail_key_update BEFORE UPDATE ON "LiteLLM_VerificationToken" FOR EACH ROW EXECUTE FUNCTION fail_key_update();
"#;

#[tokio::test]
async fn write_transactional_commits_every_queued_row() {
    let (fixture, store, client) = schema().await;
    client
        .batch_execute(
            "INSERT INTO \"LiteLLM_VerificationToken\" (token, spend, budget_reset_at) VALUES ('k1', 10, NULL), ('k2', 10, NULL);",
        )
        .await
        .unwrap();
    let writes = vec![
        write("k1", Some(moment(30)), Decimal::from(10)),
        write("k2", Some(moment(30)), Decimal::from(10)),
    ];
    write_resets(
        &store,
        IdentityPhase::Keys,
        writes,
        None::<&mut CountingReconnect>,
    )
    .await
    .unwrap();
    assert_spend(&client, "LiteLLM_VerificationToken", "token", "k1", 0.0).await;
    assert_spend(&client, "LiteLLM_VerificationToken", "token", "k2", 0.0).await;
    assert_eq!(
        reset_at_of(&client, "LiteLLM_VerificationToken", "token", "k1").await,
        Some(naive(30))
    );
    finish(fixture, store, client).await;
}

#[tokio::test]
async fn write_transactional_rolls_back_the_whole_chunk_for_a_missing_row() {
    let (fixture, store, client) = schema().await;
    client
        .batch_execute(
            "INSERT INTO \"LiteLLM_VerificationToken\" (token, spend) VALUES ('k1', 10);",
        )
        .await
        .unwrap();
    let writes = vec![
        write("k1", Some(moment(30)), Decimal::from(10)),
        write("ghost", Some(moment(30)), Decimal::from(10)),
    ];
    let error = write_resets(
        &store,
        IdentityPhase::Keys,
        writes,
        None::<&mut CountingReconnect>,
    )
    .await
    .unwrap_err();
    assert_eq!(error.error_type, ErrorType::Other);
    assert_eq!(
        error.message,
        "no row matched the reset target in LiteLLM_VerificationToken"
    );
    assert_spend(&client, "LiteLLM_VerificationToken", "token", "k1", 10.0).await;
    assert_eq!(
        reset_at_of(&client, "LiteLLM_VerificationToken", "token", "k1").await,
        None
    );
    finish(fixture, store, client).await;
}

#[tokio::test]
async fn write_transactional_retries_a_pool_timeout_after_reconnecting() {
    let (fixture, store, client) = schema().await;
    client
        .batch_execute(
            "INSERT INTO \"LiteLLM_VerificationToken\" (token, spend) VALUES ('k1', 10);",
        )
        .await
        .unwrap();
    let serial = PostgresStore {
        pool: Some(serial_pool(&fixture.options.url)),
    };
    let held = serial.session().await.unwrap();
    let mut reconnect = ReleasingReconnect::new(held);
    let writes = vec![write("k1", Some(moment(30)), Decimal::from(10))];
    write_resets(&serial, IdentityPhase::Keys, writes, Some(&mut reconnect))
        .await
        .unwrap();
    assert_eq!(reconnect.attempts(), 1);
    assert_spend(&client, "LiteLLM_VerificationToken", "token", "k1", 0.0).await;
    close_store(serial);
    finish(fixture, store, client).await;
}

#[tokio::test]
async fn write_transactional_does_not_retry_a_commit_rejected_by_the_database() {
    let (fixture, store, client) = schema().await;
    client.batch_execute(REJECT_TRIGGER).await.unwrap();
    client
        .batch_execute(
            "INSERT INTO \"LiteLLM_VerificationToken\" (token, spend) VALUES ('k1', 10);",
        )
        .await
        .unwrap();
    let mut reconnect = CountingReconnect::new(true);
    let writes = vec![write("k1", Some(moment(30)), Decimal::from(10))];
    let error = write_resets(&store, IdentityPhase::Keys, writes, Some(&mut reconnect))
        .await
        .unwrap_err();
    assert_eq!(error.error_type, ErrorType::Other);
    assert!(error.message.contains("database SQLSTATE"));
    assert_eq!(reconnect.attempts(), 0);
    assert_spend(&client, "LiteLLM_VerificationToken", "token", "k1", 10.0).await;
    finish(fixture, store, client).await;
}

#[tokio::test]
async fn write_transactional_does_not_retry_an_unknown_commit_outcome() {
    let (fixture, store, client) = schema().await;
    client.batch_execute(CUT_TRIGGER).await.unwrap();
    client
        .batch_execute(
            "INSERT INTO \"LiteLLM_VerificationToken\" (token, spend) VALUES ('k1', 10);",
        )
        .await
        .unwrap();
    let mut reconnect = CountingReconnect::new(true);
    let writes = vec![write("k1", Some(moment(30)), Decimal::from(10))];
    let error = write_resets(&store, IdentityPhase::Keys, writes, Some(&mut reconnect))
        .await
        .unwrap_err();
    assert_eq!(error.error_type, ErrorType::ReadTimeout);
    assert_eq!(reconnect.attempts(), 0);
    assert_spend(&client, "LiteLLM_VerificationToken", "token", "k1", 10.0).await;
    finish(fixture, store, client).await;
}

#[tokio::test]
async fn write_transactional_does_not_retry_a_statement_rejected_by_the_database() {
    let (fixture, store, client) = schema().await;
    client.batch_execute(FAIL_TRIGGER).await.unwrap();
    client
        .batch_execute(
            "INSERT INTO \"LiteLLM_VerificationToken\" (token, spend) VALUES ('k1', 10);",
        )
        .await
        .unwrap();
    let mut reconnect = CountingReconnect::new(true);
    let writes = vec![write("k1", Some(moment(30)), Decimal::from(10))];
    let error = write_resets(&store, IdentityPhase::Keys, writes, Some(&mut reconnect))
        .await
        .unwrap_err();
    assert_eq!(error.error_type, ErrorType::Other);
    assert!(error.message.contains("database SQLSTATE"));
    assert_eq!(reconnect.attempts(), 0);
    assert_spend(&client, "LiteLLM_VerificationToken", "token", "k1", 10.0).await;
    finish(fixture, store, client).await;
}

#[tokio::test]
async fn write_transactional_retries_a_missing_connection_once() {
    let store = PostgresStore { pool: None };
    let mut reconnect = CountingReconnect::new(true);
    let writes = vec![write("k1", Some(moment(30)), Decimal::from(10))];
    let error = write_resets(&store, IdentityPhase::Keys, writes, Some(&mut reconnect))
        .await
        .unwrap_err();
    assert_eq!(error.error_type, ErrorType::Connect);
    assert_eq!(reconnect.attempts(), 1);
    assert_eq!(
        reconnect.reasons(),
        vec!["reset_budget_write_keys_failure".to_owned()]
    );
}

#[tokio::test]
async fn write_transactional_updates_users_in_the_user_table() {
    let (fixture, store, client) = schema().await;
    client
        .batch_execute("INSERT INTO \"LiteLLM_UserTable\" (user_id, spend) VALUES ('u1', 10);")
        .await
        .unwrap();
    let writes = vec![write("u1", Some(past()), Decimal::from(10))];
    write_resets(
        &store,
        IdentityPhase::Users,
        writes,
        None::<&mut CountingReconnect>,
    )
    .await
    .unwrap();
    assert_spend(&client, "LiteLLM_UserTable", "user_id", "u1", 0.0).await;
    assert_eq!(
        reset_at_of(&client, "LiteLLM_UserTable", "user_id", "u1").await,
        Some(past())
    );
    finish(fixture, store, client).await;
}
