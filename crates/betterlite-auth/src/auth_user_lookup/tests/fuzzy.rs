use std::collections::BTreeMap;
use std::sync::atomic::Ordering;
use std::time::Duration;

use betterlite_db::budget_permission_rows::JsonData;
use tokio::time::timeout;

use super::logs::Logs;
use super::support::{
    Call, FailureKind, Harness, harness_without_store, model, request, request_with, user_with,
};
use crate::auth_user_lookup::error::UserLookupError;

#[tokio::test]
async fn sso_before_email() {
    let harness = Harness::default();
    *harness.store.by_sso.lock().unwrap() = Some(user_with("canonical", None, Some("sso-1")));
    *harness.store.by_email.lock().unwrap() = Some(user_with("email-row", None, None));
    let lookup = harness.lookup();
    let user = lookup
        .get_user_object(request_with(
            Some("requested"),
            Some("sso-1"),
            Some("a@example.com"),
        ))
        .await
        .expect("the sso lookup")
        .expect("the sso row");
    assert_eq!(user.user_id, "canonical");
    assert_eq!(
        harness.store.methods(),
        vec!["find_by_id", "find_by_sso_id", "backfill", "reread"]
    );
}

#[tokio::test]
async fn empty_fuzzy_values_preserved() {
    let harness = Harness::default();
    let lookup = harness.lookup();
    let result = lookup
        .get_user_object(request_with(Some("u1"), Some(""), Some("")))
        .await;
    assert!(matches!(result, Err(UserLookupError::NotFound(_))));
    assert_eq!(
        harness.store.calls.lock().unwrap().clone(),
        vec![
            Call::FindById {
                user_id: "u1".to_owned(),
                use_writer: false,
            },
            Call::FindBySso {
                sso_user_id: String::new(),
            },
            Call::FindByEmail {
                email: String::new(),
            },
        ]
    );
}

#[tokio::test]
async fn sso_update_overwrites_in_background() {
    let harness = Harness::default();
    let stored = user_with("canonical", Some("stored@example.com"), Some("old-sso"));
    *harness.store.by_email.lock().unwrap() = Some(stored);
    harness.store.block_update.store(true, Ordering::SeqCst);
    let lookup = harness.lookup();
    let user = lookup
        .get_user_object(request_with(
            Some("requested"),
            Some("new-sso"),
            Some("a@example.com"),
        ))
        .await
        .expect("the email lookup")
        .expect("the matched user");
    assert_eq!(user.user_id, "canonical");
    timeout(
        Duration::from_secs(1),
        harness.store.update_started.notified(),
    )
    .await
    .expect("the background write starts");
    let calls = harness.store.calls.lock().unwrap().clone();
    match calls.as_slice() {
        [
            Call::FindById { .. },
            Call::FindBySso { .. },
            Call::FindByEmail { .. },
            Call::UpdateSso {
                user_id,
                sso_user_id,
            },
        ] => {
            assert_eq!(user_id, "canonical");
            assert_eq!(sso_user_id, "new-sso");
        }
        other => panic!("the matched row's sso id is rewritten: {other:?}"),
    }
    harness.store.update_release.notify_one();
    lookup.tasks.close();
    lookup.tasks.wait().await;
}

#[tokio::test]
async fn fuzzy_never_creates_user() {
    let harness = Harness::default();
    *harness.store.by_email.lock().unwrap() =
        Some(user_with("matched", Some("a@example.com"), None));
    let lookup = harness.lookup();
    let user = lookup
        .get_user_object(request_with(Some("requested"), None, Some("A@EXAMPLE.COM")))
        .await
        .expect("the fuzzy read")
        .expect("the matched user");
    assert_eq!(user.user_id, "matched");
    assert!(harness.store.create_fields().is_none());
}

#[tokio::test]
async fn backfill_skip_without_input_or_db() {
    let without_input = Harness::default();
    *without_input.store.by_id.lock().unwrap() = Some(user_with("u1", None, None));
    let lookup = without_input.lookup();
    let user = lookup
        .get_user_object(request(Some("u1")))
        .await
        .expect("the stored user")
        .expect("the stored user is present");
    assert_eq!(user.user_email, None);
    assert_eq!(without_input.store.methods(), vec!["find_by_id"]);

    let without_db = harness_without_store();
    *without_db.cache.cached.lock().unwrap() = Some(model(&user_with("u1", None, None)));
    let cached_lookup = without_db.lookup();
    let cached = cached_lookup
        .get_user_object(request_with(Some("u1"), None, Some("a@example.com")))
        .await
        .expect("the cached user")
        .expect("the cached user is present");
    assert_eq!(cached.user_email, None);
    assert!(without_db.cache.writes.lock().unwrap().is_empty());
}

#[tokio::test]
async fn backfill_empty_input_allowed() {
    let harness = Harness::default();
    *harness.store.by_id.lock().unwrap() = Some(user_with("u1", None, None));
    let lookup = harness.lookup();
    let user = lookup
        .get_user_object(request_with(Some("u1"), None, Some("")))
        .await
        .expect("the stored user")
        .expect("the stored user is present");
    assert_eq!(user.user_email, None);
    assert_eq!(
        harness.store.calls.lock().unwrap().clone(),
        vec![
            Call::FindById {
                user_id: "u1".to_owned(),
                use_writer: false,
            },
            Call::Backfill {
                user_id: "u1".to_owned(),
                email: String::new(),
            },
            Call::Reread {
                user_id: "u1".to_owned(),
            },
        ]
    );
}

#[tokio::test]
async fn backfill_missing_reread_keeps_original() {
    let harness = Harness::default();
    *harness.store.by_id.lock().unwrap() = Some(user_with("u1", None, None));
    let lookup = harness.lookup();
    let user = lookup
        .get_user_object(request_with(Some("u1"), None, Some("input@example.com")))
        .await
        .expect("the stored user")
        .expect("the stored user is present");
    assert_eq!(user.user_email, None);
    assert_eq!(harness.cache.write_keys(), vec!["u1"]);
}

#[tokio::test]
async fn backfill_uses_reread_email_only() {
    let harness = Harness::default();
    *harness.store.by_id.lock().unwrap() = Some(JsonData::Object(BTreeMap::from([
        ("user_id".to_owned(), JsonData::String("u1".to_owned())),
        (
            "user_alias".to_owned(),
            JsonData::String("original".to_owned()),
        ),
    ])));
    let JsonData::Object(mut reread) = user_with("u1", Some("db@example.com"), None) else {
        panic!("a user row is a mapping");
    };
    reread.insert(
        "user_alias".to_owned(),
        JsonData::String("changed".to_owned()),
    );
    reread.insert(
        "user_role".to_owned(),
        JsonData::String("proxy_admin".to_owned()),
    );
    *harness.store.reread.lock().unwrap() = Some(model(&JsonData::Object(reread)));
    let lookup = harness.lookup();
    let user = lookup
        .get_user_object(request_with(Some("u1"), None, Some("input@example.com")))
        .await
        .expect("the stored user")
        .expect("the stored user is present");
    assert_eq!(user.user_email.as_deref(), Some("db@example.com"));
    assert_eq!(user.user_alias.as_deref(), Some("original"));
    assert_eq!(user.user_role, None);
    let writes = harness.cache.writes.lock().unwrap().clone();
    assert_eq!(writes.len(), 2);
    assert_eq!(writes[0].0, "u1");
    assert_eq!(writes[0].1.user_email.as_deref(), Some("db@example.com"));
}

#[tokio::test]
async fn canonical_then_requested_cache_keys() {
    let harness = Harness::default();
    *harness.store.by_sso.lock().unwrap() = Some(user_with("canon", None, Some("sso-1")));
    *harness.store.reread.lock().unwrap() = Some(model(&user_with(
        "canon",
        Some("db@example.com"),
        Some("sso-1"),
    )));
    let lookup = harness.lookup();
    let user = lookup
        .get_user_object(request_with(
            Some("requested"),
            Some("sso-1"),
            Some("input@example.com"),
        ))
        .await
        .expect("the fuzzy read")
        .expect("the matched user");
    assert_eq!(user.user_id, "canon");
    assert_eq!(user.user_email.as_deref(), Some("db@example.com"));
    assert_eq!(harness.cache.write_keys(), vec!["canon", "requested"]);
    let writes = harness.cache.writes.lock().unwrap().clone();
    assert!(
        writes
            .iter()
            .all(|(_, _, ttl)| ttl.to_bits() == 60.0_f64.to_bits())
    );
}

#[tokio::test]
async fn background_failure_does_not_fail_lookup() {
    let harness = Harness::default();
    *harness.store.by_email.lock().unwrap() =
        Some(user_with("canon", Some("stored@example.com"), None));
    harness.store.failures.lock().unwrap().insert(
        "update_sso",
        FailureKind::Named(
            "update failed\nTraceback (most recent call last):\n  secret-internal".to_owned(),
        ),
    );
    let logs = Logs::default();
    let _guard = logs.install();
    let lookup = harness.lookup();
    let user = lookup
        .get_user_object(request_with(
            Some("requested"),
            Some("new-sso"),
            Some("a@example.com"),
        ))
        .await
        .expect("the email lookup")
        .expect("the matched user");
    assert_eq!(user.user_id, "canon");
    lookup.tasks.close();
    lookup.tasks.wait().await;
    let warnings = logs.warnings();
    assert_eq!(warnings.len(), 1);
    assert!(
        warnings[0]
            .message
            .contains("background SSO user id update failed")
    );
    let cause = warnings[0].fields.get("cause").cloned().unwrap_or_default();
    assert!(cause.contains("update failed"), "{cause}");
    assert!(!cause.contains("Traceback"), "{cause}");
    assert!(!cause.contains("secret-internal"), "{cause}");
}

#[tokio::test]
async fn background_task_drained() {
    let harness = Harness::default();
    *harness.store.by_email.lock().unwrap() =
        Some(user_with("canon", Some("stored@example.com"), None));
    let lookup = harness.lookup();
    let user = lookup
        .get_user_object(request_with(
            Some("requested"),
            Some("new-sso"),
            Some("a@example.com"),
        ))
        .await
        .expect("the email lookup")
        .expect("the matched user");
    assert_eq!(user.user_id, "canon");
    lookup.tasks.close();
    lookup.tasks.wait().await;
    let methods = harness.store.methods();
    assert_eq!(
        methods
            .iter()
            .filter(|method| method.as_str() == "update_sso")
            .count(),
        1
    );
}
