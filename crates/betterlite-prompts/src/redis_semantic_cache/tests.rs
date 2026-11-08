#![cfg(test)]

mod fake_redis;
mod fixtures;
mod index_live;
mod lookup_live;
mod store_live;

use std::str;
use std::sync::Arc;

use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::Number;

use self::fixtures::{
    FakeConnector, FakeEmbedder, FakeIndex, InfoShape, PROBE, TIMESTAMP, VECTOR, alternative_error,
    cache_with, cache_with_shared, command_args, empty_env, hit, info_reply, info_reply_shaped,
    kwargs_with_message, options, stored_cache, text_args, words,
};
use crate::arguments::json::{Json, Object};
use crate::redis_semantic_cache::options::RedisEnv;
use crate::redis_semantic_cache::ports::{CacheEntryFields, IndexSchema, SemanticSearchQuery};
use crate::redis_semantic_cache::{RedisSemanticCache, SIMILARITY_FIELD, SemanticCacheKwargs};

#[test]
fn options_missing_threshold_fails_with_litellms_message() {
    let mut settings = options();
    settings.similarity_threshold = None;
    let result = RedisSemanticCache::new(
        settings,
        empty_env(),
        Arc::new(FakeEmbedder::new()),
        Arc::new(FakeConnector::new(Arc::new(FakeIndex::default()))),
    );
    let Err(error) = result else {
        panic!("a cache without a threshold must fail");
    };
    assert_eq!(
        error.to_string(),
        "similarity_threshold must be provided, passed None"
    );
}

#[test]
fn options_env_values_fill_the_missing_host_port_and_password() {
    let mut settings = options();
    settings.host = None;
    settings.port = None;
    settings.password = None;
    let env = RedisEnv {
        redis_host: Some("redis.internal".to_owned()),
        redis_port: Some("6380".to_owned()),
        redis_password: Some(SecretString::from("envpw")),
    };
    let cache = RedisSemanticCache::new(
        settings,
        env,
        Arc::new(FakeEmbedder::new()),
        Arc::new(FakeConnector::new(Arc::new(FakeIndex::default()))),
    )
    .expect("the environment settings");
    assert_eq!(
        cache.redis_url.expose_secret(),
        "redis://:envpw@redis.internal:6380"
    );
}

#[rstest]
#[case::host("REDIS_HOST", None, Some("6380"), Some("pw"))]
#[case::port("REDIS_PORT", Some("localhost"), None, Some("pw"))]
#[case::password("REDIS_PASSWORD", Some("localhost"), Some("6380"), None)]
fn options_missing_configuration_names_the_variable(
    #[case] variable: &str,
    #[case] host: Option<&str>,
    #[case] port: Option<&str>,
    #[case] password: Option<&str>,
) {
    let mut settings = options();
    settings.host = host.map(str::to_owned);
    settings.port = port.map(str::to_owned);
    settings.password = password.map(SecretString::from);
    let result = RedisSemanticCache::new(
        settings,
        empty_env(),
        Arc::new(FakeEmbedder::new()),
        Arc::new(FakeConnector::new(Arc::new(FakeIndex::default()))),
    );
    let Err(error) = result else {
        panic!("missing configuration must fail");
    };
    assert_eq!(
        error.to_string(),
        format!(
            "Missing required Redis configuration: {variable}. Provide {variable} or redis_url."
        )
    );
}

#[test]
fn options_explicit_and_empty_values_lose_to_a_non_empty_environment_value() {
    let mut settings = options();
    settings.host = Some(String::new());
    let env = RedisEnv {
        redis_host: Some("envhost".to_owned()),
        redis_port: None,
        redis_password: None,
    };
    let cache = RedisSemanticCache::new(
        settings,
        env,
        Arc::new(FakeEmbedder::new()),
        Arc::new(FakeConnector::new(Arc::new(FakeIndex::default()))),
    )
    .expect("the environment host");
    assert_eq!(cache.redis_url.expose_secret(), "redis://:pw@envhost:6379");
}

#[rstest]
#[case::host("", "6379", "pw", "redis://:pw@:6379")]
#[case::port("localhost", "", "pw", "redis://:pw@localhost:")]
#[case::password("localhost", "6379", "", "redis://:@localhost:6379")]
fn options_an_empty_environment_value_still_builds_the_url(
    #[case] host: &str,
    #[case] port: &str,
    #[case] password: &str,
    #[case] url: &str,
) {
    let mut settings = options();
    settings.host = None;
    settings.port = None;
    settings.password = None;
    let env = RedisEnv {
        redis_host: Some(host.to_owned()),
        redis_port: Some(port.to_owned()),
        redis_password: Some(SecretString::from(password)),
    };
    let cache = RedisSemanticCache::new(
        settings,
        env,
        Arc::new(FakeEmbedder::new()),
        Arc::new(FakeConnector::new(Arc::new(FakeIndex::default()))),
    )
    .expect("a set environment value is used as is");
    assert_eq!(cache.redis_url.expose_secret(), url);
}

#[test]
fn options_redis_url_wins_over_host_port_and_password() {
    let mut settings = options();
    settings.redis_url = Some(SecretString::from("redis://:urlpw@elsewhere:6390"));
    settings.host = Some("ignored".to_owned());
    let cache = RedisSemanticCache::new(
        settings,
        empty_env(),
        Arc::new(FakeEmbedder::new()),
        Arc::new(FakeConnector::new(Arc::new(FakeIndex::default()))),
    )
    .expect("the url setting");
    assert_eq!(
        cache.redis_url.expose_secret(),
        "redis://:urlpw@elsewhere:6390"
    );
}

#[test]
fn options_distance_threshold_is_one_minus_the_similarity() {
    let mut settings = options();
    settings.similarity_threshold = Some(0.25_f64);
    let cache = RedisSemanticCache::new(
        settings,
        empty_env(),
        Arc::new(FakeEmbedder::new()),
        Arc::new(FakeConnector::new(Arc::new(FakeIndex::default()))),
    )
    .expect("the threshold");
    assert!(cache.similarity_threshold.total_cmp(&0.25_f64).is_eq());
    assert!(cache.distance_threshold.total_cmp(&0.75_f64).is_eq());
    assert_eq!(cache.embedding_model, "text-embedding-ada-002");
    assert_eq!(cache.index_name, "litellm_semantic_cache_index");
}

#[rstest]
#[case::absent(None, None)]
#[case::minutes(Some(60), Some(60))]
#[case::negative(Some(-5), Some(-5))]
#[case::zero(Some(0), None)]
fn ttl_values_reach_redis_except_zero(
    #[case] configured: Option<i64>,
    #[case] expected: Option<i64>,
) {
    assert_eq!(RedisSemanticCache::get_ttl(configured), expected);
}

#[test]
fn filter_tag_escaping_covers_every_redisvl_character() {
    let escaped = super::keys::escape_tag_value("a,.<>{}[]\\\"':;!@#$%^&*()-+=~|/ ?b");
    assert_eq!(
        escaped,
        "a\\,\\.\\<\\>\\{\\}\\[\\]\\\\\\\"\\'\\:\\;\\!\\@\\#\\$\\%\\^\\&\\*\\(\\)\\-\\+\\=\\~\\|\\/\\ \\?b"
    );
}

#[test]
fn filter_entry_id_hashes_prompt_and_cache_key() {
    assert_eq!(
        super::keys::entry_id("Hello", "scope-1"),
        "543f4a38ee8458de185aa3ad234ebcf77085ea9b2290e51d5338fd5b460e8aea"
    );
}

#[rstest]
#[case::same_scope(Some("scope-1"), true)]
#[case::other_scope(Some("scope-2"), false)]
#[case::missing_field(None, false)]
fn filter_key_match_requires_the_same_scope(#[case] stored: Option<&str>, #[case] expected: bool) {
    let candidate = hit(0.25, stored, "{}");
    assert_eq!(
        RedisSemanticCache::cache_hit_matches_key(&candidate, "scope-1"),
        expected
    );
}

#[tokio::test]
async fn index_create_uses_the_base_name_and_the_probed_dimension() {
    let index = Arc::new(FakeIndex::default());
    let embedder = Arc::new(FakeEmbedder::with_vector(vec![0.5; PROBE.len() + 2]));
    let (cache, _connector) = cache_with_shared(Arc::clone(&embedder), Arc::clone(&index));
    cache
        .async_set_cache("scope-1", &Json::Null, &kwargs_with_message("hi"))
        .await;
    let creates = index.creates();
    assert_eq!(creates.len(), 1);
    assert_eq!(creates[0].name, "litellm_semantic_cache_index");
    assert_eq!(creates[0].prefix, "litellm_semantic_cache_index");
    assert_eq!(creates[0].dimension, PROBE.len());
    let probes: Vec<_> = embedder
        .calls()
        .into_iter()
        .filter(|(prompt, _)| prompt == "dimension test")
        .collect();
    assert_eq!(probes, [("dimension test".to_owned(), None)]);
}

#[tokio::test]
async fn index_is_built_once_and_the_constructor_opens_nothing() {
    let index = Arc::new(FakeIndex::default());
    let embedder = Arc::new(FakeEmbedder::new());
    let (cache, connector) = cache_with_shared(Arc::clone(&embedder), Arc::clone(&index));
    assert_eq!(connector.connects(), 0);
    assert!(embedder.calls().is_empty());
    cache
        .async_set_cache("scope-1", &Json::Null, &kwargs_with_message("hi"))
        .await;
    cache
        .async_set_cache("scope-2", &Json::Null, &kwargs_with_message("hello"))
        .await;
    assert_eq!(connector.connects(), 1);
    assert_eq!(index.creates().len(), 1);
    let probes = embedder
        .calls()
        .into_iter()
        .filter(|(prompt, _)| prompt == "dimension test")
        .count();
    assert_eq!(probes, 1);
    assert_eq!(index.stores().len(), 2);
}

#[tokio::test]
async fn index_distance_outside_zero_to_two_fails_the_build_without_creating() {
    let index = Arc::new(FakeIndex::default());
    index.set_hits(vec![hit(0.9, Some("scope-1"), "{}")]);
    let mut settings = options();
    settings.similarity_threshold = Some(-1.5_f64);
    let cache = RedisSemanticCache::new(
        settings,
        empty_env(),
        Arc::new(FakeEmbedder::new()),
        Arc::new(FakeConnector::new(Arc::clone(&index))),
    )
    .expect("the constructor has no range check");
    let error = cache.index_info().await.expect_err("the build fails");
    assert_eq!(
        error.to_string(),
        "Distance must be between 0 and 2, got 2.5"
    );
    let mut metadata = Object::new();
    let found = cache
        .async_get_cache("scope-1", &kwargs_with_message("hi"), Some(&mut metadata))
        .await
        .expect("a miss");
    assert_eq!(found, None);
    assert!(index.creates().is_empty());
    assert!(index.searches().is_empty());
    assert_eq!(
        metadata.get(SIMILARITY_FIELD),
        Some(&Json::Number(Number::from_f64(0.0).expect("a number")))
    );
}

fn matching_shape() -> InfoShape {
    InfoShape::matching("litellm_semantic_cache_index", PROBE.len())
}

fn vector_tail(algorithm: &str, datatype: &str, metric: &str) -> Vec<String> {
    words(&[
        "algorithm",
        algorithm,
        "data_type",
        datatype,
        "dim",
        "3",
        "distance_metric",
        metric,
    ])
}

#[rstest]
#[case::matching(matching_shape(), true)]
#[case::prefix(InfoShape { prefix: "other".to_owned(), ..matching_shape() }, false)]
#[case::dimension(InfoShape::matching("litellm_semantic_cache_index", 9), false)]
#[case::algorithm(InfoShape { vector_tail: vector_tail("HNSW", "FLOAT32", "COSINE"), ..matching_shape() }, false)]
#[case::metric(InfoShape { vector_tail: vector_tail("FLAT", "FLOAT32", "L2"), ..matching_shape() }, false)]
#[case::datatype(InfoShape { vector_tail: vector_tail("FLAT", "FLOAT64", "COSINE"), ..matching_shape() }, false)]
#[case::vector_name(InfoShape { vector_name: "other_vector".to_owned(), ..matching_shape() }, false)]
#[case::field_type(InfoShape { response_kind: "NUMERIC".to_owned(), ..matching_shape() }, false)]
#[case::older_layout_algorithm(InfoShape { vector_tail: words(&["HNSW", "6", "TYPE", "FLOAT32", "DIM", "3", "DISTANCE_METRIC", "COSINE"]), ..matching_shape() }, false)]
#[case::older_layout_type_datatype(InfoShape { vector_tail: words(&["FLAT", "6", "TYPE", "FLOAT64", "DIM", "3", "DISTANCE_METRIC", "COSINE"]), ..matching_shape() }, false)]
#[case::keyed_layout_type_datatype(InfoShape { vector_tail: words(&["ALGORITHM", "FLAT", "TYPE", "FLOAT64", "DIM", "3"]), ..matching_shape() }, false)]
#[case::uppercase_algorithm_name(InfoShape { vector_tail: words(&["ALGORITHM", "HNSW", "TYPE", "FLOAT32", "DIM", "3"]), ..matching_shape() }, false)]
#[case::uppercase_metric_name(InfoShape { vector_tail: words(&["DISTANCE_METRIC", "L2", "DIM", "3"]), ..matching_shape() }, false)]
#[case::uppercase_names_with_type_datatype(InfoShape { vector_tail: words(&["ALGORITHM", "FLAT", "TYPE", "FLOAT32", "DIM", "3"]), ..matching_shape() }, true)]
#[case::key_type(InfoShape { key_type: "JSON".to_owned(), ..matching_shape() }, false)]
#[case::prefix_as_text(InfoShape { prefix_as_text: true, ..matching_shape() }, true)]
#[case::other_prefix_as_text(InfoShape { prefix: "other".to_owned(), prefix_as_text: true, ..matching_shape() }, false)]
#[case::dims_at_an_odd_offset(InfoShape { vector_tail: words(&["FLAT", "TYPE", "FLOAT32", "DIMS", "3"]), ..matching_shape() }, true)]
#[case::dimension_at_an_odd_offset(InfoShape { vector_tail: words(&["FLAT", "TYPE", "FLOAT32", "DIM", "3"]), ..matching_shape() }, true)]
fn index_schema_compare_flags_every_difference(#[case] shape: InfoShape, #[case] expected: bool) {
    let schema = IndexSchema {
        name: "litellm_semantic_cache_index".to_owned(),
        prefix: "litellm_semantic_cache_index".to_owned(),
        dimension: PROBE.len(),
    };
    let reply = info_reply_shaped("litellm_semantic_cache_index", &shape);
    assert_eq!(super::schema::schema_matches(&reply, &schema), expected);
}

#[rstest]
#[case::older_positional_layout(&["FLAT", "6", "TYPE", "FLOAT32", "DIM", "3", "DISTANCE_METRIC", "COSINE"])]
#[case::uppercase_names_with_defaults(&["ALGORITHM", "FLAT", "TYPE", "FLOAT32", "DIM", "3"])]
#[case::only_the_dimension(&["DIM", "3"])]
#[tokio::test]
async fn index_older_and_defaulted_vector_layouts_reuse_the_base_index(#[case] tail: &[&str]) {
    let shape = InfoShape {
        vector_tail: words(tail),
        ..matching_shape()
    };
    let index = Arc::new(FakeIndex::default().with_info(
        "litellm_semantic_cache_index",
        info_reply_shaped("litellm_semantic_cache_index", &shape),
    ));
    let (cache, _connector) = cache_with(FakeEmbedder::new(), Arc::clone(&index));
    cache
        .async_set_cache("scope-1", &Json::Null, &kwargs_with_message("hi"))
        .await;
    assert!(index.creates().is_empty());
    assert!(index.drops().is_empty());
    let stores = index.stores();
    assert_eq!(stores.len(), 1);
    assert!(stores[0].0.starts_with("litellm_semantic_cache_index:"));
}

#[tokio::test]
async fn index_mismatch_uses_the_isolated_index() {
    let index = Arc::new(
        FakeIndex::default().with_info("litellm_semantic_cache_index", info_reply("other", 2)),
    );
    let (cache, _connector) = cache_with(FakeEmbedder::new(), Arc::clone(&index));
    cache
        .async_set_cache("scope-1", &Json::Null, &kwargs_with_message("hi"))
        .await;
    let stores = index.stores();
    assert_eq!(stores.len(), 1);
    assert!(
        stores[0]
            .0
            .starts_with("litellm_semantic_cache_index_isolated:")
    );
    let creates = index.creates();
    assert_eq!(creates[0].name, "litellm_semantic_cache_index_isolated");
}

#[tokio::test]
async fn index_second_mismatch_drops_and_recreates_the_isolated_index() {
    let index = Arc::new(
        FakeIndex::default()
            .with_info("litellm_semantic_cache_index", info_reply("other", 2))
            .with_info(
                "litellm_semantic_cache_index_isolated",
                info_reply("stale", 9),
            ),
    );
    let (cache, _connector) = cache_with(FakeEmbedder::new(), Arc::clone(&index));
    cache
        .async_set_cache("scope-1", &Json::Null, &kwargs_with_message("hi"))
        .await;
    assert_eq!(index.drops(), ["litellm_semantic_cache_index_isolated"]);
    let creates = index.creates();
    assert_eq!(creates.len(), 1);
    assert_eq!(creates[0].name, "litellm_semantic_cache_index_isolated");
    assert_eq!(creates[0].dimension, PROBE.len());
}

#[tokio::test]
async fn index_other_errors_are_returned_without_the_isolated_fallback() {
    let index = Arc::new(FakeIndex::default());
    *index.info_failure.lock().expect("the info failure") = Some(alternative_error());
    let (cache, _connector) = cache_with(FakeEmbedder::new(), Arc::clone(&index));
    let error = cache.index_info().await.expect_err("the backend error");
    assert_eq!(error.to_string(), "the redis setting 'info' is invalid");
    assert!(index.creates().is_empty());
    assert!(index.drops().is_empty());
}

#[tokio::test]
async fn index_existing_matching_schema_is_reused() {
    let index = Arc::new(FakeIndex::default().with_info(
        "litellm_semantic_cache_index",
        info_reply("litellm_semantic_cache_index", PROBE.len()),
    ));
    let (cache, _connector) = cache_with(FakeEmbedder::new(), Arc::clone(&index));
    cache
        .async_set_cache("scope-1", &Json::Null, &kwargs_with_message("hi"))
        .await;
    assert!(index.creates().is_empty());
    assert_eq!(index.stores().len(), 1);
}

#[tokio::test]
async fn index_build_is_retried_after_a_failed_connect() {
    let index = Arc::new(FakeIndex::default());
    let (cache, connector) = cache_with(FakeEmbedder::new(), Arc::clone(&index));
    *connector.failure.lock().expect("the connect failure") = Some(alternative_error());
    cache
        .async_set_cache("scope-1", &Json::Null, &kwargs_with_message("hi"))
        .await;
    assert!(index.stores().is_empty());
    cache
        .async_set_cache("scope-1", &Json::Null, &kwargs_with_message("hi"))
        .await;
    assert_eq!(connector.connects(), 2);
    assert_eq!(index.stores().len(), 1);
}

#[tokio::test]
async fn store_writes_the_entry_fields_under_the_index_key() {
    let (cache, index) = stored_cache();
    let value = Json::Object(Object::from([(
        "choices".to_owned(),
        Json::List(Vec::new()),
    )]));
    cache
        .async_set_cache("scope-1", &value, &kwargs_with_message("hi"))
        .await;
    let stores = index.stores();
    assert_eq!(stores.len(), 1);
    let (key, fields, ttl) = &stores[0];
    assert_eq!(
        key,
        &format!("litellm_semantic_cache_index:{}", fields.entry_id)
    );
    assert_eq!(fields.prompt, "hi");
    assert_eq!(fields.response, "{\"choices\":[]}");
    let expected_vector: Vec<u8> = VECTOR
        .iter()
        .flat_map(|number| number.to_le_bytes())
        .collect();
    assert_eq!(fields.prompt_vector, expected_vector);
    assert!(fields.inserted_at.total_cmp(&TIMESTAMP).is_eq());
    assert!(fields.updated_at.total_cmp(&TIMESTAMP).is_eq());
    assert_eq!(fields.litellm_cache_key, "scope-1");
    assert_eq!(*ttl, None);
}

#[tokio::test]
async fn store_passes_the_ttl_through_and_zero_means_no_expiry() {
    let (cache, index) = stored_cache();
    let mut kwargs = kwargs_with_message("hi");
    kwargs.ttl = Some(60);
    cache.async_set_cache("scope-1", &Json::Null, &kwargs).await;
    let mut zero = kwargs_with_message("hi");
    zero.ttl = Some(0);
    cache.async_set_cache("scope-1", &Json::Null, &zero).await;
    let ttls: Vec<Option<i64>> = index.stores().into_iter().map(|(_, _, ttl)| ttl).collect();
    assert_eq!(ttls, [Some(60), None]);
}

#[tokio::test]
async fn store_a_vector_of_the_wrong_size_is_not_stored() {
    let index = Arc::new(FakeIndex::default());
    let embedder = Arc::new(FakeEmbedder::with_vector(vec![0.5; PROBE.len() + 2]));
    let (cache, _connector) = cache_with_shared(embedder, Arc::clone(&index));
    cache
        .async_set_cache("scope-1", &Json::Null, &kwargs_with_message("hi"))
        .await;
    assert_eq!(index.creates().len(), 1);
    assert!(index.stores().is_empty());
}

#[tokio::test]
async fn store_without_a_prompt_touches_nothing() {
    let (cache, index) = stored_cache();
    let kwargs = SemanticCacheKwargs {
        messages: None,
        input: None,
        metadata: None,
        ttl: None,
    };
    cache.async_set_cache("scope-1", &Json::Null, &kwargs).await;
    assert!(index.stores().is_empty());
    assert!(index.searches().is_empty());
}

#[tokio::test]
async fn store_empty_messages_fall_through_to_the_responses_input() {
    let (cache, index) = stored_cache();
    let kwargs = SemanticCacheKwargs {
        messages: Some(Vec::new()),
        input: Some(Json::Text("from the input".to_owned())),
        metadata: None,
        ttl: None,
    };
    cache.async_set_cache("scope-1", &Json::Null, &kwargs).await;
    let stores = index.stores();
    assert_eq!(stores.len(), 1);
    assert_eq!(stores[0].1.prompt, "from the input");
}

#[tokio::test]
async fn store_swallows_embedding_and_backend_failures() {
    let index = Arc::new(FakeIndex::default());
    let (cache, _connector) = cache_with(FakeEmbedder::failing(), Arc::clone(&index));
    cache
        .async_set_cache("scope-1", &Json::Null, &kwargs_with_message("hi"))
        .await;
    assert!(index.stores().is_empty());

    let (second_cache, failing_index) = stored_cache();
    *failing_index
        .store_failure
        .lock()
        .expect("the store failure") = Some(alternative_error());
    second_cache
        .async_set_cache("scope-1", &Json::Null, &kwargs_with_message("hi"))
        .await;
    assert!(failing_index.stores().is_empty());
}

#[tokio::test]
async fn store_pipeline_stores_every_pair_with_the_same_kwargs() {
    let (cache, index) = stored_cache();
    let pairs = [
        ("scope-1".to_owned(), Json::Text("one".to_owned())),
        ("scope-2".to_owned(), Json::Text("two".to_owned())),
    ];
    cache
        .async_set_cache_pipeline(&pairs, &kwargs_with_message("hi"))
        .await;
    let stores = index.stores();
    assert!(
        stores
            .iter()
            .all(|(key, _, _)| key.starts_with("litellm_semantic_cache_index:"))
    );
    let mut scopes: Vec<String> = stores
        .iter()
        .map(|(_, fields, _)| fields.litellm_cache_key.clone())
        .collect();
    scopes.sort();
    assert_eq!(scopes, ["scope-1", "scope-2"]);
}

#[tokio::test]
async fn lookup_returns_the_cached_response_and_the_similarity() {
    let (cache, index) = stored_cache();
    index.set_hits(vec![hit(0.25, Some("scope-1"), "{\"choices\":[]}")]);
    let mut metadata = Object::new();
    let value = cache
        .async_get_cache("scope-1", &kwargs_with_message("hi"), Some(&mut metadata))
        .await
        .expect("a cache read");
    assert_eq!(
        value,
        Some(Json::Object(Object::from([(
            "choices".to_owned(),
            Json::List(Vec::new())
        )])))
    );
    assert_eq!(
        metadata.get(SIMILARITY_FIELD),
        Some(&Json::Number(Number::from_f64(0.75).expect("a number")))
    );
}

#[tokio::test]
async fn lookup_miss_writes_zero_similarity() {
    let (cache, index) = stored_cache();
    index.set_hits(Vec::new());
    let mut metadata = Object::new();
    let value = cache
        .async_get_cache("scope-1", &kwargs_with_message("hi"), Some(&mut metadata))
        .await
        .expect("a cache read");
    assert_eq!(value, None);
    assert_eq!(
        metadata.get(SIMILARITY_FIELD),
        Some(&Json::Number(Number::from_f64(0.0).expect("a number")))
    );
}

#[tokio::test]
async fn lookup_wrong_scope_and_a_missing_key_field_are_misses() {
    let (cache, index) = stored_cache();
    index.set_hits(vec![hit(0.25, Some("scope-2"), "{}")]);
    let mut metadata = Object::new();
    assert_eq!(
        cache
            .async_get_cache("scope-1", &kwargs_with_message("hi"), Some(&mut metadata))
            .await
            .expect("a cache read"),
        None
    );
    assert_eq!(
        metadata.get(SIMILARITY_FIELD),
        Some(&Json::Number(Number::from_f64(0.0).expect("a number")))
    );

    index.set_hits(vec![hit(0.25, None, "{}")]);
    assert_eq!(
        cache
            .async_get_cache("scope-1", &kwargs_with_message("hi"), None)
            .await
            .expect("a cache read"),
        None
    );

    let mut without_prompt = hit(0.25, Some("scope-1"), "{}");
    without_prompt.prompt = None;
    index.set_hits(vec![without_prompt]);
    assert_eq!(
        cache
            .async_get_cache("scope-1", &kwargs_with_message("hi"), None)
            .await
            .expect("a cache read"),
        None
    );
}

#[tokio::test]
async fn lookup_decode_failure_keeps_the_real_similarity() {
    let (cache, index) = stored_cache();
    index.set_hits(vec![hit(0.25, Some("scope-1"), "not json")]);
    let mut metadata = Object::new();
    let value = cache
        .async_get_cache("scope-1", &kwargs_with_message("hi"), Some(&mut metadata))
        .await
        .expect("a cache read");
    assert_eq!(value, None);
    assert_eq!(
        metadata.get(SIMILARITY_FIELD),
        Some(&Json::Number(Number::from_f64(0.75).expect("a number")))
    );
}

#[tokio::test]
async fn lookup_embedding_failure_is_a_miss_with_zero_similarity() {
    let index = Arc::new(FakeIndex::default());
    let (cache, _connector) = cache_with(FakeEmbedder::failing(), Arc::clone(&index));
    let mut metadata = Object::new();
    let value = cache
        .async_get_cache("scope-1", &kwargs_with_message("hi"), Some(&mut metadata))
        .await
        .expect("a cache read");
    assert_eq!(value, None);
    assert_eq!(
        metadata.get(SIMILARITY_FIELD),
        Some(&Json::Number(Number::from_f64(0.0).expect("a number")))
    );
    assert!(index.searches().is_empty());
}

#[tokio::test]
async fn lookup_sends_the_scoped_query_with_the_similarity_threshold() {
    let (cache, index) = stored_cache();
    index.set_hits(vec![hit(0.25, Some("scope-1"), "{}")]);
    let value = cache
        .async_get_cache("scope-1", &kwargs_with_message("hi"), None)
        .await
        .expect("a cache read");
    assert_eq!(value, Some(Json::Object(Object::new())));
    let searches = index.searches();
    assert_eq!(searches.len(), 1);
    let (name, query) = &searches[0];
    assert_eq!(name, "litellm_semantic_cache_index");
    assert_eq!(query.cache_key, "scope-1");
    assert_eq!(query.vector, VECTOR);
    assert!(query.distance_threshold.total_cmp(&0.75_f64).is_eq());
    assert_eq!(
        query.query_string(),
        "@prompt_vector:[VECTOR_RANGE $distance_threshold $vector]=>{$YIELD_DISTANCE_AS: vector_distance} (@litellm_cache_key:{scope\\-1})"
    );
}

#[tokio::test]
async fn lookup_dimension_mismatch_is_a_miss_before_the_search() {
    let index = Arc::new(FakeIndex::default());
    let mut embedder = FakeEmbedder::new();
    embedder.vector = vec![0.5, 0.5];
    let (cache, _connector) = cache_with(embedder, Arc::clone(&index));
    let value = cache
        .async_get_cache("scope-1", &kwargs_with_message("hi"), None)
        .await
        .expect("a cache read");
    assert_eq!(value, None);
    assert!(index.searches().is_empty());
}

fn info_reply_with_options() -> Json {
    Json::List(vec![
        Json::Text("index_name".to_owned()),
        Json::Text("the-index".to_owned()),
        Json::Text("index_options".to_owned()),
        Json::List(vec![Json::Text("NOFIELDS".to_owned())]),
        Json::Text("index_definition".to_owned()),
        Json::List(vec![
            Json::Text("key_type".to_owned()),
            Json::Text("HASH".to_owned()),
            Json::Text("prefixes".to_owned()),
            Json::List(vec![Json::Text("the-index".to_owned())]),
        ]),
        Json::Text("num_docs".to_owned()),
        Json::Number(Number::from(7_i64)),
    ])
}

#[test]
fn info_reply_pairs_only_the_top_level_and_keeps_nested_lists() {
    let converted =
        super::schema::info_object(&info_reply_with_options()).expect("an FT.INFO reply");
    let Json::Object(info) = converted else {
        panic!("the reply converts to an object");
    };
    let keys: Vec<&str> = info.keys().map(String::as_str).collect();
    assert_eq!(
        keys,
        [
            "index_name",
            "index_options",
            "index_definition",
            "num_docs"
        ]
    );
    assert_eq!(
        info.get("index_name"),
        Some(&Json::Text("the-index".to_owned()))
    );
    assert_eq!(
        info.get("num_docs"),
        Some(&Json::Number(Number::from(7_i64)))
    );
    assert_eq!(
        info.get("index_options"),
        Some(&Json::List(vec![Json::Text("NOFIELDS".to_owned())]))
    );
    assert_eq!(
        info.get("index_definition"),
        Some(&Json::List(vec![
            Json::Text("key_type".to_owned()),
            Json::Text("HASH".to_owned()),
            Json::Text("prefixes".to_owned()),
            Json::List(vec![Json::Text("the-index".to_owned())]),
        ]))
    );
}

#[tokio::test]
async fn info_passes_the_redis_error_text_through() {
    let index = Arc::new(FakeIndex::default());
    *index.info_failure.lock().expect("the info failure") = Some(alternative_error());
    let (cache, _connector) = cache_with(FakeEmbedder::new(), Arc::clone(&index));
    let error = cache.index_info().await.expect_err("the backend error");
    assert_eq!(error.to_string(), "the redis setting 'info' is invalid");
}

#[test]
fn index_create_command_carries_the_exact_schema() {
    let schema = IndexSchema {
        name: "idx".to_owned(),
        prefix: "idx".to_owned(),
        dimension: 3,
    };
    let args = text_args(&super::live::create_command(&schema));
    assert_eq!(
        args,
        [
            "FT.CREATE",
            "idx",
            "ON",
            "HASH",
            "PREFIX",
            "1",
            "idx",
            "SCHEMA",
            "prompt",
            "TEXT",
            "response",
            "TEXT",
            "inserted_at",
            "NUMERIC",
            "updated_at",
            "NUMERIC",
            "prompt_vector",
            "VECTOR",
            "FLAT",
            "6",
            "TYPE",
            "FLOAT32",
            "DIM",
            "3",
            "DISTANCE_METRIC",
            "COSINE",
            "litellm_cache_key",
            "TAG",
        ]
    );
}

#[test]
fn lookup_search_command_carries_the_range_query_and_params() {
    let query = SemanticSearchQuery {
        vector: vec![1.0_f32, 0.5_f32],
        distance_threshold: 0.25,
        cache_key: "a|b".to_owned(),
    };
    let args = command_args(&super::live::search_command("idx", &query));
    let expected: Vec<Vec<u8>> = vec![
        b"FT.SEARCH".to_vec(),
        b"idx".to_vec(),
        b"@prompt_vector:[VECTOR_RANGE $distance_threshold $vector]=>{$YIELD_DISTANCE_AS: vector_distance} (@litellm_cache_key:{a\\|b})".to_vec(),
        b"PARAMS".to_vec(),
        b"4".to_vec(),
        b"vector".to_vec(),
        vec![0, 0, 128, 63, 0, 0, 0, 63],
        b"distance_threshold".to_vec(),
        b"0.25".to_vec(),
        b"RETURN".to_vec(),
        b"8".to_vec(),
        b"entry_id".to_vec(),
        b"prompt".to_vec(),
        b"response".to_vec(),
        b"inserted_at".to_vec(),
        b"updated_at".to_vec(),
        b"metadata".to_vec(),
        b"litellm_cache_key".to_vec(),
        b"vector_distance".to_vec(),
        b"SORTBY".to_vec(),
        b"vector_distance".to_vec(),
        b"ASC".to_vec(),
        b"LIMIT".to_vec(),
        b"0".to_vec(),
        b"1".to_vec(),
        b"DIALECT".to_vec(),
        b"2".to_vec(),
    ];
    assert_eq!(args, expected);
}

#[test]
fn store_hash_command_names_every_field() {
    let fields = CacheEntryFields {
        entry_id: "e1".to_owned(),
        prompt: "hi".to_owned(),
        response: "{}".to_owned(),
        prompt_vector: vec![0, 0, 128, 63],
        inserted_at: 1_700_000_000.5,
        updated_at: 1_700_000_000.5,
        litellm_cache_key: "scope".to_owned(),
    };
    let args = command_args(&super::live::store_command("idx:e1", &fields));
    let expected: Vec<Vec<u8>> = vec![
        b"HSET".to_vec(),
        b"idx:e1".to_vec(),
        b"entry_id".to_vec(),
        b"e1".to_vec(),
        b"prompt".to_vec(),
        b"hi".to_vec(),
        b"response".to_vec(),
        b"{}".to_vec(),
        b"prompt_vector".to_vec(),
        vec![0, 0, 128, 63],
        b"inserted_at".to_vec(),
        b"1700000000.5".to_vec(),
        b"updated_at".to_vec(),
        b"1700000000.5".to_vec(),
        b"litellm_cache_key".to_vec(),
        b"scope".to_vec(),
    ];
    assert_eq!(args, expected);
}

#[test]
fn lookup_parses_the_search_reply_into_hits() {
    let reply = Json::List(vec![
        Json::Number(Number::from(1_i64)),
        Json::Text("idx:entry".to_owned()),
        Json::List(vec![
            Json::Text("entry_id".to_owned()),
            Json::Text("e1".to_owned()),
            Json::Text("prompt".to_owned()),
            Json::Text("hi".to_owned()),
            Json::Text("response".to_owned()),
            Json::Text("{}".to_owned()),
            Json::Text("inserted_at".to_owned()),
            Json::Text("1.0".to_owned()),
            Json::Text("updated_at".to_owned()),
            Json::Text("1.0".to_owned()),
            Json::Text("litellm_cache_key".to_owned()),
            Json::Text("scope".to_owned()),
            Json::Text("vector_distance".to_owned()),
            Json::Text("0.25".to_owned()),
        ]),
    ]);
    let hits = super::query::parse_search_reply(&reply).expect("a search reply");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].prompt.as_deref(), Some("hi"));
    assert_eq!(hits[0].response.as_deref(), Some("{}"));
    assert_eq!(hits[0].litellm_cache_key.as_deref(), Some("scope"));
    let distance = hits[0].vector_distance.expect("a distance");
    assert!(distance.total_cmp(&0.25_f64).is_eq());
}

#[rstest]
#[case::total_above_the_returned_count(2)]
#[case::total_equal(1)]
fn lookup_total_count_may_exceed_the_returned_documents(#[case] total: i64) {
    let reply = Json::List(vec![
        Json::Number(Number::from(total)),
        Json::Text("idx:entry".to_owned()),
        Json::List(vec![
            Json::Text("response".to_owned()),
            Json::Text("{}".to_owned()),
            Json::Text("vector_distance".to_owned()),
            Json::Text("0.1".to_owned()),
        ]),
    ]);
    let hits = super::query::parse_search_reply(&reply).expect("a search reply");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].response.as_deref(), Some("{}"));
}

#[test]
fn lookup_rejects_a_search_reply_with_a_key_and_no_document() {
    let reply = Json::List(vec![
        Json::Number(Number::from(1_i64)),
        Json::Text("idx:entry".to_owned()),
    ]);
    super::query::parse_search_reply(&reply).expect_err("an odd remainder");
}

#[test]
fn lookup_rejects_a_malformed_search_reply() {
    let reply = Json::List(vec![Json::Text("not a count".to_owned())]);
    super::query::parse_search_reply(&reply).expect_err("a malformed reply");
}
