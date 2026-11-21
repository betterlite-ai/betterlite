#![cfg(test)]

mod support;

use std::time::Duration;

use betterlite_error::rate_limit::RateLimitCategory;
use betterlite_error::rate_limit::RateLimitDimension;
use betterlite_redact::structured::Payload;
use rstest::rstest;

use self::support::FakeCache;
use self::support::LIMIT_FIVE;
use self::support::LIMIT_FLOAT;
use self::support::LIMIT_HUGE;
use self::support::LIMIT_INVALID;
use self::support::LIMIT_LIST;
use self::support::LIMIT_NULL;
use self::support::LIMIT_ONE;
use self::support::LIMIT_TEXT;
use self::support::LIMIT_TRUE;
use self::support::LIMIT_TWO;
use self::support::LIMIT_ZERO;
use self::support::NO_LIMIT;
use self::support::NO_PARAMS;
use self::support::ScriptMode;
use self::support::agent_key;
use self::support::agent_registry_with;
use self::support::expected_detail;
use self::support::iteration_handler;
use self::support::iteration_handler_at;
use self::support::json;
use self::support::key_metadata;
use self::support::memory_key;
use self::support::rejection;
use self::support::session_data;
use crate::limiter_records::CacheCounterValue;
use crate::session_iteration_limits::INCREMENT_ITERATIONS_SCRIPT;
use crate::session_iteration_limits::KeyIterations;
use crate::session_iteration_limits::error::SessionIterationLimitError;

#[tokio::test]
async fn missing_session_id() {
    let cache = FakeCache::without_redis();
    cache.preload_memory(&memory_key("s1"), 4);
    let handler = iteration_handler(cache.clone());
    let registry = agent_registry_with(&[LIMIT_ONE]);
    let data = json(r#"{"model":"gpt-4o","metadata":{"other":true}}"#);

    handler
        .async_pre_call_hook(&registry, None, agent_key(), &data)
        .await
        .unwrap();

    assert_eq!(cache.memory(&memory_key("s1")), Some(4));
}

#[tokio::test]
async fn missing_max_iterations() {
    let cache = FakeCache::without_redis();
    let handler = iteration_handler(cache.clone());
    let registry = agent_registry_with(&[NO_LIMIT]);
    let data = session_data("s1");

    handler
        .async_pre_call_hook(&registry, None, agent_key(), &data)
        .await
        .unwrap();

    assert_eq!(cache.memory(&memory_key("s1")), None);
    assert!(cache.memory_keys().is_empty());
}

#[tokio::test]
async fn zero_limit_rejects_the_first_call() {
    let cache = FakeCache::without_redis();
    let handler = iteration_handler(cache);
    let registry = agent_registry_with(&[LIMIT_ZERO]);
    let data = session_data("s1");

    let error = rejection(
        handler
            .async_pre_call_hook(&registry, None, agent_key(), &data)
            .await
            .unwrap_err(),
    );

    assert_eq!(error.detail, Payload::String(expected_detail("s1", 1, 0)));
}

#[tokio::test]
async fn first_call_passes() {
    let cache = FakeCache::without_redis();
    let handler = iteration_handler(cache.clone());
    let registry = agent_registry_with(&[LIMIT_TWO]);
    let data = session_data("s1");

    handler
        .async_pre_call_hook(&registry, None, agent_key(), &data)
        .await
        .unwrap();

    assert_eq!(cache.memory(&memory_key("s1")), Some(1));
    assert_eq!(cache.memory_ttl(&memory_key("s1")), Some(3600));
}

#[tokio::test]
async fn second_call_within_limit() {
    let cache = FakeCache::without_redis();
    cache.preload_memory(&memory_key("s1"), 1);
    let handler = iteration_handler(cache.clone());
    let registry = agent_registry_with(&[LIMIT_TWO]);
    let data = session_data("s1");

    handler
        .async_pre_call_hook(&registry, None, agent_key(), &data)
        .await
        .unwrap();

    assert_eq!(cache.memory(&memory_key("s1")), Some(2));
}

#[tokio::test]
async fn third_call_exceeds_limit() {
    let cache = FakeCache::without_redis();
    cache.preload_memory(&memory_key("s1"), 2);
    let handler = iteration_handler(cache.clone());
    let registry = agent_registry_with(&[LIMIT_TWO]);
    let data = session_data("s1");

    let error = rejection(
        handler
            .async_pre_call_hook(&registry, None, agent_key(), &data)
            .await
            .unwrap_err(),
    );

    assert_eq!(cache.memory(&memory_key("s1")), Some(3));
    assert_eq!(
        error.rate_limit_type,
        Some(RateLimitDimension::MaxIterations)
    );
    assert_eq!(error.category, RateLimitCategory::LitellmRateLimit);
    assert_eq!(error.model, "gpt-4o");
    assert_eq!(error.llm_provider, "openai");
    assert_eq!(error.detail, Payload::String(expected_detail("s1", 3, 2)));
}

#[tokio::test]
async fn counter_increments_on_rejection() {
    let cache = FakeCache::without_redis();
    cache.preload_memory(&memory_key("s1"), 5);
    let handler = iteration_handler(cache.clone());
    let registry = agent_registry_with(&[LIMIT_ONE]);
    let data = session_data("s1");

    rejection(
        handler
            .async_pre_call_hook(&registry, None, agent_key(), &data)
            .await
            .unwrap_err(),
    );

    assert_eq!(cache.memory(&memory_key("s1")), Some(6));
}

#[tokio::test]
async fn redis_failure_fallback_increments() {
    let cache = FakeCache::with_redis();
    cache.set_script_mode(ScriptMode::Fails);
    cache.preload_memory(&memory_key("s1"), 1);
    let handler = iteration_handler(cache.clone());
    let registry = agent_registry_with(&[LIMIT_TWO]);
    let data = session_data("s1");

    handler
        .async_pre_call_hook(&registry, None, agent_key(), &data)
        .await
        .unwrap();

    assert_eq!(cache.memory(&memory_key("s1")), Some(2));
    assert_eq!(cache.redis_count(&memory_key("s1")), None);
}

#[tokio::test]
async fn error_detail_includes_session_and_counts() {
    let cache = FakeCache::without_redis();
    cache.preload_memory(&memory_key("sess-9"), 2);
    let handler = iteration_handler(cache);
    let registry = agent_registry_with(&[LIMIT_TWO]);
    let data = session_data("sess-9");

    let error = rejection(
        handler
            .async_pre_call_hook(&registry, None, agent_key(), &data)
            .await
            .unwrap_err(),
    );

    let detail = expected_detail("sess-9", 3, 2);
    assert_eq!(error.detail, Payload::String(detail.clone()));
    assert_eq!(error.message, detail);
}

#[tokio::test]
async fn agent_params_wins_over_metadata() {
    let cache = FakeCache::without_redis();
    cache.preload_memory(&memory_key("s1"), 1);
    let handler = iteration_handler(cache.clone());
    let registry = agent_registry_with(&[LIMIT_TWO]);
    let metadata = key_metadata(r#"{"max_iterations":1}"#);
    let key = KeyIterations {
        agent_id: Some(support::AGENT_ID),
        metadata: Some(&metadata),
    };
    let data = session_data("s1");

    handler
        .async_pre_call_hook(&registry, None, key, &data)
        .await
        .unwrap();

    assert_eq!(cache.memory(&memory_key("s1")), Some(2));
}

#[tokio::test]
async fn unregistered_agent_falls_back_to_key_metadata() {
    let cache = FakeCache::without_redis();
    cache.preload_memory(&memory_key("s1"), 1);
    let handler = iteration_handler(cache);
    let registry = agent_registry_with(&[]);
    let metadata = key_metadata(r#"{"max_iterations":1}"#);
    let key = KeyIterations {
        agent_id: Some(support::AGENT_ID),
        metadata: Some(&metadata),
    };
    let data = session_data("s1");

    let error = rejection(
        handler
            .async_pre_call_hook(&registry, None, key, &data)
            .await
            .unwrap_err(),
    );

    assert_eq!(error.detail, Payload::String(expected_detail("s1", 2, 1)));
}

#[tokio::test]
async fn cache_key_uses_hash_tag() {
    let cache = FakeCache::without_redis();
    let handler = iteration_handler(cache.clone());
    let registry = agent_registry_with(&[LIMIT_TWO]);
    let data = session_data("a:b");

    handler
        .async_pre_call_hook(&registry, None, agent_key(), &data)
        .await
        .unwrap();

    assert_eq!(
        cache.memory_keys(),
        vec![String::from("{session_iterations:a:b}:count")]
    );
}

#[tokio::test]
async fn ttl_expires_counter() {
    let cache = FakeCache::without_redis();
    let handler = iteration_handler_at(cache.clone(), Duration::from_secs(60));
    let registry = agent_registry_with(&[LIMIT_TWO]);
    let data = session_data("s1");

    handler
        .async_pre_call_hook(&registry, None, agent_key(), &data)
        .await
        .unwrap();
    cache.advance(Duration::from_secs(59));
    handler
        .async_pre_call_hook(&registry, None, agent_key(), &data)
        .await
        .unwrap();
    assert_eq!(cache.memory(&memory_key("s1")), Some(2));
    assert_eq!(cache.memory_ttl(&memory_key("s1")), Some(60));

    cache.advance(Duration::from_secs(61));
    handler
        .async_pre_call_hook(&registry, None, agent_key(), &data)
        .await
        .unwrap();

    assert_eq!(cache.memory(&memory_key("s1")), Some(1));
}

#[rstest]
#[case::absent_value(LIMIT_NULL)]
#[case::absent_entry(NO_LIMIT)]
#[case::absent_params(NO_PARAMS)]
#[tokio::test]
async fn agent_without_a_limit_falls_back_to_key_metadata(#[case] record: &str) {
    let cache = FakeCache::without_redis();
    cache.preload_memory(&memory_key("s1"), 1);
    let handler = iteration_handler(cache);
    let registry = agent_registry_with(&[record]);
    let metadata = key_metadata(r#"{"max_iterations":1}"#);
    let key = KeyIterations {
        agent_id: Some(support::AGENT_ID),
        metadata: Some(&metadata),
    };
    let data = session_data("s1");

    let error = rejection(
        handler
            .async_pre_call_hook(&registry, None, key, &data)
            .await
            .unwrap_err(),
    );

    assert_eq!(error.detail, Payload::String(expected_detail("s1", 2, 1)));
}

#[rstest]
#[case::text(LIMIT_TEXT, 2)]
#[case::boolean(LIMIT_TRUE, 1)]
#[case::float(LIMIT_FLOAT, 2)]
#[tokio::test]
async fn agent_iterations_coerce_like_int(#[case] record: &str, #[case] limit: i64) {
    let cache = FakeCache::without_redis();
    cache.preload_memory(&memory_key("s1"), 2);
    let handler = iteration_handler(cache);
    let registry = agent_registry_with(&[record]);
    let data = session_data("s1");

    let error = rejection(
        handler
            .async_pre_call_hook(&registry, None, agent_key(), &data)
            .await
            .unwrap_err(),
    );

    assert_eq!(
        error.detail,
        Payload::String(expected_detail("s1", 3, limit))
    );
}

#[rstest]
#[case::number(r#"{"max_iterations":2}"#, 2)]
#[case::text(r#"{"max_iterations":"2"}"#, 2)]
#[case::boolean(r#"{"max_iterations":true}"#, 1)]
#[case::float(r#"{"max_iterations":2.7}"#, 2)]
#[tokio::test]
async fn key_metadata_coerce_like_int(#[case] text: &str, #[case] limit: i64) {
    let cache = FakeCache::without_redis();
    cache.preload_memory(&memory_key("s1"), 2);
    let handler = iteration_handler(cache);
    let registry = agent_registry_with(&[NO_LIMIT]);
    let metadata = key_metadata(text);
    let key = KeyIterations {
        agent_id: Some(support::AGENT_ID),
        metadata: Some(&metadata),
    };
    let data = session_data("s1");

    let error = rejection(
        handler
            .async_pre_call_hook(&registry, None, key, &data)
            .await
            .unwrap_err(),
    );

    assert_eq!(
        error.detail,
        Payload::String(expected_detail("s1", 3, limit))
    );
}

#[tokio::test]
async fn null_key_max_iterations_reads_as_absent() {
    let cache = FakeCache::without_redis();
    let handler = iteration_handler(cache.clone());
    let registry = agent_registry_with(&[NO_LIMIT]);
    let metadata = key_metadata(r#"{"max_iterations":null}"#);
    let key = KeyIterations {
        agent_id: Some(support::AGENT_ID),
        metadata: Some(&metadata),
    };
    let data = session_data("s1");

    handler
        .async_pre_call_hook(&registry, None, key, &data)
        .await
        .unwrap();

    assert!(cache.memory_keys().is_empty());
}

#[rstest]
#[case::text(LIMIT_INVALID)]
#[case::list(LIMIT_LIST)]
#[tokio::test]
async fn invalid_agent_max_iterations_is_a_typed_error(#[case] record: &str) {
    let cache = FakeCache::without_redis();
    let handler = iteration_handler(cache.clone());
    let registry = agent_registry_with(&[record]);
    let data = session_data("s1");

    let error = handler
        .async_pre_call_hook(&registry, None, agent_key(), &data)
        .await
        .unwrap_err();

    let SessionIterationLimitError::InvalidAgentIterations { .. } = error else {
        panic!("expected an invalid agent max_iterations");
    };
    assert!(cache.memory_keys().is_empty());
}

#[rstest]
#[case::text(r#"{"max_iterations":"soon"}"#)]
#[case::list(r#"{"max_iterations":[]}"#)]
#[case::object(r#"{"max_iterations":{}}"#)]
#[tokio::test]
async fn invalid_key_max_iterations_is_a_typed_error(#[case] text: &str) {
    let cache = FakeCache::without_redis();
    let handler = iteration_handler(cache.clone());
    let registry = agent_registry_with(&[NO_LIMIT]);
    let metadata = key_metadata(text);
    let key = KeyIterations {
        agent_id: Some(support::AGENT_ID),
        metadata: Some(&metadata),
    };
    let data = session_data("s1");

    let error = handler
        .async_pre_call_hook(&registry, None, key, &data)
        .await
        .unwrap_err();

    let SessionIterationLimitError::InvalidKeyIterations = error else {
        panic!("expected an invalid key max_iterations");
    };
    assert!(cache.memory_keys().is_empty());
}

#[tokio::test]
async fn max_iterations_beyond_i64_is_a_typed_error() {
    let cache = FakeCache::without_redis();
    let handler = iteration_handler(cache.clone());
    let registry = agent_registry_with(&[LIMIT_HUGE]);
    let data = session_data("s1");

    let error = handler
        .async_pre_call_hook(&registry, None, agent_key(), &data)
        .await
        .unwrap_err();

    let SessionIterationLimitError::InvalidAgentIterations { .. } = error else {
        panic!("expected an invalid agent max_iterations");
    };
    assert!(cache.memory_keys().is_empty());
}

#[tokio::test]
async fn key_max_iterations_beyond_i64_is_a_typed_error() {
    let cache = FakeCache::without_redis();
    let handler = iteration_handler(cache);
    let registry = agent_registry_with(&[NO_LIMIT]);
    let metadata = key_metadata(r#"{"max_iterations":9223372036854775808}"#);
    let key = KeyIterations {
        agent_id: Some(support::AGENT_ID),
        metadata: Some(&metadata),
    };
    let data = session_data("s1");

    let error = handler
        .async_pre_call_hook(&registry, None, key, &data)
        .await
        .unwrap_err();

    let SessionIterationLimitError::InvalidKeyIterations = error else {
        panic!("expected an invalid key max_iterations");
    };
}

#[tokio::test]
async fn numeric_session_id_names_the_text_bucket() {
    let cache = FakeCache::without_redis();
    let handler = iteration_handler(cache.clone());
    let registry = agent_registry_with(&[LIMIT_TWO]);
    let data = json(r#"{"model":"gpt-4o","metadata":{"session_id":5}}"#);

    handler
        .async_pre_call_hook(&registry, None, agent_key(), &data)
        .await
        .unwrap();

    assert_eq!(cache.memory(&memory_key("5")), Some(1));
}

#[rstest]
#[case::null("null")]
#[case::boolean("true")]
#[case::list(r#"["s"]"#)]
#[case::object(r#"{"id":"s"}"#)]
#[tokio::test]
async fn non_text_session_id_falls_through_to_litellm_metadata(#[case] id: &str) {
    let cache = FakeCache::without_redis();
    let handler = iteration_handler(cache.clone());
    let registry = agent_registry_with(&[LIMIT_TWO]);
    let data = json(&format!(
        r#"{{"model":"gpt-4o","metadata":{{"session_id":{id}}},"litellm_metadata":{{"session_id":"s1"}}}}"#
    ));

    handler
        .async_pre_call_hook(&registry, None, agent_key(), &data)
        .await
        .unwrap();

    assert_eq!(cache.memory(&memory_key("s1")), Some(1));
}

#[tokio::test]
async fn session_id_falls_back_to_litellm_metadata() {
    let cache = FakeCache::without_redis();
    let handler = iteration_handler(cache.clone());
    let registry = agent_registry_with(&[LIMIT_TWO]);
    let data = json(r#"{"model":"gpt-4o","litellm_metadata":{"session_id":"s1"}}"#);

    handler
        .async_pre_call_hook(&registry, None, agent_key(), &data)
        .await
        .unwrap();

    assert_eq!(cache.memory(&memory_key("s1")), Some(1));
}

#[tokio::test]
async fn session_id_metadata_wins_over_litellm_metadata() {
    let cache = FakeCache::without_redis();
    let handler = iteration_handler(cache.clone());
    let registry = agent_registry_with(&[LIMIT_TWO]);
    let data = json(
        r#"{"model":"gpt-4o","metadata":{"session_id":"first"},"litellm_metadata":{"session_id":"second"}}"#,
    );

    handler
        .async_pre_call_hook(&registry, None, agent_key(), &data)
        .await
        .unwrap();

    assert_eq!(cache.memory(&memory_key("first")), Some(1));
    assert_eq!(cache.memory(&memory_key("second")), None);
}

#[tokio::test]
async fn text_metadata_bucket_reads_as_absent() {
    let cache = FakeCache::without_redis();
    let handler = iteration_handler(cache.clone());
    let registry = agent_registry_with(&[LIMIT_TWO]);
    let data = json(r#"{"model":"gpt-4o","metadata":"s1","litellm_metadata":{"session_id":"s1"}}"#);

    handler
        .async_pre_call_hook(&registry, None, agent_key(), &data)
        .await
        .unwrap();

    assert_eq!(cache.memory(&memory_key("s1")), Some(1));
}

#[tokio::test]
async fn script_count_wins_over_the_local_counter() {
    let cache = FakeCache::with_redis();
    cache.preload_redis(&memory_key("s1"), 4);
    cache.preload_memory(&memory_key("s1"), 1);
    let handler = iteration_handler(cache.clone());
    let registry = agent_registry_with(&[LIMIT_FIVE]);
    let data = session_data("s1");

    handler
        .async_pre_call_hook(&registry, None, agent_key(), &data)
        .await
        .unwrap();

    assert_eq!(cache.redis_count(&memory_key("s1")), Some(5));
    // the script expires only a counter it created, so a key that already existed keeps no new ttl
    assert_eq!(cache.redis_ttl(&memory_key("s1")), None);
    assert_eq!(cache.memory(&memory_key("s1")), Some(1));
}

#[tokio::test]
async fn script_count_decides_the_rejection() {
    let cache = FakeCache::with_redis();
    cache.preload_redis(&memory_key("s1"), 2);
    let handler = iteration_handler(cache.clone());
    let registry = agent_registry_with(&[LIMIT_TWO]);
    let data = session_data("s1");

    let error = rejection(
        handler
            .async_pre_call_hook(&registry, None, agent_key(), &data)
            .await
            .unwrap_err(),
    );

    assert_eq!(error.detail, Payload::String(expected_detail("s1", 3, 2)));
    assert_eq!(cache.memory(&memory_key("s1")), None);
}

#[tokio::test]
async fn script_receives_the_configured_ttl() {
    let cache = FakeCache::with_redis();
    let handler = iteration_handler_at(cache.clone(), Duration::from_secs(60));
    let registry = agent_registry_with(&[LIMIT_TWO]);
    let data = session_data("s1");

    handler
        .async_pre_call_hook(&registry, None, agent_key(), &data)
        .await
        .unwrap();

    assert_eq!(cache.redis_count(&memory_key("s1")), Some(1));
    assert_eq!(cache.redis_ttl(&memory_key("s1")), Some(60));
}

#[tokio::test]
async fn script_reply_that_is_not_a_count_falls_back_to_memory() {
    let cache = FakeCache::with_redis();
    cache.set_script_mode(ScriptMode::AnswersNil);
    cache.preload_memory(&memory_key("s1"), 1);
    let handler = iteration_handler(cache.clone());
    let registry = agent_registry_with(&[LIMIT_TWO]);
    let data = session_data("s1");

    handler
        .async_pre_call_hook(&registry, None, agent_key(), &data)
        .await
        .unwrap();

    assert_eq!(cache.memory(&memory_key("s1")), Some(2));
}

#[tokio::test]
async fn local_count_text_is_read_like_int() {
    let cache = FakeCache::without_redis();
    cache.preload_memory_value(&memory_key("s1"), CacheCounterValue::Text("2".to_owned()));
    let handler = iteration_handler(cache.clone());
    let registry = agent_registry_with(&[LIMIT_FIVE]);
    let data = session_data("s1");

    handler
        .async_pre_call_hook(&registry, None, agent_key(), &data)
        .await
        .unwrap();

    assert_eq!(cache.memory(&memory_key("s1")), Some(3));
}

#[tokio::test]
async fn local_count_that_is_not_an_integer_is_a_typed_error() {
    let cache = FakeCache::without_redis();
    cache.preload_memory_value(
        &memory_key("s1"),
        CacheCounterValue::Text("soon".to_owned()),
    );
    let handler = iteration_handler(cache);
    let registry = agent_registry_with(&[LIMIT_FIVE]);
    let data = session_data("s1");

    let error = handler
        .async_pre_call_hook(&registry, None, agent_key(), &data)
        .await
        .unwrap_err();

    let SessionIterationLimitError::LocalCount(_) = error else {
        panic!("expected a local count that is not an integer");
    };
}

#[tokio::test]
async fn key_metadata_limit_applies_without_an_agent() {
    let cache = FakeCache::without_redis();
    cache.preload_memory(&memory_key("s1"), 1);
    let handler = iteration_handler(cache);
    let registry = agent_registry_with(&[LIMIT_FIVE]);
    let metadata = key_metadata(r#"{"max_iterations":1}"#);
    let key = KeyIterations {
        agent_id: None,
        metadata: Some(&metadata),
    };
    let data = session_data("s1");

    let error = rejection(
        handler
            .async_pre_call_hook(&registry, None, key, &data)
            .await
            .unwrap_err(),
    );

    assert_eq!(error.detail, Payload::String(expected_detail("s1", 2, 1)));
}

#[tokio::test]
async fn unknown_model_falls_back_to_litellm_proxy() {
    let cache = FakeCache::without_redis();
    cache.preload_memory(&memory_key("s1"), 1);
    let handler = iteration_handler(cache);
    let registry = agent_registry_with(&[LIMIT_ONE]);
    let data = json(r#"{"model":"mystery-model","metadata":{"session_id":"s1"}}"#);

    let error = rejection(
        handler
            .async_pre_call_hook(&registry, None, agent_key(), &data)
            .await
            .unwrap_err(),
    );

    assert_eq!(error.model, "mystery-model");
    assert_eq!(error.llm_provider, "litellm_proxy");
}

#[tokio::test]
async fn missing_model_leaves_the_model_empty() {
    let cache = FakeCache::without_redis();
    cache.preload_memory(&memory_key("s1"), 1);
    let handler = iteration_handler(cache);
    let registry = agent_registry_with(&[LIMIT_ONE]);
    let data = json(r#"{"metadata":{"session_id":"s1"}}"#);

    let error = rejection(
        handler
            .async_pre_call_hook(&registry, None, agent_key(), &data)
            .await
            .unwrap_err(),
    );

    assert_eq!(error.model, "");
    assert_eq!(error.llm_provider, "litellm_proxy");
}

#[test]
fn structure_handler_with_redis_keeps_the_litellm_script() {
    let handler = iteration_handler(FakeCache::with_redis());

    let source = handler
        .increment_script
        .as_ref()
        .map(|script| script.source());
    assert_eq!(source, Some(INCREMENT_ITERATIONS_SCRIPT));
    assert_eq!(handler.ttl, Duration::from_secs(3600));
}

#[test]
fn structure_handler_without_redis_holds_no_script() {
    let handler = iteration_handler(FakeCache::without_redis());

    assert!(handler.increment_script.is_none());
}

#[test]
fn increment_script_matches_litellms_incr_and_first_write_expire() {
    assert!(INCREMENT_ITERATIONS_SCRIPT.contains("redis.call('INCR', key)"));
    assert!(INCREMENT_ITERATIONS_SCRIPT.contains("if current == 1 then"));
    assert!(INCREMENT_ITERATIONS_SCRIPT.contains("redis.call('EXPIRE', key, ttl)"));
    assert!(INCREMENT_ITERATIONS_SCRIPT.contains("return current"));
}
