use std::fmt::Debug;

use betterlite_models::decisions::Json;

use super::super::access::{PollAccess, PollDenial};
use super::super::access::{PollCaller, PollOwner};
use super::super::error::PollingError;
use super::super::get_cache_key;
use super::super::store::StoredPollState;
use super::super::store::{NewPollRequest, ResponsePollingStore};
use super::support::{self, CacheOperation, FakeCache};

const ID: &str = "litellm_poll_owner";
const COMPLETED: &str = r#"{"status":"completed"}"#;
const LEGACY: &str = r#"{"id":"litellm_poll_legacy","status":"queued","output":[]}"#;
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
        .update_state(ID, access, &support::patch(COMPLETED))
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

async fn read_as(store: &ResponsePollingStore, caller: &PollCaller) -> bool {
    store
        .get_state(ID, support::access(caller))
        .await
        .expect("the caller reads the state")
        .is_some()
}

fn assert_denied<T>(result: Result<T, PollingError>, denial: PollDenial)
where
    T: Debug,
{
    support::assert_denial(result.expect_err("the caller is denied"), denial);
}

async fn expect_denial(store: &ResponsePollingStore, caller: &PollCaller, denial: PollDenial) {
    assert_denied(store.get_state(ID, support::access(caller)).await, denial);
}

async fn assert_update_denied(
    store: &ResponsePollingStore,
    id: &str,
    caller: &PollCaller,
    denial: PollDenial,
) {
    let result = store
        .update_state(id, support::access(caller), &support::patch(COMPLETED))
        .await;
    assert_denied(result, denial);
}

async fn assert_user_denials(store: &ResponsePollingStore) {
    let other = support::caller(Some("b"), Some("t"), false, None);
    expect_denial(store, &other, PollDenial::User).await;
    let no_user = support::caller(None, Some("t"), false, None);
    expect_denial(store, &no_user, PollDenial::User).await;
}

async fn assert_team_denials(store: &ResponsePollingStore) {
    let other_team = support::caller(Some("a"), Some("x"), false, None);
    assert_update_denied(store, ID, &other_team, PollDenial::Team).await;
    let no_team = support::caller(Some("a"), None, false, None);
    expect_denial(store, &no_team, PollDenial::Team).await;
}

async fn assert_admin_reads(store: &ResponsePollingStore) {
    let admin = support::caller(Some("root"), Some("x"), true, None);
    assert!(read_as(store, &admin).await);
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
    let caller = support::caller(Some("a"), Some("t"), false, None);
    let records = [
        ("litellm_poll_legacy", LEGACY, PollDenial::OwnerlessRecord),
        (
            "litellm_poll_anonymous",
            ANONYMOUS,
            PollDenial::AnonymousRecord,
        ),
    ];
    for (id, raw, denial) in records {
        let read_start = cache.effects().len();
        cache.seed(&get_cache_key(id), raw);
        assert_denied(store.get_state(id, support::access(&caller)).await, denial);
        assert_auth_disabled_reads(store, id).await;
        assert_eq!(cache.effects().len(), read_start.saturating_add(2));
    }
    let update_start = cache.effects().len();
    assert_update_denied(
        store,
        "litellm_poll_legacy",
        &caller,
        PollDenial::OwnerlessRecord,
    )
    .await;
    assert_eq!(cache.effects().len(), update_start.saturating_add(1));
}

async fn assert_auth_disabled_reads(store: &ResponsePollingStore, id: &str) {
    let state = store.get_state(id, PollAccess::AuthDisabled).await;
    assert!(state.expect("auth-disabled reads the record").is_some());
}

fn owned() -> Json {
    support::json(r#"{"user_id":"a","team_id":"t","key_fingerprint":"k"}"#)
}
