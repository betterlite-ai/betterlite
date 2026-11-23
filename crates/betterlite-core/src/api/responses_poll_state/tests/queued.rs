use std::sync::Arc;

use betterlite_models::decisions::Json;
use uuid::Uuid;

use crate::api::api_boundaries::ApiLimits;

use super::super::record::PollMetadata;
use super::super::store::{NewPollRequest, ResponsePollingStore};
use super::super::{generate_polling_id, get_cache_key};
use super::support::{self, CacheOperation, FakeCache};

const CREATED_AT: i64 = 1_700_000_000;

pub(super) async fn queued_record() {
    let cache = FakeCache::shared();
    let store = support::store(Arc::clone(&cache), 3600);
    let uuid = Uuid::parse_str("00000000-0000-4000-8000-000000000001").expect("the UUID parses");
    let polling_id = generate_polling_id(uuid);
    let request = NewPollRequest {
        polling_id: &polling_id,
        created_at: CREATED_AT,
        metadata: Some(support::metadata(("trace", "a"))),
        owner: None,
    };
    let response = store
        .create_initial_state(request)
        .await
        .expect("the queued record is written");
    let public = Json::Object(response.clone());
    assert_queued_fields(&public, &polling_id);
    assert_eq!(response.len(), 23);
    assert_write_effect(&cache, &polling_id);
    assert_envelope(&cache, &polling_id, &public);
}

fn assert_queued_fields(public: &Json, polling_id: &str) {
    assert_eq!(
        support::field(public, "id"),
        Json::Text(polling_id.to_owned())
    );
    assert_eq!(
        support::field(public, "object"),
        support::json("\"response\"")
    );
    assert_eq!(
        support::field(public, "status"),
        support::json("\"queued\"")
    );
    assert_eq!(
        support::field(public, "created_at"),
        support::json("1700000000")
    );
    assert_eq!(support::field(public, "output"), support::json("[]"));
    assert_eq!(support::field(public, "usage"), Json::Null);
    assert_eq!(
        support::field(public, "metadata"),
        support::json(r#"{"trace":"a"}"#)
    );
    assert_eq!(support::field(public, "model"), Json::Null);
}

fn assert_write_effect(cache: &FakeCache, polling_id: &str) {
    let effects = cache.effects();
    assert_eq!(effects.len(), 1);
    assert_eq!(effects[0].operation, CacheOperation::Write);
    assert_eq!(effects[0].key, get_cache_key(polling_id));
    assert_eq!(effects[0].ttl, Some(3600));
}

fn assert_envelope(cache: &FakeCache, polling_id: &str, public: &Json) {
    let written = cache
        .entry(&get_cache_key(polling_id))
        .expect("the queued record is stored");
    let envelope = support::json(&written);
    assert!(support::has_key(&envelope, "owner"));
    assert_eq!(support::field(&envelope, "owner"), Json::Null);
    let written_response = support::field(&envelope, "response");
    assert_eq!(&written_response, public);
    support::check_b1138(
        &serde_json::to_string(&written_response).expect("the response serializes"),
    );
}

pub(super) async fn queued_without_metadata() {
    let cache = FakeCache::shared();
    let store = support::store(Arc::clone(&cache), 17);
    let request = NewPollRequest {
        polling_id: "litellm_poll_no_metadata",
        created_at: CREATED_AT,
        metadata: None,
        owner: None,
    };
    let response = store
        .create_initial_state(request)
        .await
        .expect("the queued record is written");
    let public = Json::Object(response);
    assert_eq!(support::field(&public, "metadata"), support::json("{}"));
    let effects = cache.effects();
    assert_eq!(effects[0].ttl, Some(17));
}

pub(super) async fn queued_metadata_null() {
    let cache = FakeCache::shared();
    let store = support::store(Arc::clone(&cache), 3600);
    let request = NewPollRequest {
        polling_id: "litellm_poll_null_metadata",
        created_at: CREATED_AT,
        metadata: Some(PollMetadata::Null),
        owner: None,
    };
    let response = store
        .create_initial_state(request)
        .await
        .expect("the queued record is written");
    assert_eq!(
        support::field(&Json::Object(response), "metadata"),
        Json::Null
    );
}

pub(super) async fn queued_without_cache() {
    let store = ResponsePollingStore {
        cache: None,
        ttl: 3600,
        limits: ApiLimits::default(),
    };
    assert!(store.cache.is_none());
    let response = store
        .create_initial_state(support::new_request("litellm_poll_no_cache", None))
        .await
        .expect("the queued record is returned without a cache");
    assert_eq!(
        support::field(&Json::Object(response), "status"),
        support::json("\"queued\"")
    );
}
