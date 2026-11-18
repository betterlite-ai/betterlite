#![cfg(test)]

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use betterlite_models::decisions::Json;
use chrono::{DateTime, FixedOffset};
use http::HeaderMap;
use indexmap::IndexMap;
use rstest::rstest;
use url::Url;

use super::observer::{PromptCacheObserver, SuccessEvent};
use super::{
    CacheObservation, ObservationCache, ObservationParser, ObservedCache, ObservedPrefix,
    WireRequest, lookup, read_exact,
};
use crate::fields::payload::StandardLoggingPayload;

const FINGERPRINT: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
const OLDER_FINGERPRINT: &str = "1111111111111111111111111111111111111111111111111111111111111111";
const STALE_FINGERPRINT: &str = "2222222222222222222222222222222222222222222222222222222222222222";
const OTHER_FINGERPRINT: &str = "3333333333333333333333333333333333333333333333333333333333333333";
const UPPERCASE_FINGERPRINT: &str =
    "ABCDEF0123456789abcdef0123456789abcdef0123456789abcdef0123456789";
const NON_HEX_FINGERPRINT: &str =
    "g123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
const SCOPE: &str = "scope-a";
const NATIVE_URL: &str = "https://api.anthropic.com/v1/messages";
const NOW: f64 = 2_000.0;
const STARTED: i64 = 1_000;
const BODY: &str = r#"{"model":"claude-3-5-sonnet","messages":[{"role":"user","content":[{"type":"text","text":"a","cache_control":{"type":"ephemeral"}}]}]}"#;
const PAYLOAD: &str = r#"{"id":"chatcmpl-1","trace_id":"trace-1","session_id":"session-1","call_type":"anthropic_messages","custom_llm_provider":"anthropic","stream":false,"response_cost":0.5,"status":"success","status_fields":{"llm_api_status":"success","guardrail_status":"not_run"},"total_tokens":1010,"prompt_tokens":1000,"completion_tokens":10,"startTime":1704067200.0,"endTime":1704067201.5,"completionStartTime":1704067200.5,"response_time":1.0,"model_map_information":{},"model":"claude-3-5-sonnet","model_id":"deployment","api_base":"https://api.anthropic.com","metadata":{"user_api_key_hash":"caller-1"},"saved_cache_cost":0.0,"request_tags":[],"model_parameters":{},"hidden_params":{}}"#;

fn payload() -> StandardLoggingPayload {
    serde_json::from_str(PAYLOAD).unwrap()
}

fn payload_replacing(from: &str, to: &str) -> StandardLoggingPayload {
    serde_json::from_str(&PAYLOAD.replace(from, to)).unwrap()
}

fn json(text: &str) -> Json {
    serde_json::from_str(text).unwrap()
}

fn starts_at(second: i64) -> DateTime<FixedOffset> {
    DateTime::from_timestamp(second, 0).unwrap().fixed_offset()
}

fn observation(fingerprint: &str, cached: i64, expires_at: f64) -> CacheObservation {
    CacheObservation {
        fingerprint: fingerprint.to_owned(),
        cached_tokens: cached,
        observed_at: 500.0,
        expires_at,
    }
}

fn evidence(creation: i64) -> ObservedCache {
    ObservedCache {
        prefix: ObservedPrefix {
            fingerprint: FINGERPRINT.to_owned(),
            fingerprints: vec![
                FINGERPRINT.to_owned(),
                OLDER_FINGERPRINT.to_owned(),
                STALE_FINGERPRINT.to_owned(),
            ],
        },
        ttl_seconds: 300,
        scope: SCOPE.to_owned(),
        cached_tokens: 500,
        cache_creation_tokens: creation,
    }
}

fn entry_key(scope: &str, fingerprint: &str) -> String {
    format!("prompt-cache-observation:{scope}:{fingerprint}")
}

struct Write {
    key: String,
    value: Json,
    ttl: f64,
    local_only: bool,
}

struct FakeCache {
    entries: Mutex<IndexMap<String, Json>>,
    writes: Mutex<Vec<Write>>,
    redis: bool,
}

impl FakeCache {
    fn new(redis: bool) -> Self {
        Self {
            entries: Mutex::new(IndexMap::new()),
            writes: Mutex::new(Vec::new()),
            redis,
        }
    }

    fn seed(&self, scope: &str, fingerprint: &str, value: Json) {
        self.entries
            .lock()
            .unwrap()
            .insert(entry_key(scope, fingerprint), value);
    }

    fn write_count(&self) -> usize {
        self.writes.lock().unwrap().len()
    }
}

#[async_trait]
impl ObservationCache for FakeCache {
    async fn get(&self, key: &str) -> Option<Json> {
        self.entries.lock().unwrap().get(key).cloned()
    }

    async fn set(&self, key: &str, value: &Json, ttl_seconds: f64, local_only: bool) {
        self.writes.lock().unwrap().push(Write {
            key: key.to_owned(),
            value: value.clone(),
            ttl: ttl_seconds,
            local_only,
        });
        if !local_only {
            self.entries
                .lock()
                .unwrap()
                .insert(key.to_owned(), value.clone());
        }
    }

    fn has_redis(&self) -> bool {
        self.redis
    }
}

#[derive(Default)]
struct FakeParser {
    observed: Option<ObservedCache>,
    seen: Mutex<Vec<(String, String)>>,
}

impl ObservationParser for FakeParser {
    fn parse(
        &self,
        _wire: &WireRequest<'_>,
        _response: &Json,
        caller_key_hash: &str,
        deployment_id: &str,
    ) -> Option<ObservedCache> {
        self.seen
            .lock()
            .unwrap()
            .push((caller_key_hash.to_owned(), deployment_id.to_owned()));
        self.observed.clone()
    }
}

fn build_observer(
    fake: &Arc<FakeCache>,
    observed: Option<ObservedCache>,
    now: f64,
) -> PromptCacheObserver {
    let cache: Arc<FakeCache> = Arc::clone(fake);
    let cache_port: Arc<dyn ObservationCache> = cache;
    PromptCacheObserver {
        cache: cache_port,
        parser: Arc::new(FakeParser {
            observed,
            seen: Mutex::new(Vec::new()),
        }),
        clock: Arc::new(move || now),
    }
}

fn event<'event>(
    payload: &'event StandardLoggingPayload,
    url: &'event Url,
    headers: &'event HeaderMap,
    response: &'event Json,
    body: &'event [u8],
) -> SuccessEvent<'event> {
    SuccessEvent {
        payload,
        wire: WireRequest { url, headers, body },
        response,
        first_api_call_start_time: starts_at(STARTED),
        status_code: 200,
        cache_hit: None,
        stream: false,
        prompt_cache_response_complete: false,
    }
}

fn decode(value: &Json) -> CacheObservation {
    let Json::Text(text) = value else {
        panic!("the observer stores the serialized observation");
    };
    serde_json::from_str(text).unwrap()
}

#[test]
fn cache_observation_accepts_a_valid_entry() {
    assert!(observation(FINGERPRINT, 100, 300.0).validated().is_some());
}

#[test]
fn cache_observation_accepts_the_zero_time_boundary() {
    let entry = CacheObservation {
        fingerprint: FINGERPRINT.to_owned(),
        cached_tokens: 1,
        observed_at: 0.0,
        expires_at: 0.0,
    };
    assert!(entry.validated().is_some());
}

#[test]
fn cache_observation_rejects_a_fingerprint_one_character_too_long() {
    let entry = CacheObservation {
        fingerprint: format!("{FINGERPRINT}a"),
        cached_tokens: 100,
        observed_at: 0.0,
        expires_at: 300.0,
    };
    assert!(entry.validated().is_none());
}

#[rstest]
#[case::empty_fingerprint("", 100, 0.0, 300.0)]
#[case::short_fingerprint("abc", 100, 0.0, 300.0)]
#[case::uppercase_fingerprint(UPPERCASE_FINGERPRINT, 100, 0.0, 300.0)]
#[case::non_hex_fingerprint(NON_HEX_FINGERPRINT, 100, 0.0, 300.0)]
#[case::zero_cached(FINGERPRINT, 0, 0.0, 300.0)]
#[case::negative_cached(FINGERPRINT, -1, 0.0, 300.0)]
#[case::negative_observed_at(FINGERPRINT, 100, -1.0, 300.0)]
#[case::negative_expires_at(FINGERPRINT, 100, 0.0, -1.0)]
#[case::nan_observed_at(FINGERPRINT, 100, f64::NAN, 300.0)]
#[case::infinite_expires_at(FINGERPRINT, 100, 0.0, f64::INFINITY)]
fn cache_observation_rejects_invalid_entries(
    #[case] fingerprint: &str,
    #[case] cached: i64,
    #[case] observed_at: f64,
    #[case] expires_at: f64,
) {
    let entry = CacheObservation {
        fingerprint: fingerprint.to_owned(),
        cached_tokens: cached,
        observed_at,
        expires_at,
    };
    assert!(entry.validated().is_none());
}

#[test]
fn cache_observation_rejects_unknown_fields() {
    let text = format!(
        r#"{{"fingerprint":"{FINGERPRINT}","cached_tokens":100,"observed_at":0.0,"expires_at":300.0,"scope":"extra"}}"#
    );
    assert!(serde_json::from_str::<CacheObservation>(&text).is_err());
}

#[test]
fn cache_observation_serializes_the_stored_fields() {
    let text = serde_json::to_string(&observation(FINGERPRINT, 100, 300.0)).unwrap();
    assert_eq!(
        text,
        format!(
            r#"{{"fingerprint":"{FINGERPRINT}","cached_tokens":100,"observed_at":500.0,"expires_at":300.0}}"#
        )
    );
}

#[tokio::test]
async fn cache_key_format_scopes_by_scope_and_fingerprint() {
    let fake = FakeCache::new(false);
    let entry = observation(FINGERPRINT, 100, 300.0);
    fake.seed(
        SCOPE,
        FINGERPRINT,
        json(&serde_json::to_string(&entry).unwrap()),
    );
    assert_eq!(read_exact(&fake, SCOPE, FINGERPRINT).await, Some(entry));
    assert!(read_exact(&fake, "scope-b", FINGERPRINT).await.is_none());
}

#[tokio::test]
async fn read_exact_returns_a_stored_entry() {
    let fake = FakeCache::new(false);
    let entry = observation(FINGERPRINT, 100, 300.0);
    fake.seed(
        SCOPE,
        FINGERPRINT,
        Json::Text(serde_json::to_string(&entry).unwrap()),
    );
    assert_eq!(read_exact(&fake, SCOPE, FINGERPRINT).await, Some(entry));
}

#[tokio::test]
async fn read_exact_decodes_a_json_object_entry() {
    let fake = FakeCache::new(false);
    let entry = observation(FINGERPRINT, 100, 300.0);
    fake.seed(
        SCOPE,
        FINGERPRINT,
        json(&serde_json::to_string(&entry).unwrap()),
    );
    assert_eq!(read_exact(&fake, SCOPE, FINGERPRINT).await, Some(entry));
}

#[tokio::test]
async fn read_exact_returns_none_for_a_missing_entry() {
    let fake = FakeCache::new(false);
    assert!(read_exact(&fake, SCOPE, FINGERPRINT).await.is_none());
}

#[tokio::test]
async fn read_exact_returns_none_for_an_invalid_entry() {
    let fake = FakeCache::new(false);
    let entry = observation(FINGERPRINT, 0, 300.0);
    fake.seed(
        SCOPE,
        FINGERPRINT,
        json(&serde_json::to_string(&entry).unwrap()),
    );
    assert!(read_exact(&fake, SCOPE, FINGERPRINT).await.is_none());
}

#[tokio::test]
async fn read_exact_returns_none_for_a_fingerprint_mismatch() {
    let fake = FakeCache::new(false);
    let entry = observation(OTHER_FINGERPRINT, 100, 300.0);
    fake.seed(
        SCOPE,
        FINGERPRINT,
        json(&serde_json::to_string(&entry).unwrap()),
    );
    assert!(read_exact(&fake, SCOPE, FINGERPRINT).await.is_none());
}

#[tokio::test]
async fn read_exact_returns_none_for_a_foreign_value() {
    let fake = FakeCache::new(false);
    fake.seed(SCOPE, FINGERPRINT, Json::Bool(true));
    assert!(read_exact(&fake, SCOPE, FINGERPRINT).await.is_none());
}

#[tokio::test]
async fn lookup_returns_the_fresh_exact_entry() {
    let fake = FakeCache::new(false);
    let entry = observation(FINGERPRINT, 100, 2_500.0);
    fake.seed(
        SCOPE,
        FINGERPRINT,
        json(&serde_json::to_string(&entry).unwrap()),
    );
    let found = lookup(&fake, SCOPE, &evidence(0).prefix, Some(NOW)).await;
    assert_eq!(found, Some(entry));
}

#[tokio::test]
async fn lookup_prefers_a_fresh_exact_entry_over_fresh_lookback_evidence() {
    let fake = FakeCache::new(false);
    let exact = observation(FINGERPRINT, 100, 2_500.0);
    let fresh = observation(OLDER_FINGERPRINT, 900, 2_500.0);
    fake.seed(
        SCOPE,
        FINGERPRINT,
        json(&serde_json::to_string(&exact).unwrap()),
    );
    fake.seed(
        SCOPE,
        OLDER_FINGERPRINT,
        json(&serde_json::to_string(&fresh).unwrap()),
    );
    let found = lookup(&fake, SCOPE, &evidence(0).prefix, Some(NOW)).await;
    assert_eq!(found, Some(exact));
}

#[tokio::test]
async fn lookup_treats_a_lookback_entry_expiring_now_as_stale() {
    let fake = FakeCache::new(false);
    let exact = observation(FINGERPRINT, 100, 1_500.0);
    let expiring = observation(OLDER_FINGERPRINT, 900, NOW);
    fake.seed(
        SCOPE,
        FINGERPRINT,
        json(&serde_json::to_string(&exact).unwrap()),
    );
    fake.seed(
        SCOPE,
        OLDER_FINGERPRINT,
        json(&serde_json::to_string(&expiring).unwrap()),
    );
    let found = lookup(&fake, SCOPE, &evidence(0).prefix, Some(NOW)).await;
    assert_eq!(found, Some(exact));
}

#[tokio::test]
async fn lookup_falls_back_to_a_fresh_lookback_entry() {
    let fake = FakeCache::new(false);
    let entry = observation(OLDER_FINGERPRINT, 100, 2_500.0);
    fake.seed(
        SCOPE,
        OLDER_FINGERPRINT,
        json(&serde_json::to_string(&entry).unwrap()),
    );
    let found = lookup(&fake, SCOPE, &evidence(0).prefix, Some(NOW)).await;
    assert_eq!(found, Some(entry));
}

#[tokio::test]
async fn lookup_prefers_fresh_lookback_evidence_over_a_stale_exact_entry() {
    let fake = FakeCache::new(false);
    let stale = observation(FINGERPRINT, 100, 1_500.0);
    let fresh = observation(OLDER_FINGERPRINT, 900, 2_500.0);
    fake.seed(
        SCOPE,
        FINGERPRINT,
        json(&serde_json::to_string(&stale).unwrap()),
    );
    fake.seed(
        SCOPE,
        OLDER_FINGERPRINT,
        json(&serde_json::to_string(&fresh).unwrap()),
    );
    let found = lookup(&fake, SCOPE, &evidence(0).prefix, Some(NOW)).await;
    assert_eq!(found, Some(fresh));
}

#[tokio::test]
async fn lookup_returns_the_exact_entry_when_every_entry_is_stale() {
    let fake = FakeCache::new(false);
    let stale = observation(FINGERPRINT, 100, 1_500.0);
    let older = observation(OLDER_FINGERPRINT, 900, 1_600.0);
    fake.seed(
        SCOPE,
        FINGERPRINT,
        json(&serde_json::to_string(&stale).unwrap()),
    );
    fake.seed(
        SCOPE,
        OLDER_FINGERPRINT,
        json(&serde_json::to_string(&older).unwrap()),
    );
    let found = lookup(&fake, SCOPE, &evidence(0).prefix, Some(NOW)).await;
    assert_eq!(found, Some(stale));
}

#[tokio::test]
async fn lookup_returns_none_without_evidence() {
    let fake = FakeCache::new(false);
    let found = lookup(&fake, SCOPE, &evidence(0).prefix, Some(NOW)).await;
    assert!(found.is_none());
}

#[tokio::test]
async fn lookup_defaults_the_checked_time_to_the_system_clock() {
    let fake = FakeCache::new(false);
    let stale = observation(FINGERPRINT, 100, 1_500.0);
    let fresh = observation(OLDER_FINGERPRINT, 900, 4_000_000_000.0);
    fake.seed(
        SCOPE,
        FINGERPRINT,
        json(&serde_json::to_string(&stale).unwrap()),
    );
    fake.seed(
        SCOPE,
        OLDER_FINGERPRINT,
        json(&serde_json::to_string(&fresh).unwrap()),
    );
    let found = lookup(&fake, SCOPE, &evidence(0).prefix, None).await;
    assert_eq!(found, Some(fresh));
}

#[tokio::test]
async fn lookup_treats_an_exact_entry_expiring_now_as_stale() {
    let fake = FakeCache::new(false);
    let exact = observation(FINGERPRINT, 100, NOW);
    let fresh = observation(OLDER_FINGERPRINT, 900, 2_500.0);
    fake.seed(
        SCOPE,
        FINGERPRINT,
        json(&serde_json::to_string(&exact).unwrap()),
    );
    fake.seed(
        SCOPE,
        OLDER_FINGERPRINT,
        json(&serde_json::to_string(&fresh).unwrap()),
    );
    let found = lookup(&fake, SCOPE, &evidence(0).prefix, Some(NOW)).await;
    assert_eq!(found, Some(fresh));
}

#[test]
fn observer_init_keeps_the_injected_cache_and_parser() {
    let fake = Arc::new(FakeCache::new(false));
    let cache: Arc<FakeCache> = Arc::clone(&fake);
    let cache_port: Arc<dyn ObservationCache> = cache;
    let parser: Arc<dyn ObservationParser> = Arc::new(FakeParser::default());
    let observer = PromptCacheObserver::new(fake, Arc::clone(&parser));
    assert!(Arc::ptr_eq(&observer.cache, &cache_port));
    assert!(Arc::ptr_eq(&observer.parser, &parser));
}

#[test]
fn observer_init_defaults_the_clock_to_unix_time() {
    let fake = Arc::new(FakeCache::new(false));
    let observer = PromptCacheObserver::new(fake, Arc::new(FakeParser::default()));
    assert!((observer.clock)() > 1_700_000_000.0);
}

#[tokio::test]
async fn event_rejects_a_foreign_call_type() {
    let fake = Arc::new(FakeCache::new(false));
    let observer = build_observer(&fake, Some(evidence(400)), NOW);
    let logged = payload_replacing(
        r#""call_type":"anthropic_messages""#,
        r#""call_type":"completion""#,
    );
    let url = Url::parse(NATIVE_URL).unwrap();
    let headers = HeaderMap::new();
    let response = Json::Null;
    let success = event(&logged, &url, &headers, &response, BODY.as_bytes());
    observer.async_log_success_event(&success).await;
    assert_eq!(fake.write_count(), 0);
}

#[rstest]
#[case::foreign_provider(
    r#""custom_llm_provider":"anthropic""#,
    r#""custom_llm_provider":"openai""#
)]
#[case::absent_provider(
    r#""custom_llm_provider":"anthropic""#,
    r#""custom_llm_provider":null"#
)]
#[case::failed_status(r#""status":"success""#, r#""status":"failure""#)]
#[case::absent_caller(r#""metadata":{"user_api_key_hash":"caller-1"}"#, r#""metadata":{}"#)]
#[case::empty_caller(r#""user_api_key_hash":"caller-1""#, r#""user_api_key_hash":"""#)]
#[case::absent_model_id(r#""model_id":"deployment""#, r#""model_id":null"#)]
#[case::empty_model_id(r#""model_id":"deployment""#, r#""model_id":"""#)]
#[tokio::test]
async fn event_rejects_a_foreign_payload(#[case] from: &str, #[case] to: &str) {
    let fake = Arc::new(FakeCache::new(false));
    let observer = build_observer(&fake, Some(evidence(400)), NOW);
    let logged = payload_replacing(from, to);
    let url = Url::parse(NATIVE_URL).unwrap();
    let headers = HeaderMap::new();
    let response = Json::Null;
    let success = event(&logged, &url, &headers, &response, BODY.as_bytes());
    observer.async_log_success_event(&success).await;
    assert_eq!(fake.write_count(), 0);
}

#[tokio::test]
async fn event_rejects_a_cache_hit() {
    let fake = Arc::new(FakeCache::new(false));
    let observer = build_observer(&fake, Some(evidence(400)), NOW);
    let logged = payload();
    let url = Url::parse(NATIVE_URL).unwrap();
    let headers = HeaderMap::new();
    let response = Json::Null;
    let mut success = event(&logged, &url, &headers, &response, BODY.as_bytes());
    success.cache_hit = Some(true);
    observer.async_log_success_event(&success).await;
    assert_eq!(fake.write_count(), 0);
}

#[rstest]
#[case::unset(None)]
#[case::cache_hit_false(Some(false))]
#[tokio::test]
async fn event_accepts_an_unset_or_false_cache_hit(#[case] cache_hit: Option<bool>) {
    let fake = Arc::new(FakeCache::new(false));
    let observer = build_observer(&fake, Some(evidence(400)), NOW);
    let logged = payload();
    let url = Url::parse(NATIVE_URL).unwrap();
    let headers = HeaderMap::new();
    let response = Json::Null;
    let mut success = event(&logged, &url, &headers, &response, BODY.as_bytes());
    success.cache_hit = cache_hit;
    observer.async_log_success_event(&success).await;
    assert_eq!(fake.write_count(), 1);
}

#[tokio::test]
async fn event_rejects_a_non_200_status() {
    let fake = Arc::new(FakeCache::new(false));
    let observer = build_observer(&fake, Some(evidence(400)), NOW);
    let logged = payload();
    let url = Url::parse(NATIVE_URL).unwrap();
    let headers = HeaderMap::new();
    let response = Json::Null;
    let mut success = event(&logged, &url, &headers, &response, BODY.as_bytes());
    success.status_code = 429;
    observer.async_log_success_event(&success).await;
    assert_eq!(fake.write_count(), 0);
}

#[tokio::test]
async fn event_rejects_an_incomplete_stream() {
    let fake = Arc::new(FakeCache::new(false));
    let observer = build_observer(&fake, Some(evidence(400)), NOW);
    let logged = payload();
    let url = Url::parse(NATIVE_URL).unwrap();
    let headers = HeaderMap::new();
    let response = Json::Null;
    let mut success = event(&logged, &url, &headers, &response, BODY.as_bytes());
    success.stream = true;
    observer.async_log_success_event(&success).await;
    assert_eq!(fake.write_count(), 0);
}

#[tokio::test]
async fn event_accepts_a_complete_stream() {
    let fake = Arc::new(FakeCache::new(false));
    let observer = build_observer(&fake, Some(evidence(400)), NOW);
    let logged = payload();
    let url = Url::parse(NATIVE_URL).unwrap();
    let headers = HeaderMap::new();
    let response = Json::Null;
    let mut success = event(&logged, &url, &headers, &response, BODY.as_bytes());
    success.stream = true;
    success.prompt_cache_response_complete = true;
    observer.async_log_success_event(&success).await;
    assert_eq!(fake.write_count(), 1);
}

#[tokio::test]
async fn event_drops_a_request_that_starts_in_the_future() {
    let fake = Arc::new(FakeCache::new(false));
    let observer = build_observer(&fake, Some(evidence(400)), NOW);
    let logged = payload();
    let url = Url::parse(NATIVE_URL).unwrap();
    let headers = HeaderMap::new();
    let response = Json::Null;
    let mut success = event(&logged, &url, &headers, &response, BODY.as_bytes());
    success.first_api_call_start_time = starts_at(3_000);
    observer.async_log_success_event(&success).await;
    assert_eq!(fake.write_count(), 0);
}

#[tokio::test]
async fn event_accepts_a_request_that_starts_now() {
    let fake = Arc::new(FakeCache::new(false));
    let observer = build_observer(&fake, Some(evidence(400)), NOW);
    let logged = payload();
    let url = Url::parse(NATIVE_URL).unwrap();
    let headers = HeaderMap::new();
    let response = Json::Null;
    let mut success = event(&logged, &url, &headers, &response, BODY.as_bytes());
    success.first_api_call_start_time = starts_at(2_000);
    observer.async_log_success_event(&success).await;
    assert_eq!(fake.write_count(), 1);
}

#[tokio::test]
async fn event_drops_an_unparsable_observation() {
    let fake = Arc::new(FakeCache::new(false));
    let observer = build_observer(&fake, None, NOW);
    let logged = payload();
    let url = Url::parse(NATIVE_URL).unwrap();
    let headers = HeaderMap::new();
    let response = Json::Null;
    let success = event(&logged, &url, &headers, &response, BODY.as_bytes());
    observer.async_log_success_event(&success).await;
    assert_eq!(fake.write_count(), 0);
}

#[tokio::test]
async fn event_passes_the_caller_hash_and_deployment_to_the_parser() {
    let fake = Arc::new(FakeCache::new(false));
    let recorder = Arc::new(FakeParser::default());
    let cache: Arc<FakeCache> = Arc::clone(&fake);
    let cache_port: Arc<dyn ObservationCache> = cache;
    let parser: Arc<FakeParser> = Arc::clone(&recorder);
    let parser_port: Arc<dyn ObservationParser> = parser;
    let observer = PromptCacheObserver {
        cache: cache_port,
        parser: parser_port,
        clock: Arc::new(|| NOW),
    };
    let logged = payload();
    let url = Url::parse(NATIVE_URL).unwrap();
    let headers = HeaderMap::new();
    let response = Json::Null;
    let success = event(&logged, &url, &headers, &response, BODY.as_bytes());
    observer.async_log_success_event(&success).await;
    assert_eq!(
        *recorder.seen.lock().unwrap(),
        [("caller-1".to_owned(), "deployment".to_owned())]
    );
}

#[tokio::test]
async fn read_only_hit_refreshes_matching_evidence() {
    let fake = Arc::new(FakeCache::new(false));
    let previous = observation(FINGERPRINT, 500, 1_500.0);
    fake.seed(
        SCOPE,
        FINGERPRINT,
        json(&serde_json::to_string(&previous).unwrap()),
    );
    let observer = build_observer(&fake, Some(evidence(0)), NOW);
    let logged = payload();
    let url = Url::parse(NATIVE_URL).unwrap();
    let headers = HeaderMap::new();
    let response = Json::Null;
    let success = event(&logged, &url, &headers, &response, BODY.as_bytes());
    observer.async_log_success_event(&success).await;
    assert_eq!(fake.write_count(), 1);
}

#[tokio::test]
async fn read_only_hit_needs_existing_evidence() {
    let fake = Arc::new(FakeCache::new(false));
    let observer = build_observer(&fake, Some(evidence(0)), NOW);
    let logged = payload();
    let url = Url::parse(NATIVE_URL).unwrap();
    let headers = HeaderMap::new();
    let response = Json::Null;
    let success = event(&logged, &url, &headers, &response, BODY.as_bytes());
    observer.async_log_success_event(&success).await;
    assert_eq!(fake.write_count(), 0);
}

#[tokio::test]
async fn read_only_hit_rejects_changed_cached_tokens() {
    let fake = Arc::new(FakeCache::new(false));
    let previous = observation(FINGERPRINT, 400, 1_500.0);
    fake.seed(
        SCOPE,
        FINGERPRINT,
        json(&serde_json::to_string(&previous).unwrap()),
    );
    let observer = build_observer(&fake, Some(evidence(0)), NOW);
    let logged = payload();
    let url = Url::parse(NATIVE_URL).unwrap();
    let headers = HeaderMap::new();
    let response = Json::Null;
    let success = event(&logged, &url, &headers, &response, BODY.as_bytes());
    observer.async_log_success_event(&success).await;
    assert_eq!(fake.write_count(), 0);
}

#[tokio::test]
async fn read_only_hit_rejects_a_different_fingerprint() {
    let fake = Arc::new(FakeCache::new(false));
    let previous = observation(OTHER_FINGERPRINT, 500, 1_500.0);
    fake.seed(
        SCOPE,
        FINGERPRINT,
        json(&serde_json::to_string(&previous).unwrap()),
    );
    let observer = build_observer(&fake, Some(evidence(0)), NOW);
    let logged = payload();
    let url = Url::parse(NATIVE_URL).unwrap();
    let headers = HeaderMap::new();
    let response = Json::Null;
    let success = event(&logged, &url, &headers, &response, BODY.as_bytes());
    observer.async_log_success_event(&success).await;
    assert_eq!(fake.write_count(), 0);
}

#[tokio::test]
async fn read_only_hit_skips_the_prior_check_with_creation_tokens() {
    let fake = Arc::new(FakeCache::new(false));
    let observer = build_observer(&fake, Some(evidence(500)), NOW);
    let logged = payload();
    let url = Url::parse(NATIVE_URL).unwrap();
    let headers = HeaderMap::new();
    let response = Json::Null;
    let success = event(&logged, &url, &headers, &response, BODY.as_bytes());
    observer.async_log_success_event(&success).await;
    assert_eq!(fake.write_count(), 1);
}

#[tokio::test]
async fn storage_writes_the_observation_with_the_retention_ttl() {
    let fake = Arc::new(FakeCache::new(false));
    let observer = build_observer(&fake, Some(evidence(500)), NOW);
    let logged = payload();
    let url = Url::parse(NATIVE_URL).unwrap();
    let headers = HeaderMap::new();
    let response = Json::Null;
    let success = event(&logged, &url, &headers, &response, BODY.as_bytes());
    observer.async_log_success_event(&success).await;
    let writes = fake.writes.lock().unwrap();
    assert_eq!(writes.len(), 1);
    assert_eq!(writes[0].key, entry_key(SCOPE, FINGERPRINT));
    assert_eq!(writes[0].ttl.to_bits(), 86_400.0_f64.to_bits());
    assert!(!writes[0].local_only);
    let stored = decode(&writes[0].value);
    assert_eq!(stored.fingerprint, FINGERPRINT);
    assert_eq!(stored.cached_tokens, 500);
    assert_eq!(stored.observed_at.to_bits(), NOW.to_bits());
    assert_eq!(stored.expires_at.to_bits(), 1_300.0_f64.to_bits());
}

#[rstest]
#[case::five_minutes(300, 1_300.0)]
#[case::one_hour(3_600, 4_600.0)]
#[tokio::test]
async fn storage_expires_at_the_request_start_plus_the_ttl(
    #[case] ttl_seconds: u32,
    #[case] expires_at: f64,
) {
    let fake = Arc::new(FakeCache::new(false));
    let mut result = evidence(500);
    result.ttl_seconds = ttl_seconds;
    let observer = build_observer(&fake, Some(result), NOW);
    let logged = payload();
    let url = Url::parse(NATIVE_URL).unwrap();
    let headers = HeaderMap::new();
    let response = Json::Null;
    let success = event(&logged, &url, &headers, &response, BODY.as_bytes());
    observer.async_log_success_event(&success).await;
    let writes = fake.writes.lock().unwrap();
    let stored = decode(&writes[0].value);
    assert_eq!(stored.expires_at.to_bits(), expires_at.to_bits());
}

#[rstest]
#[case::without_redis(false)]
#[case::with_redis(true)]
#[tokio::test]
async fn storage_copies_locally_only_with_redis(#[case] redis: bool) {
    let fake = Arc::new(FakeCache::new(redis));
    let observer = build_observer(&fake, Some(evidence(500)), NOW);
    let logged = payload();
    let url = Url::parse(NATIVE_URL).unwrap();
    let headers = HeaderMap::new();
    let response = Json::Null;
    let success = event(&logged, &url, &headers, &response, BODY.as_bytes());
    observer.async_log_success_event(&success).await;
    let writes = fake.writes.lock().unwrap();
    let local: Vec<&Write> = writes.iter().filter(|write| write.local_only).collect();
    if redis {
        assert_eq!(writes.len(), 2);
        assert_eq!(local.len(), 1);
        assert_eq!(local[0].ttl.to_bits(), 1.0_f64.to_bits());
        assert_eq!(local[0].key, entry_key(SCOPE, FINGERPRINT));
    } else {
        assert_eq!(writes.len(), 1);
        assert!(local.is_empty());
    }
}

#[tokio::test]
async fn storage_drops_an_observation_with_a_non_finite_clock() {
    let fake = Arc::new(FakeCache::new(false));
    let observer = build_observer(&fake, Some(evidence(500)), f64::NAN);
    let logged = payload();
    let url = Url::parse(NATIVE_URL).unwrap();
    let headers = HeaderMap::new();
    let response = Json::Null;
    let success = event(&logged, &url, &headers, &response, BODY.as_bytes());
    observer.async_log_success_event(&success).await;
    assert_eq!(fake.write_count(), 0);
}
