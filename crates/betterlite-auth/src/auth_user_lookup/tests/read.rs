use std::sync::atomic::Ordering;
use std::time::Duration;

use super::logs::Logs;
use super::support::{Call, FailureKind, Harness, model, user_with};
use crate::auth_identity::errors::AuthError;
use crate::auth_user_lookup::error::UserLookupError;
use crate::auth_user_lookup::lookup::UserLookupRequest;

#[tokio::test]
async fn none_id_skips_lookup() {
    let harness = Harness::default();
    let lookup = harness.lookup();
    let found = lookup
        .get_user_object(UserLookupRequest::default())
        .await
        .expect("a missing id is not a lookup");
    assert!(found.is_none());
    assert!(harness.store.calls.lock().unwrap().is_empty());
    assert!(harness.cache.reads.lock().unwrap().is_empty());
}

#[tokio::test]
async fn empty_id_lookup() {
    let harness = Harness::default();
    let lookup = harness.lookup();
    let result = lookup
        .get_user_object(UserLookupRequest {
            user_id: Some(""),
            ..UserLookupRequest::default()
        })
        .await;
    match result {
        Err(UserLookupError::NotFound(AuthError::UserNotFound { user_id })) => {
            assert_eq!(user_id, "");
        }
        other => panic!("an empty id is still a lookup: {other:?}"),
    }
    assert_eq!(harness.cache.reads.lock().unwrap().clone(), vec![""]);
    assert_eq!(
        harness.store.calls.lock().unwrap().clone(),
        vec![Call::FindById {
            user_id: String::new(),
            use_writer: false,
        }]
    );
}

#[tokio::test]
async fn typed_hit_without_database() {
    let harness = Harness {
        with_store: false,
        ..Harness::default()
    };
    *harness.cache.cached.lock().unwrap() =
        Some(model(&user_with("u1", Some("a@example.com"), None)));
    let lookup = harness.lookup();
    let user = lookup
        .get_user_object(UserLookupRequest {
            user_id: Some("u1"),
            ..UserLookupRequest::default()
        })
        .await
        .expect("the cached user")
        .expect("the cached user is present");
    assert_eq!(user.user_id, "u1");
    assert_eq!(user.user_email.as_deref(), Some("a@example.com"));
    assert!(harness.store.calls.lock().unwrap().is_empty());
    assert_eq!(harness.cache.reads.lock().unwrap().clone(), vec!["u1"]);
}

#[tokio::test]
async fn hit_backfill_error_outside_catch() {
    let harness = Harness::default();
    *harness.cache.cached.lock().unwrap() = Some(model(&user_with("u1", None, None)));
    harness
        .store
        .failures
        .lock()
        .unwrap()
        .insert("backfill", FailureKind::Io("connection reset".to_owned()));
    let logs = Logs::default();
    let _guard = logs.install();
    let lookup = harness.lookup();
    let result = lookup
        .get_user_object(UserLookupRequest {
            user_id: Some("u1"),
            user_email: Some("a@example.com"),
            ..UserLookupRequest::default()
        })
        .await;
    match result {
        Err(UserLookupError::PreLookup(error)) => {
            assert!(error.to_string().contains("connection reset"), "{error}")
        }
        other => panic!("a cache-hit backfill failure is not wrapped: {other:?}"),
    }
    assert!(logs.entries().is_empty());
}

#[tokio::test]
async fn cache_error_outside_catch() {
    let harness = Harness::default();
    harness.cache.read_fails.store(true, Ordering::SeqCst);
    let logs = Logs::default();
    let _guard = logs.install();
    let lookup = harness.lookup();
    let result = lookup
        .get_user_object(UserLookupRequest {
            user_id: Some("u1"),
            ..UserLookupRequest::default()
        })
        .await;
    match result {
        Err(UserLookupError::PreLookup(error)) => {
            assert_eq!(error.to_string(), "the cached user could not be read");
        }
        other => panic!("a cache read failure is not wrapped: {other:?}"),
    }
    assert!(harness.store.calls.lock().unwrap().is_empty());
    assert!(logs.entries().is_empty());
}

#[tokio::test]
async fn missing_db_outside_catch() {
    let harness = Harness {
        with_store: false,
        ..Harness::default()
    };
    let logs = Logs::default();
    let _guard = logs.install();
    let lookup = harness.lookup();
    let result = lookup
        .get_user_object(UserLookupRequest {
            user_id: Some("u1"),
            ..UserLookupRequest::default()
        })
        .await;
    let rendered = result.as_ref().err().map(ToString::to_string);
    assert_eq!(
        rendered.as_deref(),
        Some("No DB Connected. See - https://docs.litellm.ai/docs/proxy/virtual_keys")
    );
    assert!(logs.entries().is_empty());
}

#[tokio::test]
async fn db_only_skips_cache_and_cadence() {
    let harness = Harness::default();
    harness.clock.set(Duration::from_secs(100));
    harness.cadence.update("user_id:u1", None);
    harness.clock.set(Duration::from_secs(101));
    *harness.store.by_id.lock().unwrap() = Some(user_with("u1", Some("a@example.com"), None));
    let lookup = harness.lookup();
    let user = lookup
        .get_user_object(UserLookupRequest {
            user_id: Some("u1"),
            check_db_only: Some(true),
            ..UserLookupRequest::default()
        })
        .await
        .expect("the forced read")
        .expect("the stored user");
    assert_eq!(user.user_id, "u1");
    assert!(harness.cache.reads.lock().unwrap().is_empty());
    assert_eq!(
        harness.store.calls.lock().unwrap().clone(),
        vec![Call::FindById {
            user_id: "u1".to_owned(),
            use_writer: true,
        }]
    );
}

#[tokio::test]
async fn primary_writer_only_when_requested() {
    for (check_db_only, expected_writer) in
        [(None, false), (Some(false), false), (Some(true), true)]
    {
        let harness = Harness::default();
        *harness.store.by_id.lock().unwrap() = Some(user_with("u1", None, None));
        let lookup = harness.lookup();
        let user = lookup
            .get_user_object(UserLookupRequest {
                user_id: Some("u1"),
                check_db_only,
                ..UserLookupRequest::default()
            })
            .await
            .expect("the primary read")
            .expect("the stored user");
        assert_eq!(user.user_id, "u1");
        let calls = harness.store.calls.lock().unwrap().clone();
        match calls.as_slice() {
            [Call::FindById { use_writer, .. }] => {
                assert_eq!(
                    *use_writer, expected_writer,
                    "check_db_only {check_db_only:?}"
                );
            }
            other => panic!("expected one primary read, got {other:?}"),
        }
    }
}

#[tokio::test]
async fn fuzzy_still_normal_routing() {
    let harness = Harness::default();
    *harness.store.by_sso.lock().unwrap() = Some(user_with("canonical", None, Some("sso-1")));
    let lookup = harness.lookup();
    let user = lookup
        .get_user_object(UserLookupRequest {
            user_id: Some("requested"),
            sso_user_id: Some("sso-1"),
            check_db_only: Some(true),
            ..UserLookupRequest::default()
        })
        .await
        .expect("the fuzzy read")
        .expect("the matched user");
    assert_eq!(user.user_id, "canonical");
    let calls = harness.store.calls.lock().unwrap().clone();
    match calls.as_slice() {
        [
            Call::FindById {
                use_writer: true, ..
            },
            Call::FindBySso { sso_user_id },
        ] => assert_eq!(sso_user_id, "sso-1"),
        other => panic!("the primary read forces the writer, the fuzzy read does not: {other:?}"),
    }
}

#[tokio::test]
async fn suppressed_miss_skips_primary_and_fuzzy() {
    let harness = Harness::default();
    harness.clock.set(Duration::from_secs(100));
    harness.cadence.update("user_id:u1", None);
    harness.clock.set(Duration::from_secs(102));
    *harness.store.by_sso.lock().unwrap() = Some(user_with("fuzzy-row", None, Some("sso-1")));
    let lookup = harness.lookup();
    let result = lookup
        .get_user_object(UserLookupRequest {
            user_id: Some("u1"),
            sso_user_id: Some("sso-1"),
            user_email: Some("a@example.com"),
            ..UserLookupRequest::default()
        })
        .await;
    match result {
        Err(UserLookupError::NotFound(_)) => {}
        other => panic!("a suppressed miss stays suppressed: {other:?}"),
    }
    assert!(harness.store.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn negative_not_extended() {
    let harness = Harness::default();
    harness.clock.set(Duration::from_secs(100));
    harness.cadence.update("user_id:u1", None);
    harness.clock.set(Duration::from_secs(102));
    let lookup = harness.lookup();
    let result = lookup
        .get_user_object(UserLookupRequest {
            user_id: Some("u1"),
            ..UserLookupRequest::default()
        })
        .await;
    assert!(matches!(result, Err(UserLookupError::NotFound(_))));
    let entry = harness.cadence.entry("user_id:u1").unwrap();
    assert_eq!(entry.checked_at, Duration::from_secs(100));
    assert_eq!(entry.record, None);
}

#[tokio::test]
async fn passed_expiry_sets_window() {
    let harness = Harness::new(7);
    let lookup = harness.lookup();
    let first = lookup
        .get_user_object(UserLookupRequest {
            user_id: Some("u1"),
            ..UserLookupRequest::default()
        })
        .await;
    assert!(matches!(first, Err(UserLookupError::NotFound(_))));
    assert_eq!(harness.store.calls.lock().unwrap().len(), 1);
    harness.clock.set(Duration::from_secs(3));
    let second = lookup
        .get_user_object(UserLookupRequest {
            user_id: Some("u1"),
            ..UserLookupRequest::default()
        })
        .await;
    assert!(matches!(second, Err(UserLookupError::NotFound(_))));
    assert_eq!(
        harness.store.calls.lock().unwrap().len(),
        1,
        "the negative window suppresses the primary read"
    );
    harness.clock.set(Duration::from_secs(7));
    let third = lookup
        .get_user_object(UserLookupRequest {
            user_id: Some("u1"),
            ..UserLookupRequest::default()
        })
        .await;
    assert!(matches!(third, Err(UserLookupError::NotFound(_))));
    assert_eq!(
        harness.store.calls.lock().unwrap().len(),
        2,
        "the window expires at exactly the expiry"
    );
    assert_eq!(
        harness.cadence.entry("user_id:u1").unwrap().checked_at,
        Duration::from_secs(7)
    );
}

#[tokio::test]
async fn actual_absence_populates_negative() {
    let harness = Harness::default();
    let lookup = harness.lookup();
    let result = lookup
        .get_user_object(UserLookupRequest {
            user_id: Some("u1"),
            ..UserLookupRequest::default()
        })
        .await;
    assert!(matches!(result, Err(UserLookupError::NotFound(_))));
    let entry = harness.cadence.entry("user_id:u1").unwrap();
    assert_eq!(entry.record, None);
    assert_eq!(entry.checked_at, Duration::ZERO);
    assert_eq!(harness.store.calls.lock().unwrap().len(), 1);
    assert!(harness.cache.writes.lock().unwrap().is_empty());
}

#[tokio::test]
async fn no_negative_on_read_failure() {
    let harness = Harness::default();
    harness
        .store
        .failures
        .lock()
        .unwrap()
        .insert("find_by_id", FailureKind::Named("boom".to_owned()));
    let lookup = harness.lookup();
    let result = lookup
        .get_user_object(UserLookupRequest {
            user_id: Some("u1"),
            ..UserLookupRequest::default()
        })
        .await;
    match result {
        Err(UserLookupError::UserMissing(_)) => {}
        other => panic!("an ordinary read failure is rewritten: {other:?}"),
    }
    assert_eq!(harness.cadence.entry("user_id:u1"), None);
}
