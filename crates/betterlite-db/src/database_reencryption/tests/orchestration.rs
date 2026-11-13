use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use betterlite_secrets::secret_codec::Algorithm;
use deadpool_postgres::PoolError;
use rstest::rstest;
use secrecy::SecretString;
use tokio::sync::Notify;
use tokio::task::{JoinSet, yield_now};

use super::fixtures::{
    Events, FakeStorage, MASTER_KEY, PREVIOUS_KEY, cipher, column, decrypted_leaves, events, json,
    keys, log_into, recorded, request, row, sealed, steps,
};
use crate::database_reencryption::MigrationError;
use crate::database_reencryption::outcome::{MigrationFailure, MigrationOutcome, NothingToMigrate};
use crate::database_reencryption::registry::SecretColumn;
use crate::database_reencryption::runner::migrate_if_requested;
use crate::database_reencryption::storage::SecretRow;
use crate::postgres_store::diagnostics::PoolFailure;

fn credentials() -> SecretColumn {
    column("LiteLLM_CredentialsTable", "credential_values")
}

fn legacy_row(key: &str) -> SecretRow {
    row("credential-1", &format!("{{\"api_key\":\"{key}\"}}"))
}

#[tokio::test]
async fn absent_key_skips_database_callback() {
    let events = events();
    let storage = FakeStorage::new(&events, vec![credentials()]);
    let (_, master) = keys();
    let cipher = cipher(Algorithm::XSalsa20Poly1305);
    let disconnected = AtomicBool::new(false);
    let request = request(None, Some(&master), false, &cipher);
    let outcome = migrate_if_requested(
        &request,
        || {
            disconnected.store(true, Ordering::SeqCst);
            Ok(Some(&storage))
        },
        log_into(&events),
        Ok,
    )
    .await;
    assert!(outcome.expect("the run succeeds").is_none());
    assert!(!disconnected.load(Ordering::SeqCst));
    assert!(recorded(&events).is_empty());
}

#[tokio::test]
async fn empty_previous_key_is_request() {
    let events = events();
    let storage = FakeStorage::new(&events, vec![credentials()]);
    let (_, master) = keys();
    let empty = SecretString::from("");
    let cipher = cipher(Algorithm::XSalsa20Poly1305);
    let request = request(Some(&empty), Some(&master), false, &cipher);
    let outcome = migrate_if_requested(&request, || Ok(Some(&storage)), log_into(&events), Ok)
        .await
        .expect("the run succeeds")
        .expect("an empty variable is still a request");
    assert!(matches!(
        outcome,
        MigrationOutcome::Nothing(NothingToMigrate::NothingLeftToMigrate)
    ));
}

#[rstest]
#[case::salt_with_database("salt-database")]
#[case::salt_without_database("salt-no-database")]
#[case::no_database("no-database")]
#[case::same_key("same-key")]
#[tokio::test]
async fn salt_before_database_before_same_key(#[case] scenario: &str) {
    let events = events();
    let storage = FakeStorage::new(&events, vec![credentials()]);
    let previous = SecretString::from(if scenario == "same-key" {
        MASTER_KEY
    } else {
        PREVIOUS_KEY
    });
    let master = SecretString::from(MASTER_KEY);
    let salt = scenario.starts_with("salt");
    let database = !scenario.contains("without") && scenario != "no-database";
    let cipher = cipher(Algorithm::XSalsa20Poly1305);
    let request = request(Some(&previous), Some(&master), salt, &cipher);
    let outcome = if database {
        migrate_if_requested(&request, || Ok(Some(&storage)), log_into(&events), Ok).await
    } else {
        let missing: Option<&FakeStorage> = None;
        migrate_if_requested(&request, || Ok(missing), log_into(&events), Ok).await
    };
    let produced = outcome.expect("the run succeeds").expect("an outcome");
    let expected = match scenario {
        "salt-database" | "salt-no-database" => "salt",
        "no-database" => "no_database",
        _ => "same",
    };
    assert_eq!(outcome_name(&produced), expected);
    assert!(!recorded(&events).iter().any(|event| event == "columns"));
}

fn outcome_name(outcome: &MigrationOutcome) -> &'static str {
    match outcome {
        MigrationOutcome::Nothing(NothingToMigrate::SaltKeyEncryptsStoredValues) => "salt",
        MigrationOutcome::Nothing(NothingToMigrate::NoDatabase) => "no_database",
        MigrationOutcome::Nothing(NothingToMigrate::NothingLeftToMigrate) => "same",
        MigrationOutcome::Migrated { .. } => "migrated",
        MigrationOutcome::Failed(_) => "failed",
    }
}

#[tokio::test]
async fn same_key_with_rows_skips_writes() {
    let events = events();
    let storage = FakeStorage::new(&events, vec![credentials()]);
    let ciphertext = sealed("hunter2", PREVIOUS_KEY, Algorithm::XSalsa20Poly1305);
    storage.script_rows(vec![legacy_row(&ciphertext)]);
    let same = SecretString::from(PREVIOUS_KEY);
    let cipher = cipher(Algorithm::XSalsa20Poly1305);
    let request = request(Some(&same), Some(&same), false, &cipher);
    let outcome = migrate_if_requested(&request, || Ok(Some(&storage)), log_into(&events), Ok)
        .await
        .expect("the run succeeds")
        .expect("an outcome");
    assert!(matches!(
        outcome,
        MigrationOutcome::Nothing(NothingToMigrate::NothingLeftToMigrate)
    ));
    assert!(storage.replaced().is_empty(), "equal keys never write");
    assert!(
        !recorded(&events).iter().any(|event| event == "columns"),
        "equal keys are checked before the count"
    );
}

#[tokio::test]
async fn connected_callback_runs_before_salt_skip() {
    let events = events();
    let storage = FakeStorage::new(&events, vec![credentials()]);
    let (previous, master) = keys();
    let cipher = cipher(Algorithm::XSalsa20Poly1305);
    let request = request(Some(&previous), Some(&master), true, &cipher);
    let flag = Arc::clone(&events);
    let stored = &storage;
    let outcome = migrate_if_requested(
        &request,
        move || {
            flag.lock()
                .expect("the events lock")
                .push("connected".to_owned());
            Ok(Some(stored))
        },
        log_into(&events),
        Ok,
    )
    .await
    .expect("the run succeeds")
    .expect("an outcome");
    assert!(matches!(
        outcome,
        MigrationOutcome::Nothing(NothingToMigrate::SaltKeyEncryptsStoredValues)
    ));
    assert_eq!(recorded(&events)[0], "connected");
    assert!(!recorded(&events).iter().any(|event| event == "columns"));
}

#[tokio::test]
async fn count_write_recount_order() {
    let events = events();
    let storage = FakeStorage::new(&events, vec![credentials()]);
    let legacy = sealed("hunter2", PREVIOUS_KEY, Algorithm::XSalsa20Poly1305);
    storage.script_rows(vec![legacy_row(&legacy)]);
    storage.script_rows(vec![legacy_row(&legacy)]);
    storage.script_rows(vec![row("credential-1", "{\"api_key\":\"plain\"}")]);
    storage.script_update(true);
    let (previous, master) = keys();
    let cipher = cipher(Algorithm::XSalsa20Poly1305);
    let request = request(Some(&previous), Some(&master), false, &cipher);
    let outcome = migrate_if_requested(&request, || Ok(Some(&storage)), log_into(&events), Ok)
        .await
        .expect("the run succeeds")
        .expect("an outcome");
    assert!(matches!(
        outcome,
        MigrationOutcome::Migrated {
            migrated: 1,
            remaining: 0
        }
    ));
    assert_eq!(
        steps(&recorded(&events)),
        vec![
            "columns", "rows", "progress", "columns", "rows", "replace", "columns", "rows",
            "outcome"
        ]
    );
    let replaced = storage.replaced();
    assert_eq!(replaced.len(), 1);
    let new = json(&replaced[0].new);
    assert_eq!(decrypted_leaves(&new, MASTER_KEY), vec!["hunter2"]);
    assert!(decrypted_leaves(&new, PREVIOUS_KEY).is_empty());
}

#[tokio::test]
async fn no_old_ciphertexts_skip_write() {
    let events = events();
    let storage = FakeStorage::new(&events, vec![credentials()]);
    storage.script_rows(vec![row("credential-1", "{\"api_key\":\"plain\"}")]);
    let (previous, master) = keys();
    let cipher = cipher(Algorithm::XSalsa20Poly1305);
    let request = request(Some(&previous), Some(&master), false, &cipher);
    let outcome = migrate_if_requested(&request, || Ok(Some(&storage)), log_into(&events), Ok)
        .await
        .expect("the run succeeds")
        .expect("an outcome");
    assert!(matches!(
        outcome,
        MigrationOutcome::Nothing(NothingToMigrate::NothingLeftToMigrate)
    ));
    assert_eq!(
        steps(&recorded(&events)),
        vec!["columns", "rows", "outcome"]
    );
    assert!(storage.replaced().is_empty());
}

#[tokio::test]
async fn other_worker_finished_zero_zero() {
    let events = events();
    let storage = FakeStorage::new(&events, vec![credentials()]);
    let legacy = sealed("hunter2", PREVIOUS_KEY, Algorithm::XSalsa20Poly1305);
    storage.script_rows(vec![legacy_row(&legacy)]);
    storage.script_rows(vec![legacy_row(&legacy)]);
    storage.script_update(false);
    storage.script_rows(vec![row("credential-1", "{\"api_key\":\"plain\"}")]);
    let (previous, master) = keys();
    let cipher = cipher(Algorithm::XSalsa20Poly1305);
    let request = request(Some(&previous), Some(&master), false, &cipher);
    let outcome = migrate_if_requested(&request, || Ok(Some(&storage)), log_into(&events), Ok)
        .await
        .expect("the run succeeds")
        .expect("an outcome");
    assert!(matches!(
        outcome,
        MigrationOutcome::Nothing(NothingToMigrate::NothingLeftToMigrate)
    ));
    assert_eq!(storage.replaced().len(), 1, "one attempt, no retry");
}

#[tokio::test]
async fn remaining_count_no_retry() {
    let events = events();
    let storage = FakeStorage::new(&events, vec![credentials()]);
    let legacy = sealed("hunter2", PREVIOUS_KEY, Algorithm::XSalsa20Poly1305);
    storage.script_rows(vec![legacy_row(&legacy)]);
    storage.script_rows(vec![legacy_row(&legacy)]);
    storage.script_rows(vec![legacy_row(&legacy)]);
    storage.script_update(true);
    let (previous, master) = keys();
    let cipher = cipher(Algorithm::XSalsa20Poly1305);
    let request = request(Some(&previous), Some(&master), false, &cipher);
    let outcome = migrate_if_requested(&request, || Ok(Some(&storage)), log_into(&events), Ok)
        .await
        .expect("the run succeeds")
        .expect("an outcome");
    assert!(matches!(
        outcome,
        MigrationOutcome::Migrated {
            migrated: 1,
            remaining: 1
        }
    ));
    assert_eq!(storage.replaced().len(), 1, "no automatic retry");
    let logged = recorded(&events);
    assert!(
        logged
            .iter()
            .any(|event| event.contains("Keep LITELLM_MIGRATE_FROM_MASTER_KEY set and restart")),
        "{logged:?}"
    );
}

#[tokio::test]
async fn progress_then_outcome_log() {
    let events = events();
    let storage = FakeStorage::new(&events, vec![credentials()]);
    let legacy = sealed("hunter2", PREVIOUS_KEY, Algorithm::XSalsa20Poly1305);
    storage.script_rows(vec![legacy_row(&legacy)]);
    storage.script_rows(vec![legacy_row(&legacy)]);
    storage.script_rows(Vec::new());
    storage.script_update(true);
    let (previous, master) = keys();
    let cipher = cipher(Algorithm::XSalsa20Poly1305);
    let request = request(Some(&previous), Some(&master), false, &cipher);
    drop(migrate_if_requested(&request, || Ok(Some(&storage)), log_into(&events), Ok).await);
    let logs: Vec<String> = recorded(&events)
        .into_iter()
        .filter(|event| event.starts_with("log:"))
        .collect();
    assert_eq!(logs.len(), 2, "{logs:?}");
    assert_eq!(
        logs[0],
        "log:Re-encrypting 1 stored value(s) from the LITELLM_MIGRATE_FROM_MASTER_KEY key to the new master key."
    );
    assert!(
        logs[1].starts_with("log:Done re-encrypting 1 stored value(s) with the new master key.")
    );
}

#[tokio::test]
async fn outcome_before_failure_policy() {
    let events = events();
    let storage = FakeStorage::new(&events, vec![credentials()]);
    storage.script_unavailable();
    let (previous, master) = keys();
    let cipher = cipher(Algorithm::XSalsa20Poly1305);
    let request = request(Some(&previous), Some(&master), false, &cipher);
    let policy_events = Arc::clone(&events);
    let seen = Mutex::new(None);
    let outcome = migrate_if_requested(
        &request,
        || Ok(Some(&storage)),
        log_into(&events),
        |failure: MigrationFailure| {
            policy_events
                .lock()
                .expect("the events lock")
                .push("policy".to_owned());
            *seen.lock().expect("the seen lock") = Some(failure.message.clone());
            Ok(failure)
        },
    )
    .await
    .expect("a tolerated failure still returns an outcome")
    .expect("an outcome");
    assert!(matches!(outcome, MigrationOutcome::Failed(_)));
    assert_eq!(
        steps(&recorded(&events)),
        vec!["columns", "rows", "outcome", "policy"]
    );
    assert_eq!(
        seen.lock().expect("the seen lock").as_deref(),
        Some("the database connection is unavailable")
    );
}

#[tokio::test]
async fn tolerated_failure_returned() {
    let events = events();
    let storage = FakeStorage::new(&events, vec![credentials()]);
    storage.script_unavailable();
    let (previous, master) = keys();
    let cipher = cipher(Algorithm::XSalsa20Poly1305);
    let request = request(Some(&previous), Some(&master), false, &cipher);
    let outcome = migrate_if_requested(&request, || Ok(Some(&storage)), log_into(&events), Ok)
        .await
        .expect("the policy tolerates the failure")
        .expect("an outcome");
    let MigrationOutcome::Failed(failure) = outcome else {
        panic!("the run failed")
    };
    assert_eq!(failure.class, "ConnectError");
    assert_eq!(failure.message, "the database connection is unavailable");
}

#[tokio::test]
async fn rejected_failure_propagates_the_original_cause() {
    let events = events();
    let storage = FakeStorage::new(&events, vec![credentials()]);
    storage.script_unavailable();
    let (previous, master) = keys();
    let cipher = cipher(Algorithm::XSalsa20Poly1305);
    let request = request(Some(&previous), Some(&master), false, &cipher);
    let seen = Mutex::new(String::new());
    let policy = |failure: MigrationFailure| {
        *seen.lock().expect("the seen lock") = format!("{failure:?}");
        Err(failure)
    };
    let returned = migrate_if_requested(&request, || Ok(Some(&storage)), log_into(&events), policy)
        .await
        .expect_err("a rejected failure escapes");
    assert!(matches!(returned.cause, MigrationError::Connect(_)));
    assert_eq!(
        format!("{returned:?}"),
        *seen.lock().expect("the seen lock"),
        "the policy hands back the very same cause"
    );
}

#[tokio::test]
async fn cancellation_not_captured() {
    let events = events();
    let gate = Arc::new(Notify::new());
    let storage = FakeStorage::new(&events, vec![credentials()]).block_on_read(Arc::clone(&gate));
    let mut tasks = JoinSet::new();
    let task_events = Arc::clone(&events);
    tasks.spawn(async move {
        let (previous, master) = keys();
        let cipher = cipher(Algorithm::XSalsa20Poly1305);
        let request = request(Some(&previous), Some(&master), false, &cipher);
        migrate_if_requested(&request, || Ok(Some(&storage)), log_into(&task_events), Ok).await
    });
    let started = wait_for_read(&events).await;
    assert!(started, "the migration blocks inside the read");
    tasks.abort_all();
    let joined = tasks
        .join_next()
        .await
        .expect("the task was spawned")
        .expect_err("cancellation propagates");
    assert!(joined.is_cancelled());
    let logged = recorded(&events);
    assert!(
        !logged.iter().any(|event| event.starts_with("log:")),
        "a cancelled run has no outcome: {logged:?}"
    );
}

async fn wait_for_read(events: &Events) -> bool {
    for _ in 0_u16..10_000_u16 {
        if recorded(events)
            .iter()
            .any(|event| event.starts_with("rows:"))
        {
            return true;
        }
        yield_now().await;
    }
    false
}

#[tokio::test]
async fn callback_and_final_log_errors_escape() {
    let events = events();
    let storage = FakeStorage::new(&events, Vec::new());
    let (previous, master) = keys();
    let cipher = cipher(Algorithm::XSalsa20Poly1305);
    let request = request(Some(&previous), Some(&master), false, &cipher);
    let connect_error = migrate_if_requested(
        &request,
        || {
            Err::<Option<&FakeStorage>, _>(MigrationFailure::from_cause(
                MigrationError::Connect(PoolFailure {
                    driver: PoolError::Closed,
                }),
                &[],
            ))
        },
        log_into(&events),
        Ok,
    )
    .await
    .expect_err("a connect failure escapes");
    assert_eq!(
        connect_error.message,
        "the database connection is unavailable"
    );
    assert!(recorded(&events).is_empty());

    let log_error = migrate_if_requested(
        &request,
        || Ok(Some(&storage)),
        |_| Err(MigrationError::Logger(io::Error::other("the logger broke"))),
        |_| panic!("the policy never sees a log failure"),
    )
    .await
    .expect_err("a final log failure escapes");
    assert!(
        log_error.message.contains("the logger broke"),
        "{}",
        log_error.message
    );
}

#[tokio::test]
async fn progress_log_error_captured() {
    let events = events();
    let storage = FakeStorage::new(&events, vec![credentials()]);
    let legacy = sealed("hunter2", PREVIOUS_KEY, Algorithm::XSalsa20Poly1305);
    storage.script_rows(vec![legacy_row(&legacy)]);
    let (previous, master) = keys();
    let cipher = cipher(Algorithm::XSalsa20Poly1305);
    let request = request(Some(&previous), Some(&master), false, &cipher);
    let captured = Arc::clone(&events);
    let seen = Mutex::new(None);
    let log = move |message: &str| {
        if message.starts_with("Re-encrypting") {
            return Err(MigrationError::Logger(io::Error::other(
                "the progress logger broke",
            )));
        }
        captured
            .lock()
            .expect("the events lock")
            .push(format!("log:{message}"));
        Ok(())
    };
    let policy = |failure: MigrationFailure| {
        *seen.lock().expect("the seen lock") = Some(failure.message.clone());
        Ok(failure)
    };
    let outcome = migrate_if_requested(&request, || Ok(Some(&storage)), log, policy)
        .await
        .expect("the policy tolerates the failure")
        .expect("an outcome");
    assert!(matches!(outcome, MigrationOutcome::Failed(_)));
    assert_eq!(
        seen.lock().expect("the seen lock").as_deref(),
        Some("the migration logger failed: the progress logger broke")
    );
    let logged = recorded(&events);
    assert!(
        logged
            .iter()
            .any(|event| event.starts_with("log:Could not migrate"))
    );
}
