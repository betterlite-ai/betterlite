use betterlite_models::decisions::Json;

use super::super::access::{PollAccess, PollDenial};
use super::super::access::{PollCaller, PollOwner};
use super::super::get_cache_key;
use super::super::store::StoredPollState;
use super::super::store::{NewPollRequest, ResponsePollingStore};
use super::support::{self, CacheOperation, FakeCache};

const ID: &str = "litellm_poll_owner";
const ENVELOPE: &str = concat!(
    r#"{"owner":{"user_id":"a","team_id":"t","key_fingerprint":"k"},"#,
    r#""response":{"id":"litellm_poll_owner","object":"response","#,
    r#""status":"in_progress","output":[]}}"#
);
const FINGERPRINT_ENVELOPE: &str = concat!(
    r#"{"owner":{"user_id":"","team_id":"","key_fingerprint":"k"},"#,
    r#""response":{"id":"litellm_poll_owner","status":"queued","output":[]}}"#
);
const EMPTY_FINGERPRINT_ENVELOPE: &str = concat!(
    r#"{"owner":{"user_id":"","team_id":"","key_fingerprint":""},"#,
    r#""response":{"id":"litellm_poll_owner","status":"queued","output":[]}}"#
);
const ANONYMOUS: &str = concat!(
    r#"{"owner":null,"response":{"id":"litellm_poll_anonymous","#,
    r#""status":"queued","output":[]}}"#
);

pub(super) fn identity_derivation() {
    let principal = support::principal(Some("a"), Some("t"), &["platform_admin"]);
    let owner = PollOwner::from_principal(&principal, Some("k"));
    assert_eq!(owner, support::owner("a", "t", "k"));
    let caller = PollCaller::from_principal(&principal, Some("k"));
    assert_eq!(
        caller,
        support::caller(Some("a"), Some("t"), true, Some("k"))
    );
    let anonymous = support::principal(None, None, &[]);
    assert_eq!(
        PollOwner::from_principal(&anonymous, None),
        PollOwner::default()
    );
    assert_eq!(
        PollCaller::from_principal(&anonymous, None),
        PollCaller::default()
    );
}

pub(super) async fn owner_retention(cache: &FakeCache, store: &ResponsePollingStore) {
    create_owned(cache, store).await;
    let caller = support::caller(Some("a"), Some("t"), false, None);
    let access = support::access(&caller);
    assert_owner_update(cache, store, access).await;
    assert_owner_read(store, access).await;
    assert_owner_cancel(cache, store, access).await;
    assert_owner_delete(cache, store).await;
}

async fn create_owned(cache: &FakeCache, store: &ResponsePollingStore) {
    let request = NewPollRequest {
        polling_id: ID,
        created_at: 1_700_000_000,
        metadata: None,
        owner: Some(support::owner("a", "t", "k")),
    };
    let created = store
        .create_initial_state(request)
        .await
        .expect("the owned record is written");
    let envelope = support::stored(cache, ID);
    assert_eq!(support::field(&envelope, "owner"), owned());
    assert!(!support::has_key(&envelope, "id"));
    assert_eq!(support::field(&envelope, "response"), Json::Object(created));
    let StoredPollState::Record(record) = support::stored_state(cache, ID) else {
        panic!("the owner envelope decodes as a record");
    };
    assert_eq!(record.owner, Some(support::owner("a", "t", "k")));
}

async fn assert_owner_update(
    cache: &FakeCache,
    store: &ResponsePollingStore,
    access: PollAccess<'_>,
) {
    let _update = store
        .update_state(ID, access, &support::patch(r#"{"status":"completed"}"#))
        .await
        .expect("the owner may patch the state");
    let envelope = support::stored(cache, ID);
    assert_eq!(support::field(&envelope, "owner"), owned());
}

async fn assert_owner_read(store: &ResponsePollingStore, access: PollAccess<'_>) {
    let public = store
        .get_state(ID, access)
        .await
        .expect("the owner may read the state")
        .expect("the key exists");
    assert!(public.contains_key("id"));
    assert!(!public.contains_key("owner"));
}

async fn assert_owner_cancel(
    cache: &FakeCache,
    store: &ResponsePollingStore,
    access: PollAccess<'_>,
) {
    let cancelled = store
        .cancel_polling(ID, access)
        .await
        .expect("the owner may cancel");
    assert!(cancelled);
    let envelope = support::stored(cache, ID);
    assert_eq!(support::field(&envelope, "owner"), owned());
    let response = support::field(&envelope, "response");
    assert_eq!(
        support::field(&response, "status"),
        support::json("\"cancelled\"")
    );
    let effects = cache.effects();
    let last = effects.last().expect("the cancel wrote");
    assert_eq!(last.operation, CacheOperation::Write);
    assert_eq!(last.ttl, Some(3600));
}

async fn assert_owner_delete(cache: &FakeCache, store: &ResponsePollingStore) {
    let deleted = store
        .delete_polling(ID)
        .await
        .expect("the record is deleted");
    assert!(deleted);
    assert!(cache.entry(&get_cache_key(ID)).is_none());
}

pub(super) async fn identity_denials(cache: &FakeCache, store: &ResponsePollingStore) {
    let writes = cache.count(CacheOperation::Write);
    cache.seed(&get_cache_key(ID), ENVELOPE);
    assert_auth_disabled(store).await;
    assert_user_denials(store).await;
    assert_team_denials(store).await;
    assert_admin_reads(store).await;
    assert_eq!(cache.count(CacheOperation::Write), writes);
}

async fn assert_auth_disabled(store: &ResponsePollingStore) {
    let state = store
        .get_state(ID, PollAccess::AuthDisabled)
        .await
        .expect("auth-disabled reads everything");
    assert!(state.is_some());
}

async fn assert_user_denials(store: &ResponsePollingStore) {
    let other = support::caller(Some("b"), Some("t"), false, None);
    support::assert_denial(
        store
            .get_state(ID, support::access(&other))
            .await
            .expect_err("another user is denied"),
        PollDenial::User,
    );
    let no_user = support::caller(None, Some("t"), false, None);
    support::assert_denial(
        store
            .get_state(ID, support::access(&no_user))
            .await
            .expect_err("a caller without the record's user is denied"),
        PollDenial::User,
    );
}

async fn assert_team_denials(store: &ResponsePollingStore) {
    let other_team = support::caller(Some("a"), Some("x"), false, None);
    support::assert_denial(
        store
            .update_state(
                ID,
                support::access(&other_team),
                &support::patch(r#"{"status":"completed"}"#),
            )
            .await
            .expect_err("another team is denied"),
        PollDenial::Team,
    );
    let no_team = support::caller(Some("a"), None, false, None);
    support::assert_denial(
        store
            .get_state(ID, support::access(&no_team))
            .await
            .expect_err("a caller without the record's team is denied"),
        PollDenial::Team,
    );
}

async fn assert_admin_reads(store: &ResponsePollingStore) {
    let admin = support::caller(Some("root"), Some("x"), true, None);
    let state = store
        .get_state(ID, support::access(&admin))
        .await
        .expect("the proxy admin reads everything");
    assert!(state.is_some());
}

pub(super) async fn fingerprint_denials(cache: &FakeCache, store: &ResponsePollingStore) {
    cache.seed(&get_cache_key(ID), FINGERPRINT_ENVELOPE);
    let wrong = support::caller(None, None, false, Some("other"));
    support::assert_denial(
        store
            .get_state(ID, support::access(&wrong))
            .await
            .expect_err("a different key is denied"),
        PollDenial::KeyFingerprint,
    );
    let right = support::caller(None, None, false, Some("k"));
    assert!(
        store
            .get_state(ID, support::access(&right))
            .await
            .expect("the same key reads the state")
            .is_some()
    );
    let absent = support::caller(None, None, false, None);
    support::assert_denial(
        store
            .get_state(ID, support::access(&absent))
            .await
            .expect_err("an absent key is denied"),
        PollDenial::KeyFingerprint,
    );
    cache.seed(&get_cache_key(ID), EMPTY_FINGERPRINT_ENVELOPE);
    assert!(
        store
            .get_state(ID, support::access(&absent))
            .await
            .expect("two absent keys are equal")
            .is_some()
    );
}

pub(super) async fn record_shape_denials(cache: &FakeCache, store: &ResponsePollingStore) {
    let writes = cache.count(CacheOperation::Write);
    let caller = support::caller(Some("a"), Some("t"), false, None);
    assert_ownerless_denied(cache, store, &caller).await;
    assert_anonymous_denied(cache, store, &caller).await;
    assert_eq!(cache.count(CacheOperation::Write), writes);
}

async fn assert_ownerless_denied(
    cache: &FakeCache,
    store: &ResponsePollingStore,
    caller: &PollCaller,
) {
    let legacy_id = "litellm_poll_legacy";
    cache.seed(
        &get_cache_key(legacy_id),
        r#"{"id":"litellm_poll_legacy","status":"queued","output":[]}"#,
    );
    support::assert_denial(
        store
            .get_state(legacy_id, support::access(caller))
            .await
            .expect_err("the ownerless record is denied"),
        PollDenial::OwnerlessRecord,
    );
    let readable = store
        .get_state(legacy_id, PollAccess::AuthDisabled)
        .await
        .expect("auth-disabled reads the ownerless record");
    assert!(readable.is_some());
    support::assert_denial(
        store
            .update_state(
                legacy_id,
                support::access(caller),
                &support::patch(r#"{"status":"completed"}"#),
            )
            .await
            .expect_err("the ownerless update is denied"),
        PollDenial::OwnerlessRecord,
    );
}

async fn assert_anonymous_denied(
    cache: &FakeCache,
    store: &ResponsePollingStore,
    caller: &PollCaller,
) {
    let anonymous_id = "litellm_poll_anonymous";
    cache.seed(&get_cache_key(anonymous_id), ANONYMOUS);
    support::assert_denial(
        store
            .get_state(anonymous_id, support::access(caller))
            .await
            .expect_err("the anonymous record is denied"),
        PollDenial::AnonymousRecord,
    );
    let readable = store
        .get_state(anonymous_id, PollAccess::AuthDisabled)
        .await
        .expect("auth-disabled reads the anonymous record");
    assert!(readable.is_some());
}

fn owned() -> Json {
    support::json(r#"{"user_id":"a","team_id":"t","key_fingerprint":"k"}"#)
}
