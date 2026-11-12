use std::collections::BTreeMap;

use betterlite_cache::backend::memory::MemoryCacheOps as _;
use betterlite_cache::codec::CodecError;
use betterlite_models::decisions::Json;
use serde::{Deserialize, Serialize};

use super::keys_copy::{auth_fixture, payload as reserved_payload};
use super::support::{capture, local_cache, lock, recording_cache};
use crate::auth_cache::payload::PayloadError;
use crate::auth_cache::{AuthCacheError, WriteOptions};
use crate::resolved_key::auth::AuthError;

#[derive(Debug, Deserialize, PartialEq, Serialize)]
struct BudgetRow {
    model: String,
    #[serde(rename = "max_budget")]
    limit: Option<f64>,
}

fn json(text: &str) -> Json {
    serde_json::from_str(text).expect("the fixture parses")
}

#[tokio::test]
async fn raw_false_zero_empty_retained() {
    let cache = local_cache(Some(60.0_f64));
    let falsy = json("false");
    let zero = json("0");
    let empty = json("\"\"");
    cache
        .set(Some("tag:falsy"), &falsy, WriteOptions::default())
        .unwrap();
    cache
        .set(Some("tag:zero"), &zero, WriteOptions::default())
        .unwrap();
    cache
        .set(Some("tag:empty"), &empty, WriteOptions::default())
        .unwrap();
    assert_eq!(cache.get("tag:falsy", false), Some(json("false")));
    assert_eq!(cache.get("tag:zero", false), Some(json("0")));
    assert_eq!(cache.get("tag:empty", false), Some(json("\"\"")));
    assert_eq!(
        cache.async_get("tag:falsy", false).await,
        Some(json("false"))
    );
}

#[test]
fn typed_none_silent() {
    let cache = local_cache(Some(60.0_f64));
    let (decoded, log) = capture(|| cache.get_typed::<BudgetRow>("tag:absent", false));
    assert_eq!(decoded.unwrap(), None);
    assert!(!log.contains("ERROR"), "{log}");
    assert!(!log.contains("validation failed"), "{log}");
}

#[test]
fn matching_model_identity_retained() {
    let cache = local_cache(Some(60.0_f64));
    let row = BudgetRow {
        model: "gpt-4o".to_owned(),
        limit: Some(10.5_f64),
    };
    cache
        .set(Some("tag:row"), &row, WriteOptions::default())
        .unwrap();
    let decoded = cache.get_typed::<BudgetRow>("tag:row", false).unwrap();
    assert_eq!(decoded, Some(row));
    let stored = cache.get("tag:row", false).unwrap();
    assert_eq!(stored, json("{\"model\":\"gpt-4o\",\"max_budget\":10.5}"));
}

#[test]
fn malformed_dict_warning_then_error() {
    let cache = local_cache(Some(60.0_f64));
    cache
        .set(
            Some("tag:malformed"),
            &json("{\"max_budget\": 10.5}"),
            WriteOptions::default(),
        )
        .unwrap();
    let (decoded, log) = capture(|| cache.get_typed::<BudgetRow>("tag:malformed", false));
    assert_eq!(decoded.unwrap(), None);
    assert!(
        log.contains("CacheCodec.decode: validation failed"),
        "{log}"
    );
    assert!(
        log.contains("UserApiKeyCache.get_cache failed to deserialize"),
        "{log}"
    );
    let warning = log.find("WARN").expect("the codec warns");
    let error = log.find("ERROR").expect("the wrapper logs an error");
    assert!(warning < error, "{log}");
}

#[test]
fn scalar_hit_error_without_codec_warning() {
    let cache = local_cache(Some(60.0_f64));
    cache
        .set(
            Some("tag:scalar"),
            &json("\"cached\""),
            WriteOptions::default(),
        )
        .unwrap();
    let (decoded, log) = capture(|| cache.get_typed::<BudgetRow>("tag:scalar", false));
    assert_eq!(decoded.unwrap(), None);
    assert!(!log.contains("validation failed"), "{log}");
    assert!(
        log.contains("UserApiKeyCache.get_cache failed to deserialize"),
        "{log}"
    );
}

#[test]
fn invalid_entry_not_evicted() {
    let cache = local_cache(Some(60.0_f64));
    let bad = json("{\"max_budget\": \"ten\"}");
    cache
        .set(Some("tag:kept"), &bad, WriteOptions::default())
        .unwrap();
    let (decoded, _log) = capture(|| cache.get_typed::<BudgetRow>("tag:kept", false));
    assert_eq!(decoded.unwrap(), None);
    assert_eq!(cache.get("tag:kept", false), Some(bad));
}

#[tokio::test]
async fn codec_nonvalidation_error_propagates() {
    let cache = local_cache(Some(60.0_f64));
    cache
        .set(
            Some("tag:auth"),
            &json("{\"api_key\": 5}"),
            WriteOptions::default(),
        )
        .unwrap();
    let result = cache.get_auth("tag:auth", false);
    assert!(
        matches!(result, Err(AuthError::NonStringApiKey)),
        "{result:?}"
    );
    let async_result = cache.async_get_auth("tag:auth", false).await;
    assert!(matches!(async_result, Err(AuthError::NonStringApiKey)));
}

#[tokio::test]
async fn model_dump_same_memory_and_redis() {
    let cache = local_cache(Some(60.0_f64));
    let mut auth = auth_fixture();
    auth.extra.request_route = Some("/v1/chat/completions".to_owned());
    cache
        .async_set(Some("tag:dump"), &auth, WriteOptions::default())
        .await
        .unwrap();
    // DualCache hands this one payload to the memory tier and to Redis, so a memory hit is the dump itself
    assert!(
        cache
            .in_memory_cache_for("tag:dump")
            .get_cache("tag:dump")
            .unwrap()
            .is_some()
    );
    let through_the_cache = json(&serde_json::to_string(&auth).unwrap());
    assert_eq!(cache.get("tag:dump", false), Some(through_the_cache));
}

#[test]
fn typed_serialization_fails_before_write() {
    let fixture = recording_cache();
    let explode = BTreeMap::from([(vec![1_u8], 1_i64)]);
    let result = fixture
        .cache
        .set(Some("tag:bad"), &explode, WriteOptions::default());
    match result {
        Err(AuthCacheError::Payload(error)) => {
            assert!(matches!(
                *error,
                PayloadError::Codec(CodecError::Serialize(_))
            ));
        }
        other => panic!("{other:?}"),
    }
    assert!(fixture.operations().is_empty());
    assert!(lock(&fixture.ordinary).sets.is_empty());
}

#[test]
fn resolved_key_exclusions_survive_dump() {
    let cache = local_cache(Some(60.0_f64));
    let mut auth = auth_fixture();
    auth.extra.budget_reservation = Some(reserved_payload("reserved"));
    auth.extra.matched_model_access_groups = Some(vec!["group-1".to_owned()]);
    auth.extra.requires_fresh_policy = true;
    auth.extra.is_session_token = true;
    cache
        .set(Some("tag:excluded"), &auth, WriteOptions::default())
        .unwrap();
    let stored = cache.get("tag:excluded", false).unwrap();
    let Json::Object(fields) = stored else {
        panic!("the cached auth is an object");
    };
    assert!(!fields.contains_key("budget_reservation"));
    assert!(!fields.contains_key("matched_model_access_groups"));
    assert!(!fields.contains_key("requires_fresh_policy"));
    assert_eq!(fields.get("is_session_token"), Some(&Json::Bool(true)));
}
