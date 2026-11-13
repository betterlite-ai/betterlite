#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use betterlite_redis::cache_primitives::pipeline_increment::IncrementOperation;
use rstest::rstest;

use super::{
    AUDIO_BYTES_PER_TOKEN, AtomicCounterMeta, AtomicCounterState, CacheCounterValue, CounterRefund,
    CounterReplyError, DEFAULT_AUDIO_TOKEN_ESTIMATE, DEFAULT_CHARS_PER_TOKEN,
    DEFAULT_MAX_TOKENS_ESTIMATE, DescriptorAtomicGroup, EMBEDDING_API_CALL_TYPES,
    GOOGLE_GENAI_NATIVE_CALL_TYPES, LimiterClock, ModelGroupResolver, PARALLEL_SLOT_TTL_SECONDS,
    PROJECT_ITPM_DESCRIPTOR_KEY, PROJECT_OTPM_DESCRIPTOR_KEY, ParallelRequestGauge,
    ParallelSlotAcquisition, RERANK_API_CALL_TYPES, RESPONSES_API_CALL_TYPES,
    RESPONSES_MIN_OUTPUT_TOKENS, RateLimitDescriptor, RateLimitDescriptorRateLimit,
    RateLimitResponse, RateLimitStatus, RateLimitType, RateLimitedModel, ReservationAwareIncrement,
    ReservationBackend, ReservationWindowIdentity, ResponseWithDescriptors, ScriptReply,
    ScriptReplyKind, TEXT_COMPLETION_API_CALL_TYPES, TPM_FLOOR_FRACTION, TagRateLimit,
    TagRateLimitResolver, WindowKeyMetadata, WindowRateLimitType, counter_values_from_reply,
    create_rate_limit_keys, sibling_counter_keys,
};

fn status() -> RateLimitStatus {
    RateLimitStatus {
        code: "OK".to_owned(),
        current_limit: 10,
        limit_remaining: 4,
        rate_limit_type: RateLimitType::Requests,
        descriptor_key: "model_per_key".to_owned(),
        descriptor_value: Some("key-1".to_owned()),
    }
}

fn descriptor() -> RateLimitDescriptor {
    RateLimitDescriptor {
        key: "model".to_owned(),
        value: "gpt-4".to_owned(),
        rate_limit: Some(RateLimitDescriptorRateLimit {
            requests_per_unit: Some(60),
            tokens_per_unit: None,
            max_parallel_requests: Some(3),
            window_size: None,
        }),
    }
}

fn json<T>(value: &T) -> String
where
    T: serde::Serialize,
{
    serde_json::to_string(value).expect("the record serializes")
}

#[test]
fn descriptor_records_keep_litellm_field_names() {
    assert_eq!(
        json(&descriptor()),
        r#"{"key":"model","value":"gpt-4","rate_limit":{"requests_per_unit":60,"tokens_per_unit":null,"max_parallel_requests":3,"window_size":null}}"#
    );
    assert_eq!(
        json(&ParallelRequestGauge {
            counter_key: "{model:gpt-4}:requests".to_owned(),
            limit: 60,
            descriptor_key: "model".to_owned(),
        }),
        r#"{"counter_key":"{model:gpt-4}:requests","limit":60,"descriptor_key":"model"}"#
    );
    assert_eq!(
        json(&ParallelSlotAcquisition {
            slot_id: "slot-1".to_owned(),
            counter_keys: vec!["counter-key-1".to_owned()],
        }),
        r#"{"slot_id":"slot-1","counter_keys":["counter-key-1"]}"#
    );
    let group: DescriptorAtomicGroup = (Vec::new(), vec![2], Vec::new());
    assert_eq!(group.1, vec![2]);
}

#[test]
fn status_and_window_records_keep_litellm_field_names() {
    assert_eq!(
        json(&status()),
        r#"{"code":"OK","current_limit":10,"limit_remaining":4,"rate_limit_type":"requests","descriptor_key":"model_per_key","descriptor_value":"key-1"}"#
    );
    assert_eq!(
        json(&WindowKeyMetadata {
            requests_limit: Some(60),
            tokens_limit: None,
            window_size: 60,
            descriptor_key: "model".to_owned(),
            descriptor_value: "gpt-4".to_owned(),
        }),
        r#"{"requests_limit":60,"tokens_limit":null,"window_size":60,"descriptor_key":"model","descriptor_value":"gpt-4"}"#
    );
    assert_eq!(
        json(&AtomicCounterState {
            window_expired: true,
            current: 3,
            window_start: "1700000000".to_owned(),
        }),
        r#"{"window_expired":true,"current":3,"window_start":"1700000000"}"#
    );
}

#[test]
fn atomic_and_refund_records_keep_litellm_field_names() {
    assert_eq!(
        json(&AtomicCounterMeta {
            descriptor_key: "model".to_owned(),
            descriptor_value: "gpt-4".to_owned(),
            current_limit: 60,
            rate_limit_type: WindowRateLimitType::Tokens,
            window_key: "{model:gpt-4}:window".to_owned(),
            counter_key: "{model:gpt-4}:tokens".to_owned(),
            increment: 1,
            ttl: 60,
            window_size: 60,
        }),
        r#"{"descriptor_key":"model","descriptor_value":"gpt-4","current_limit":60,"rate_limit_type":"tokens","window_key":"{model:gpt-4}:window","counter_key":"{model:gpt-4}:tokens","increment":1,"ttl":60,"window_size":60}"#
    );
    assert_eq!(
        json(&CounterRefund {
            window_key: "{model:gpt-4}:window".to_owned(),
            counter_key: "{model:gpt-4}:tokens".to_owned(),
            window_start: "1700000000".to_owned(),
            increment: 2,
        }),
        r#"{"window_key":"{model:gpt-4}:window","counter_key":"{model:gpt-4}:tokens","window_start":"1700000000","increment":2}"#
    );
}

#[test]
fn responses_carry_statuses_and_optional_reservation_windows() {
    let plain = RateLimitResponse {
        overall_code: "OK".to_owned(),
        statuses: vec![status()],
        reservation_windows: None,
    };
    assert_eq!(
        json(&plain),
        r#"{"overall_code":"OK","statuses":[{"code":"OK","current_limit":10,"limit_remaining":4,"rate_limit_type":"requests","descriptor_key":"model_per_key","descriptor_value":"key-1"}],"reservation_windows":null}"#
    );
    let reserved = RateLimitResponse {
        overall_code: "OK".to_owned(),
        statuses: Vec::new(),
        reservation_windows: Some(BTreeSet::from([identity(ReservationBackend::Redis)])),
    };
    assert_eq!(
        json(&reserved),
        r#"{"overall_code":"OK","statuses":[],"reservation_windows":[{"counter_key":"{model:gpt-4}:tokens","window_start":"1700000000","backend":"redis"}]}"#
    );
}

#[test]
fn responses_with_descriptors_pair_the_descriptors_and_the_response() {
    let response = ResponseWithDescriptors {
        descriptors: vec![descriptor()],
        response: RateLimitResponse {
            overall_code: "OK".to_owned(),
            statuses: Vec::new(),
            reservation_windows: Some(BTreeSet::from([identity(ReservationBackend::Local)])),
        },
    };
    assert_eq!(
        json(&response),
        r#"{"descriptors":[{"key":"model","value":"gpt-4","rate_limit":{"requests_per_unit":60,"tokens_per_unit":null,"max_parallel_requests":3,"window_size":null}}],"response":{"overall_code":"OK","statuses":[],"reservation_windows":[{"counter_key":"{model:gpt-4}:tokens","window_start":"1700000000","backend":"local"}]}}"#
    );
}

fn identity(backend: ReservationBackend) -> ReservationWindowIdentity {
    ReservationWindowIdentity {
        counter_key: "{model:gpt-4}:tokens".to_owned(),
        window_start: "1700000000".to_owned(),
        backend,
    }
}

#[rstest]
#[case::requests(RateLimitType::Requests, "requests")]
#[case::tokens(RateLimitType::Tokens, "tokens")]
#[case::max_parallel(RateLimitType::MaxParallelRequests, "max_parallel_requests")]
fn rate_limit_type_serializes_to_its_wire_string(#[case] kind: RateLimitType, #[case] wire: &str) {
    assert_eq!(json(&kind), format!("\"{wire}\""));
    assert_eq!(kind.as_str(), wire);
    assert_eq!(
        serde_json::from_str::<RateLimitType>(&format!("\"{wire}\"")).unwrap(),
        kind
    );
}

#[rstest]
#[case::requests(WindowRateLimitType::Requests, "requests")]
#[case::tokens(WindowRateLimitType::Tokens, "tokens")]
fn window_rate_limit_type_serializes_to_its_wire_string(
    #[case] kind: WindowRateLimitType,
    #[case] wire: &str,
) {
    assert_eq!(json(&kind), format!("\"{wire}\""));
}

#[test]
fn create_rate_limit_keys_wraps_the_descriptor_in_a_hash_tag() {
    assert_eq!(
        create_rate_limit_keys("team", "abc", RateLimitType::Requests),
        "{team:abc}:requests"
    );
    assert_eq!(
        create_rate_limit_keys("key", "sk-1", RateLimitType::Tokens),
        "{key:sk-1}:tokens"
    );
    assert_eq!(
        create_rate_limit_keys("model", "gpt-4", RateLimitType::MaxParallelRequests),
        "{model:gpt-4}:max_parallel_requests"
    );
}

#[rstest]
#[case::window_suffix("team:abc:window", ("team:abc:requests", "team:abc:tokens"))]
#[case::no_suffix("team:abc", ("team:abc:requests", "team:abc:tokens"))]
#[case::only_the_suffix(":window", (":requests", ":tokens"))]
#[case::one_suffix_removed(
    "team:window:window",
    ("team:window:requests", "team:window:tokens")
)]
#[case::empty("", (":requests", ":tokens"))]
fn sibling_counter_keys_strip_one_window_suffix(
    #[case] window_key: &str,
    #[case] expected: (&str, &str),
) {
    assert_eq!(
        sibling_counter_keys(window_key),
        (expected.0.to_owned(), expected.1.to_owned())
    );
}

#[test]
fn limit_in_prefers_the_requested_name_and_keeps_a_zero() {
    let model = RateLimitedModel {
        requested: "gpt4".to_owned(),
        group: "gpt-4".to_owned(),
    };
    let both = BTreeMap::from([("gpt4".to_owned(), 0), ("gpt-4".to_owned(), 500)]);
    assert_eq!(model.limit_in(Some(&both)), Some(0));
    let group_only = BTreeMap::from([("gpt-4".to_owned(), 500)]);
    assert_eq!(model.limit_in(Some(&group_only)), Some(500));
    let neither = BTreeMap::from([("other".to_owned(), 7)]);
    assert_eq!(model.limit_in(Some(&neither)), None);
    assert_eq!(model.limit_in(Some(&BTreeMap::new())), None);
    assert_eq!(model.limit_in(None), None);
    let requested_only = BTreeMap::from([("gpt4".to_owned(), 12)]);
    assert_eq!(model.limit_in(Some(&requested_only)), Some(12));
}

#[test]
fn counter_values_from_reply_keeps_each_value_in_order() {
    let reply = ScriptReply::Array(vec![
        ScriptReply::Int(3),
        ScriptReply::Float(2.5),
        ScriptReply::Text("ok".to_owned()),
        ScriptReply::Bytes(vec![1, 2]),
    ]);
    assert_eq!(
        counter_values_from_reply(&reply).unwrap(),
        vec![
            CacheCounterValue::Int(3),
            CacheCounterValue::Float(2.5),
            CacheCounterValue::Text("ok".to_owned()),
            CacheCounterValue::Bytes(vec![1, 2]),
        ]
    );
    assert_eq!(
        counter_values_from_reply(&ScriptReply::Array(Vec::new())).unwrap(),
        Vec::new()
    );
}

#[rstest]
#[case::nil_reply(ScriptReply::Nil, CounterReplyError::NotAnArray { kind: ScriptReplyKind::Nil })]
#[case::int_reply(ScriptReply::Int(1), CounterReplyError::NotAnArray { kind: ScriptReplyKind::Int })]
#[case::float_reply(ScriptReply::Float(1.5), CounterReplyError::NotAnArray { kind: ScriptReplyKind::Float })]
#[case::text_reply(
    ScriptReply::Text("x".to_owned()),
    CounterReplyError::NotAnArray { kind: ScriptReplyKind::Text }
)]
#[case::bytes_reply(
    ScriptReply::Bytes(vec![0]),
    CounterReplyError::NotAnArray { kind: ScriptReplyKind::Bytes }
)]
#[case::nil_element(
    ScriptReply::Array(vec![ScriptReply::Int(1), ScriptReply::Nil]),
    CounterReplyError::UnsupportedElement { kind: ScriptReplyKind::Nil }
)]
#[case::nested_array(
    ScriptReply::Array(vec![
        ScriptReply::Int(1),
        ScriptReply::Array(vec![ScriptReply::Int(2)]),
    ]),
    CounterReplyError::UnsupportedElement { kind: ScriptReplyKind::Array }
)]
fn counter_values_from_reply_rejects_unsupported_replies(
    #[case] reply: ScriptReply,
    #[case] expected: CounterReplyError,
) {
    assert_eq!(counter_values_from_reply(&reply), Err(expected));
}

#[test]
fn constants_carry_litellm_values_and_members() {
    assert_eq!(PROJECT_ITPM_DESCRIPTOR_KEY, "model_per_project_itpm");
    assert_eq!(PROJECT_OTPM_DESCRIPTOR_KEY, "model_per_project_otpm");
    assert_eq!(PARALLEL_SLOT_TTL_SECONDS, 3600);
    assert_eq!(DEFAULT_MAX_TOKENS_ESTIMATE, 4096);
    assert_eq!(DEFAULT_CHARS_PER_TOKEN, 4);
    assert_eq!(TPM_FLOOR_FRACTION, 4);
    assert_eq!(RESPONSES_MIN_OUTPUT_TOKENS, 16);
    assert_eq!(DEFAULT_AUDIO_TOKEN_ESTIMATE, 300);
    assert_eq!(AUDIO_BYTES_PER_TOKEN, 1600);
    assert_eq!(RESPONSES_API_CALL_TYPES, ["aresponses", "responses"]);
    assert_eq!(EMBEDDING_API_CALL_TYPES, ["aembedding", "embedding"]);
    assert_eq!(
        TEXT_COMPLETION_API_CALL_TYPES,
        ["atext_completion", "text_completion"]
    );
    assert_eq!(RERANK_API_CALL_TYPES, ["rerank", "arerank"]);
    assert_eq!(
        GOOGLE_GENAI_NATIVE_CALL_TYPES,
        [
            "generate_content",
            "agenerate_content",
            "generate_content_stream",
            "agenerate_content_stream",
        ]
    );
}

#[test]
fn reservation_aware_increments_carry_the_operation_and_the_window() {
    let increment = ReservationAwareIncrement {
        operation: IncrementOperation {
            key: "{model:gpt-4}:tokens".to_owned(),
            increment_value: 12.0,
            ttl: Some(60),
        },
        window_key: Some("{model:gpt-4}:window".to_owned()),
        expected_window_start: Some("1700000000".to_owned()),
        reservation_backend: Some(ReservationBackend::Redis),
    };
    assert_eq!(increment.operation.key, "{model:gpt-4}:tokens");
    assert_eq!(increment.operation.ttl, Some(60));
    assert_eq!(
        increment.window_key.as_deref(),
        Some("{model:gpt-4}:window")
    );
    assert_eq!(
        increment.reservation_backend,
        Some(ReservationBackend::Redis)
    );
    let local = ReservationAwareIncrement {
        window_key: None,
        expected_window_start: None,
        reservation_backend: None,
        ..increment
    };
    assert_eq!(local.reservation_backend, None);
}

struct FixedClock(SystemTime);

impl LimiterClock for FixedClock {
    fn current_time(&self) -> SystemTime {
        self.0
    }
}

struct Aliases(BTreeMap<String, String>);

impl ModelGroupResolver for Aliases {
    fn resolve(&self, model: &str) -> Option<String> {
        self.0.get(model).cloned()
    }
}

struct Tags(BTreeMap<String, TagRateLimit>);

#[async_trait::async_trait]
impl TagRateLimitResolver for Tags {
    async fn resolve(&self, tag_names: &[String]) -> BTreeMap<String, TagRateLimit> {
        tag_names
            .iter()
            .filter_map(|tag| self.0.get(tag).map(|limit| (tag.clone(), *limit)))
            .collect()
    }
}

#[test]
fn seams_are_injectable() {
    let injected = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
    let clock: Arc<dyn LimiterClock> = Arc::new(FixedClock(injected));
    assert_eq!(clock.current_time(), injected);
    let resolver: Box<dyn ModelGroupResolver> = Box::new(Aliases(BTreeMap::from([(
        "gpt4".to_owned(),
        "gpt-4".to_owned(),
    )])));
    assert_eq!(resolver.resolve("gpt4"), Some("gpt-4".to_owned()));
    assert_eq!(resolver.resolve("unknown"), None);
}

#[tokio::test]
async fn the_tag_resolver_is_awaited_through_its_trait() {
    let resolver: Box<dyn TagRateLimitResolver> = Box::new(Tags(BTreeMap::from([(
        "free".to_owned(),
        TagRateLimit {
            rpm_limit: Some(5),
            tpm_limit: None,
        },
    )])));
    let limits_by_tag = resolver
        .resolve(&["free".to_owned(), "missing".to_owned()])
        .await;
    assert_eq!(
        limits_by_tag.get("free"),
        Some(&TagRateLimit {
            rpm_limit: Some(5),
            tpm_limit: None,
        })
    );
    assert_eq!(limits_by_tag.len(), 1);
}
