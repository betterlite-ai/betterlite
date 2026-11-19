use std::time::Duration;

use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use indexmap::IndexMap;
use rstest::rstest;
use serde_json::Number;

use super::support::{
    TestClock, key, local_pins, metadata, pin, pin_fields, runtime, shared_store,
};
use crate::affinity_claims::AffinityClaims;
use crate::affinity_claims::identity::{
    AffinityKey, hash_user_key, looks_like_sha256_hex, marker_session_affinity_ttl,
    session_id_from_request_kwargs, user_key_from_request_kwargs,
};
use crate::affinity_claims::pin::{MAX_PIN_BYTES, PinError, StoredPin};
use crate::affinity_claims::shared::{MAX_CLAIM_ATTEMPTS, SharedFallback};
use crate::routing_state_store::test_support::FakeRedis;

const HASHED_CALLER: &str = "61de55ba3388aec4fdf6ecfc6eef846fb0f1b13e28cc8381270c27ffa313aee5";

#[test]
fn a_pin_at_the_byte_limit_parses_and_one_byte_past_is_rejected() {
    let at_limit = format!("\"{}\"", "a".repeat(MAX_PIN_BYTES.saturating_sub(2)));
    assert_eq!(at_limit.len(), MAX_PIN_BYTES);
    assert!(matches!(
        StoredPin::parse(at_limit.as_bytes()),
        Ok(StoredPin::Text(_))
    ));
    let over = format!("{at_limit} ");
    assert_eq!(
        StoredPin::parse(over.as_bytes()).unwrap_err(),
        PinError::TooLarge {
            bytes: MAX_PIN_BYTES.saturating_add(1),
            limit: MAX_PIN_BYTES,
        }
    );
}

#[test]
fn the_parser_keeps_every_legacy_json_shape() {
    assert_eq!(
        StoredPin::parse(br#"{"model_id":"dep-a","tier":"t1"}"#).unwrap(),
        StoredPin::Pin(pin_fields(&[("model_id", "dep-a"), ("tier", "t1")]))
    );
    assert_eq!(
        StoredPin::parse(br#""dep-a""#).unwrap(),
        StoredPin::Text("dep-a".to_owned())
    );
    assert_eq!(
        StoredPin::parse(b"7").unwrap(),
        StoredPin::Other(Json::Number(Number::from(7_u8)))
    );
    assert_eq!(
        StoredPin::parse(b"null").unwrap(),
        StoredPin::Other(Json::Null)
    );
    assert_eq!(
        StoredPin::parse(b"[]").unwrap(),
        StoredPin::Other(Json::List(Vec::new()))
    );
    assert_eq!(
        StoredPin::parse(br#"{"model_id":7}"#).unwrap(),
        StoredPin::Other(Json::Object(IndexMap::from([(
            "model_id".to_owned(),
            Json::Number(Number::from(7_u8))
        )])))
    );
    assert_eq!(
        StoredPin::parse(b"dep-a").unwrap(),
        StoredPin::Text("dep-a".to_owned())
    );
    assert_eq!(StoredPin::parse(b"\xff").unwrap_err(), PinError::NotText);
}

#[test]
fn legacy_matching_allows_extra_stored_fields_and_bare_strings() {
    let candidate = pin("model_id", "dep-a");
    let stored = StoredPin::Pin(pin_fields(&[("model_id", "dep-a"), ("tier", "t1")]));
    assert!(stored.matches_legacy(&candidate));
    assert!(!stored.matches_exactly(&candidate));
    let bare = StoredPin::Text("dep-a".to_owned());
    assert!(bare.matches_legacy(&candidate));
    assert!(bare.matches_legacy(&pin("model", "dep-a")));
    assert!(!bare.matches_legacy(&pin_fields(&[("model_id", "dep-a"), ("tier", "t1")])));
    let number = StoredPin::Other(Json::Object(IndexMap::from([(
        "model_id".to_owned(),
        Json::Number(Number::from(5_u8)),
    )])));
    assert!(!number.matches_legacy(&pin("model_id", "5")));
    assert_eq!(number.pinned_model_id(), None);
}

#[test]
fn exact_matching_needs_the_same_field_set() {
    let both = pin_fields(&[("model_id", "dep-a"), ("tier", "t1")]);
    assert!(StoredPin::Pin(both.clone()).matches_exactly(&both));
    assert!(!StoredPin::Pin(both).matches_exactly(&pin("model_id", "dep-a")));
    let empty = pin_fields(&[]);
    assert!(StoredPin::Pin(empty.clone()).equals_pin(&empty));
    assert!(StoredPin::Other(Json::List(Vec::new())).matches_exactly(&empty));
    assert!(!StoredPin::Other(Json::List(Vec::new())).equals_pin(&empty));
    assert!(!StoredPin::Text("dep-a".to_owned()).equals_pin(&pin("model_id", "dep-a")));
}

#[test]
fn an_affinity_key_is_capped_at_the_shared_key_limit() {
    let at_limit = "k".repeat(1024);
    assert_eq!(AffinityKey::parse(&at_limit).unwrap().as_ref().len(), 1024);
    let over = "k".repeat(1025);
    assert_eq!(
        AffinityKey::parse(&over).unwrap_err().to_string(),
        "a route key of 1025 bytes is past the 1024-byte limit"
    );
}

#[rstest]
#[case(
    "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
    true
)]
#[case(
    "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcde",
    false
)]
#[case(
    "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef0",
    false
)]
#[case(
    "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdeg",
    false
)]
#[case("", false)]
fn only_sixty_four_ascii_hex_characters_look_like_a_sha256(
    #[case] value: &str,
    #[case] expected: bool,
) {
    assert_eq!(looks_like_sha256_hex(value), expected);
}

#[test]
fn a_python_only_hex_form_is_hashed_instead_of_kept() {
    let prefixed = format!("0x{}", "a".repeat(62));
    assert_eq!(prefixed.len(), 64);
    assert!(!looks_like_sha256_hex(&prefixed));
    assert_ne!(hash_user_key(&prefixed), prefixed);
    assert_eq!(hash_user_key("caller"), HASHED_CALLER);
    let uppercase = HASHED_CALLER.to_uppercase();
    assert_eq!(hash_user_key(&uppercase), HASHED_CALLER);
}

#[test]
fn wrong_metadata_types_name_their_field() {
    let mut hashed = JsonObject::new();
    hashed.insert(
        "metadata".to_owned(),
        Json::Object(metadata(&[(
            "user_api_key_hash",
            Json::Number(Number::from(7_u8)),
        )])),
    );
    assert_eq!(
        user_key_from_request_kwargs(&hashed)
            .unwrap_err()
            .to_string(),
        "the request metadata field user_api_key_hash is not a string"
    );
    let mut numeric = JsonObject::new();
    numeric.insert(
        "metadata".to_owned(),
        Json::Object(metadata(&[(
            "session_id",
            Json::Number(Number::from(1_u8)),
        )])),
    );
    assert_eq!(
        session_id_from_request_kwargs(&numeric)
            .unwrap_err()
            .to_string(),
        "the request metadata field session_id is not a string"
    );
}

#[test]
fn a_wrong_session_marker_type_names_the_marker() {
    let mut marker = JsonObject::new();
    marker.insert(
        "metadata".to_owned(),
        Json::Object(metadata(&[
            ("session_id", Json::Text("session-1".to_owned())),
            ("litellm_session_id_generated", Json::Text("yes".to_owned())),
        ])),
    );
    assert_eq!(
        session_id_from_request_kwargs(&marker)
            .unwrap_err()
            .to_string(),
        "the request metadata field litellm_session_id_generated is not a boolean"
    );
}

fn marker_kwargs(value: Json) -> JsonObject {
    let mut kwargs = JsonObject::new();
    kwargs.insert(
        "metadata".to_owned(),
        Json::Object(metadata(&[("_session_deployment_affinity_ttl", value)])),
    );
    kwargs
}

#[test]
fn the_session_marker_ttl_takes_positive_integers_up_to_the_u32_limit() {
    let at_limit = marker_kwargs(Json::Number(Number::from(4_294_967_295_u64)));
    assert_eq!(
        marker_session_affinity_ttl(&at_limit).unwrap(),
        Some(u32::MAX)
    );
    let past_limit = marker_kwargs(Json::Number(Number::from(4_294_967_296_u64)));
    assert_eq!(
        marker_session_affinity_ttl(&past_limit)
            .unwrap_err()
            .to_string(),
        "the request metadata field _session_deployment_affinity_ttl is out of the ttl range"
    );
    assert_eq!(
        marker_session_affinity_ttl(&marker_kwargs(Json::Number(Number::from(0_u8)))).unwrap(),
        None
    );
    assert_eq!(
        marker_session_affinity_ttl(&marker_kwargs(Json::Number(Number::from(-1_i8)))).unwrap(),
        None
    );
    assert_eq!(
        marker_session_affinity_ttl(&marker_kwargs(Json::Bool(true))).unwrap(),
        None
    );
}

#[test]
fn a_local_pin_lives_until_its_expiry_instant_and_not_past_it() {
    let clock = TestClock::default();
    clock.at(1_000.0);
    let pins = local_pins(&clock);
    let cache_key = key("deployment_affinity:v1:group:caller");
    pins.claim_in_memory(&cache_key, &pin("model_id", "dep-a"), 10, None)
        .unwrap();
    assert_eq!(pins.expiry_seconds(&cache_key).unwrap(), Some(1_010.0));
    clock.at(1_010.0);
    assert_eq!(
        pins.read(&cache_key).unwrap(),
        Some(StoredPin::Pin(pin("model_id", "dep-a")))
    );
    clock.at(1_010.25);
    assert_eq!(pins.read(&cache_key).unwrap(), None);
}

#[test]
fn a_zero_ttl_shared_claim_falls_back_to_the_local_pin() {
    runtime().block_on(async {
        let server = FakeRedis::try_start().await.unwrap();
        let clock = TestClock::default();
        clock.at(1_700_000_000.0);
        let service = AffinityClaims {
            local: local_pins(&clock),
            shared: Some(shared_store(&server)),
        };
        let cache_key = key("deployment_affinity:v1:group:caller");
        let claimed = service
            .claim(&cache_key, &pin("model_id", "dep-a"), 0, None)
            .await
            .unwrap();
        assert_eq!(claimed.fallback, Some(SharedFallback::Unavailable));
        assert_eq!(claimed.pin, StoredPin::Pin(pin("model_id", "dep-a")));
        clock.at(1_700_000_001.0);
        assert_eq!(service.local.read(&cache_key).unwrap(), None);
    });
}

#[test]
fn a_claim_that_loses_the_race_too_often_falls_back_to_the_local_pin() {
    runtime().block_on(async {
        let server = FakeRedis::try_start().await.unwrap();
        let clock = TestClock::default();
        clock.at(1_700_000_000.0);
        let service = AffinityClaims {
            local: local_pins(&clock),
            shared: Some(shared_store(&server)),
        };
        let cache_key = key("deployment_affinity:v1:group:caller");
        service
            .claim(&cache_key, &pin("model_id", "dep-a"), 60, None)
            .await
            .unwrap();
        let candidates = [pin("model_id", "dep-a"), pin("model_id", "dep-b")];
        let losses = u64::from(MAX_CLAIM_ATTEMPTS.saturating_sub(1));
        server.rival_writes_before_exec(losses);
        let survived = service
            .claim(&cache_key, &pin("model_id", "dep-b"), 60, Some(&candidates))
            .await
            .unwrap();
        assert_eq!(survived.fallback, None);
        assert_eq!(survived.pin, StoredPin::Pin(pin("model_id", "dep-a")));
        server.rival_writes_before_exec(u64::from(MAX_CLAIM_ATTEMPTS));
        let conflicted = service
            .claim(&cache_key, &pin("model_id", "dep-b"), 60, Some(&candidates))
            .await
            .unwrap();
        assert_eq!(conflicted.fallback, Some(SharedFallback::Conflict));
        assert_eq!(conflicted.pin, StoredPin::Pin(pin("model_id", "dep-a")));
    });
}

#[test]
fn a_foreign_shared_value_falls_back_to_the_local_pin() {
    runtime().block_on(async {
        let server = FakeRedis::try_start().await.unwrap();
        let clock = TestClock::default();
        clock.at(1_700_000_000.0);
        let service = AffinityClaims {
            local: local_pins(&clock),
            shared: Some(shared_store(&server)),
        };
        let cache_key = key("deployment_affinity:v1:group:caller");
        server.seed(cache_key.as_ref(), br#"{"counter":0}"#, 60);
        let claimed = service
            .claim(&cache_key, &pin("model_id", "dep-a"), 60, None)
            .await
            .unwrap();
        assert_eq!(claimed.fallback, Some(SharedFallback::ValueUnusable));
        assert_eq!(claimed.pin, StoredPin::Pin(pin("model_id", "dep-a")));
        assert_eq!(
            server.raw(cache_key.as_ref()).unwrap().0.as_slice(),
            br#"{"counter":0}"#.as_slice()
        );
    });
}

#[test]
fn a_cancelled_claim_leaves_the_pin_to_the_next_claim() {
    runtime().block_on(async {
        let server = FakeRedis::try_start().await.unwrap();
        let clock = TestClock::default();
        clock.at(1_700_000_000.0);
        let service = AffinityClaims {
            local: local_pins(&clock),
            shared: Some(shared_store(&server)),
        };
        let cache_key = key("deployment_affinity:v1:group:caller");
        let cancelled = tokio::time::timeout(
            Duration::ZERO,
            service.claim(&cache_key, &pin("model_id", "dep-a"), 60, None),
        )
        .await;
        let _elapsed = cancelled.unwrap_err();
        let claimed = service
            .claim(&cache_key, &pin("model_id", "dep-a"), 60, None)
            .await
            .unwrap();
        assert_eq!(claimed.pin, StoredPin::Pin(pin("model_id", "dep-a")));
        assert_eq!(claimed.fallback, None);
    });
}

#[test]
fn a_pin_over_the_local_item_limit_is_not_kept() {
    let clock = TestClock::default();
    clock.at(1_000.0);
    let pins = local_pins(&clock);
    let cache_key = key("deployment_affinity:v1:group:caller");
    let huge = pin("model_id", &"x".repeat(MAX_PIN_BYTES));
    let winner = pins.claim_in_memory(&cache_key, &huge, 60, None).unwrap();
    assert_eq!(winner, StoredPin::Pin(huge));
    assert_eq!(pins.read(&cache_key).unwrap(), None);
}
