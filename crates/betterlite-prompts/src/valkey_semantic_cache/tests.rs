#![cfg(test)]

mod fake_server;
mod fixtures;

use std::error::Error as _;
use std::sync::Arc;

use betterlite_redis::valkey::pack_vector;
use redis::Value;
use rstest::rstest;
use secrecy::SecretString;
use uuid::Uuid;

use self::fake_server::FakeValkey;
use self::fixtures::{
    FakeEmbedder, FakeIndex, INDEX_NAME, ONE_HIT, PROMPT, SIMPLE_STRING_INFO, VECTOR,
    alternative_error, assert_similarity, build, cache_with, cache_with_injected_index,
    command_args, empty_env, exists_error, hit, index_schema, info_reply, info_with_attributes,
    kwargs_with_message, kwargs_without_prompt, live_index, options, packed_vector, server_error,
    tag_attribute, text_args, url_of, vector_attribute, vector_attribute_without_marker,
};
use crate::arguments::json::{Json, Object};
use crate::message_content::Message;
use crate::valkey_semantic_cache::error::ValkeySemanticError;
use crate::valkey_semantic_cache::live::{
    create_command, parse_search_reply, search_command, store_command,
};
use crate::valkey_semantic_cache::options::ValkeyEnv;
use crate::valkey_semantic_cache::ports::{
    DocIdSource as _, Hit, IndexSchema, UuidDocIds, ValkeyIndex,
};
use crate::valkey_semantic_cache::schema::{
    assert_dim_matches, doc_mapping, extract_index_dim, knn_query, scope_tag,
};

#[test]
fn options_missing_threshold_fails_with_litellms_message() {
    let mut settings = options();
    settings.similarity_threshold = None;
    let Err(error) = build(settings, empty_env()) else {
        panic!("a cache without a threshold must fail");
    };
    assert_eq!(
        error.to_string(),
        "similarity_threshold must be provided, passed None"
    );
}

#[test]
fn options_startup_nodes_fail_with_litellms_message() {
    let mut settings = options();
    settings.startup_nodes = vec!["valkey-0001:6379".to_owned()];
    let Err(error) = build(settings, empty_env()) else {
        panic!("a cluster-mode endpoint must fail");
    };
    assert_eq!(
        error.to_string(),
        "valkey-semantic does not support cluster-mode-enabled (multi-shard) endpoints. The \
         async cluster client cannot route the FT.* search commands reliably. Point it at a \
         cluster-mode-disabled endpoint instead (a primary with replicas is fine; only \
         horizontal sharding is unsupported), or pass a single redis_url. On AWS, vector search \
         needs ElastiCache for Valkey 8.2+ on a node-based cluster."
    );
}

#[test]
fn options_missing_threshold_is_reported_before_startup_nodes() {
    let mut settings = options();
    settings.similarity_threshold = None;
    settings.startup_nodes = vec!["valkey-0001:6379".to_owned()];
    let Err(error) = build(settings, empty_env()) else {
        panic!("the threshold check comes first");
    };
    assert_eq!(
        error.to_string(),
        "similarity_threshold must be provided, passed None"
    );
}

#[rstest]
#[case::default_name(None, INDEX_NAME)]
#[case::configured_name(Some("custom-index"), "custom-index")]
fn options_index_name_and_key_prefix(#[case] index_name: Option<&str>, #[case] expected: &str) {
    let mut settings = options();
    settings.index_name = index_name.map(str::to_owned);
    let cache = build(settings, empty_env()).expect("the index settings");
    assert_eq!(cache.index_name, expected);
    assert_eq!(cache.key_prefix, format!("{expected}:"));
}

#[rstest]
#[case::valkey_wins(
    ValkeyEnv {
        valkey_host: Some("valkey.internal".to_owned()),
        valkey_port: Some("6380".to_owned()),
        valkey_password: Some(SecretString::from("envpw")),
        redis_host: Some("redis.internal".to_owned()),
        redis_port: Some("6381".to_owned()),
        redis_password: Some(SecretString::from("redispw")),
    },
    "redis://:envpw@valkey.internal:6380"
)]
#[case::redis_falls_back(
    ValkeyEnv {
        valkey_host: None,
        valkey_port: None,
        valkey_password: None,
        redis_host: Some("redis.internal".to_owned()),
        redis_port: Some("6381".to_owned()),
        redis_password: Some(SecretString::from("redispw")),
    },
    "redis://:redispw@redis.internal:6381"
)]
#[case::an_empty_valkey_value_falls_back(
    ValkeyEnv {
        valkey_host: Some(String::new()),
        valkey_port: Some(String::new()),
        valkey_password: Some(SecretString::from("")),
        redis_host: Some("redis.internal".to_owned()),
        redis_port: Some("6381".to_owned()),
        redis_password: Some(SecretString::from("redispw")),
    },
    "redis://:redispw@redis.internal:6381"
)]
fn options_env_values_fill_the_missing_host_port_and_password(
    #[case] env: ValkeyEnv,
    #[case] expected: &str,
) {
    let mut settings = options();
    settings.host = None;
    settings.port = None;
    settings.password = None;
    let cache = build(settings, env).expect("the environment settings");
    assert_eq!(url_of(&cache), expected);
}

#[rstest]
#[case::host(None, Some("6379"))]
#[case::port(Some("localhost"), None)]
fn options_missing_host_or_port_fails_with_litellms_message(
    #[case] host: Option<&str>,
    #[case] port: Option<&str>,
) {
    let mut settings = options();
    settings.host = host.map(str::to_owned);
    settings.port = port.map(str::to_owned);
    let Err(error) = build(settings, empty_env()) else {
        panic!("a missing host or port must fail");
    };
    assert_eq!(
        error.to_string(),
        "Missing required Valkey configuration. Provide host and port (or VALKEY_HOST/VALKEY_PORT), \
         or pass redis_url."
    );
}

#[test]
fn options_a_non_numeric_port_fails_naming_the_field() {
    let mut settings = options();
    settings.port = Some("not-a-port".to_owned());
    let Err(error) = build(settings, empty_env()) else {
        panic!("a non-numeric port must fail");
    };
    assert_eq!(
        error.to_string(),
        "the valkey port 'not-a-port' is not a number between 0 and 65535"
    );
    assert!(error.source().is_some());
}

#[rstest]
#[case::an_empty_explicit_host_loses_to_the_environment("", "pw", "envhost", "pw")]
#[case::a_set_explicit_host_wins("explicit", "pw", "explicit", "pw")]
#[case::an_empty_explicit_password_loses_to_the_environment("explicit", "", "explicit", "valkeypw")]
#[case::a_set_explicit_password_wins("explicit", "set", "explicit", "set")]
fn options_explicit_values_win_and_empty_strings_count_as_missing(
    #[case] host: &str,
    #[case] password: &str,
    #[case] expected_host: &str,
    #[case] expected_password: &str,
) {
    let mut settings = options();
    settings.host = Some(host.to_owned());
    settings.password = Some(SecretString::from(password));
    let env = ValkeyEnv {
        valkey_host: Some("envhost".to_owned()),
        valkey_password: Some(SecretString::from("valkeypw")),
        ..empty_env()
    };
    let cache = build(settings, env).expect("the explicit choice");
    assert_eq!(
        url_of(&cache),
        format!("redis://:{expected_password}@{expected_host}:6379")
    );
}

#[test]
fn options_redis_url_wins_over_host_port_and_password() {
    let mut settings = options();
    settings.redis_url = Some(SecretString::from("redis://:urlpw@elsewhere:6390"));
    settings.host = Some("ignored".to_owned());
    let cache = build(settings, empty_env()).expect("the url setting");
    assert_eq!(url_of(&cache), "redis://:urlpw@elsewhere:6390");
}

#[rstest]
#[case::punctuation(Some("p@ss word/"), false, "redis://:p%40ss%20word%2F@localhost:6379")]
#[case::python_safe_characters(Some("a~b_c.d-e"), false, "redis://:a~b_c.d-e@localhost:6379")]
#[case::ssl_selects_the_rediss_scheme(Some("pw"), true, "rediss://:pw@localhost:6379")]
#[case::a_missing_password_builds_a_credential_free_url(None, false, "redis://localhost:6379")]
fn options_url_encodes_the_password_and_picks_the_scheme(
    #[case] password: Option<&str>,
    #[case] ssl: bool,
    #[case] expected: &str,
) {
    let mut settings = options();
    settings.password = password.map(SecretString::from);
    settings.ssl = ssl;
    let cache = build(settings, empty_env()).expect("the url settings");
    assert_eq!(url_of(&cache), expected);
}

#[test]
fn options_an_injected_index_needs_no_url() {
    let backend: Arc<dyn ValkeyIndex> = Arc::new(FakeIndex::default());
    let mut settings = options();
    settings.host = None;
    settings.port = None;
    settings.password = None;
    settings.redis_url = None;
    settings.index = Some(backend);
    let cache = build(settings, empty_env()).expect("the injected index");
    assert!(cache.url.is_none());
}

#[test]
fn options_nothing_is_opened_until_first_use() {
    let index = Arc::new(FakeIndex::default());
    let (cache, connector) = cache_with(Arc::new(FakeEmbedder::new()), &index);
    assert!(connector.connects().is_empty());
    assert!(cache.url.is_some());
}

#[test]
fn options_embedding_settings_are_kept_public() {
    let cache = build(options(), empty_env()).expect("the embedding defaults");
    assert_eq!(cache.embedding_model, "text-embedding-ada-002");
    assert!(cache.embedding_max_input_tokens.is_none());
    assert_eq!(cache.embedding_timeout.to_bits(), 5.0_f64.to_bits());
}

#[rstest]
#[case::ascii(
    "user-1",
    "c6c289e49e9c05b2145860387b73bcb18df43fb09a1e4a4a9713c76c88bb541b"
)]
#[case::unicode(
    "h\u{e9}llo \u{2603}",
    "c3a3e84b30327c8e3f04a788d8b68da3fd94c22353c5a30e3fe6ca7ebde3ab3f"
)]
fn tag_scope_tag_is_the_lowercase_hex_sha256_of_the_key(#[case] key: &str, #[case] expected: &str) {
    assert_eq!(scope_tag(key), expected);
}

#[tokio::test]
async fn tag_documents_are_prefixed_with_the_scope_tag_and_a_new_uuid() {
    let index = Arc::new(FakeIndex::default());
    let (cache, _connector) = cache_with(Arc::new(FakeEmbedder::new()), &index);
    let kwargs = kwargs_with_message(PROMPT);
    cache
        .async_set_cache("user-1", &Json::Text("first".to_owned()), &kwargs)
        .await;
    cache
        .async_set_cache("user-1", &Json::Text("second".to_owned()), &kwargs)
        .await;
    let stores = index.stores();
    assert_eq!(stores.len(), 2);
    let scope = scope_tag("user-1");
    assert_eq!(
        stores[0].0,
        format!("{INDEX_NAME}:{scope}:00000000-0000-4000-8000-000000000000")
    );
    assert_eq!(
        stores[1].0,
        format!("{INDEX_NAME}:{scope}:00000000-0000-4000-8000-000000000001")
    );
}

#[tokio::test]
async fn tag_doc_mapping_carries_the_litellm_fields() {
    let index = Arc::new(FakeIndex::default());
    let (cache, _connector) = cache_with(Arc::new(FakeEmbedder::new()), &index);
    cache
        .async_set_cache(
            "user-1",
            &Json::Text("cached answer".to_owned()),
            &kwargs_with_message(PROMPT),
        )
        .await;
    let stores = index.stores();
    let fields = &stores[0].1;
    assert_eq!(fields.litellm_cache_key, scope_tag("user-1"));
    assert_eq!(fields.prompt, PROMPT);
    assert_eq!(fields.response, "\"cached answer\"");
    assert_eq!(fields.embedding, packed_vector());
}

#[test]
fn tag_generated_document_ids_are_distinct_uuid_v4() {
    let first = UuidDocIds.next_id();
    let second = UuidDocIds.next_id();
    assert_ne!(first, second);
    let parsed = Uuid::parse_str(&first).expect("a uuid text");
    assert_eq!(parsed.get_version_num(), 4);
}

#[tokio::test]
async fn schema_live_index_sends_ft_create() {
    let server = FakeValkey::start(&[]).await;
    let schema = index_schema();
    live_index(&server)
        .await
        .ft_create(INDEX_NAME, &schema)
        .await
        .expect("the created index");
    assert_eq!(
        server.received(&[b"FT.CREATE"]),
        [command_args(&create_command(INDEX_NAME, &schema))]
    );
}

#[test]
fn tag_pack_vector_is_little_endian_f32() {
    assert_eq!(
        pack_vector(&[1.0_f32, -2.5_f32]),
        vec![0_u8, 0, 128, 63, 0, 0, 32, 192]
    );
}

#[test]
fn schema_index_schema_matches_litellm() {
    let schema = index_schema();
    assert_eq!(
        text_args(&create_command(INDEX_NAME, &schema)),
        vec![
            "FT.CREATE",
            INDEX_NAME,
            "ON",
            "HASH",
            "PREFIX",
            "1",
            "litellm_semantic_cache_index:",
            "SCORE",
            "1.0",
            "SCHEMA",
            "litellm_cache_key",
            "TAG",
            "SEPARATOR",
            ",",
            "embedding",
            "VECTOR",
            "HNSW",
            "6",
            "TYPE",
            "FLOAT32",
            "DIM",
            "3",
            "DISTANCE_METRIC",
            "COSINE",
        ]
    );
}

#[test]
fn schema_knn_query_matches_litellm() {
    let query = knn_query("user-1");
    assert_eq!(
        query.text,
        format!(
            "(@litellm_cache_key:{{{}}})=>[KNN 1 @embedding $vec AS vector_distance]",
            scope_tag("user-1")
        )
    );
    assert_eq!(query.return_fields, vec!["response", "vector_distance"]);
    assert_eq!(query.dialect, 2);
}

#[test]
fn schema_search_command_sends_the_knn_query() {
    let query = knn_query("user-1");
    let args = command_args(&search_command(INDEX_NAME, &query, &packed_vector()));
    let text: Vec<String> = args
        .iter()
        .take(15)
        .map(|bytes| String::from_utf8(bytes.clone()).expect("a text argument"))
        .collect();
    assert_eq!(
        text,
        vec![
            "FT.SEARCH",
            INDEX_NAME,
            query.text.as_str(),
            "RETURN",
            "2",
            "response",
            "vector_distance",
            "DIALECT",
            "2",
            "LIMIT",
            "0",
            "10",
            "PARAMS",
            "2",
            "vec",
        ]
    );
    assert_eq!(args.last(), Some(&packed_vector()));
}

#[test]
fn schema_store_command_writes_the_litellm_fields() {
    let fields = doc_mapping("user-1", PROMPT, "{}", &VECTOR);
    let args = command_args(&store_command("doc-key", &fields));
    let text: Vec<String> = args
        .iter()
        .take(9)
        .map(|bytes| String::from_utf8(bytes.clone()).expect("a text argument"))
        .collect();
    assert_eq!(
        text,
        vec![
            "HSET",
            "doc-key",
            "litellm_cache_key",
            scope_tag("user-1").as_str(),
            "prompt",
            PROMPT,
            "response",
            "{}",
            "embedding",
        ]
    );
    assert_eq!(args.last(), Some(&packed_vector()));
}

#[test]
fn schema_extract_index_dim_reads_a_nested_dimensions_marker() {
    assert_eq!(extract_index_dim(&info_reply(768)), Some(768));
}

#[rstest]
#[case::number(Json::Number(serde_json::Number::from(512_u64)))]
#[case::text(Json::Text("512".to_owned()))]
fn schema_extract_index_dim_reads_the_marker_value(#[case] dimension: Json) {
    let attribute = Json::List(vec![
        Json::Text("identifier".to_owned()),
        Json::Text("embedding".to_owned()),
        Json::List(vec![Json::Text("dimensions".to_owned()), dimension]),
    ]);
    let info = Json::Object(Object::from([(
        "attributes".to_owned(),
        Json::List(vec![attribute]),
    )]));
    assert_eq!(extract_index_dim(&info), Some(512));
}

#[rstest]
#[case::no_attributes(Json::Object(Object::new()))]
#[case::no_dimensions_marker(Json::Object(Object::from([(
    "attributes".to_owned(),
    Json::List(vec![vector_attribute_without_marker()]),
)])))]
fn schema_extract_index_dim_is_none_without_the_marker(#[case] info: Json) {
    assert_eq!(extract_index_dim(&info), None);
}

#[rstest]
#[case::the_same_dimension(info_reply(3))]
#[case::an_index_without_a_dimension(Json::Object(Object::new()))]
fn schema_assert_dim_matches_accepts(#[case] info: Json) {
    assert_dim_matches(&info, 3, INDEX_NAME).expect("a matching or missing dimension");
}

#[test]
fn schema_assert_dim_matches_fails_with_litellms_text() {
    let Err(error) = assert_dim_matches(&info_reply(4), 3, INDEX_NAME) else {
        panic!("a dimension change must fail");
    };
    assert_eq!(
        error.to_string(),
        format!(
            "Valkey semantic-cache index '{INDEX_NAME}' already exists with embedding dimension 4, \
             but the configured embedding model produced dimension 3. Use a different \
             valkey_semantic_cache_index_name or drop the existing index."
        )
    );
}

#[tokio::test]
async fn index_creates_the_index_on_the_first_write() {
    let index = Arc::new(FakeIndex::default());
    let (cache, _connector) = cache_with(Arc::new(FakeEmbedder::new()), &index);
    cache
        .async_set_cache(
            "user-1",
            &Json::Text("cached".to_owned()),
            &kwargs_with_message(PROMPT),
        )
        .await;
    let creates = index.creates();
    assert_eq!(creates.len(), 1);
    assert_eq!(creates[0].0, INDEX_NAME);
    assert_eq!(
        creates[0].1,
        IndexSchema {
            prefix: format!("{INDEX_NAME}:"),
            dimension: 3,
        }
    );
}

#[tokio::test]
async fn index_remembers_the_dimension_after_a_create() {
    let index = Arc::new(FakeIndex::default());
    let (cache, _connector) = cache_with(Arc::new(FakeEmbedder::new()), &index);
    let kwargs = kwargs_with_message(PROMPT);
    cache
        .async_set_cache("user-1", &Json::Text("first".to_owned()), &kwargs)
        .await;
    cache
        .async_set_cache("user-1", &Json::Text("second".to_owned()), &kwargs)
        .await;
    assert_eq!(index.creates().len(), 1);
}

#[tokio::test]
async fn index_a_new_dimension_creates_the_index_again() {
    let embedder = Arc::new(FakeEmbedder::new());
    let index = Arc::new(FakeIndex::default());
    let (cache, _connector) = cache_with(Arc::clone(&embedder), &index);
    let kwargs = kwargs_with_message(PROMPT);
    cache
        .async_set_cache("user-1", &Json::Text("first".to_owned()), &kwargs)
        .await;
    embedder.set_vector(vec![0.5_f32, 0.5_f32]);
    cache
        .async_set_cache("user-1", &Json::Text("second".to_owned()), &kwargs)
        .await;
    let creates = index.creates();
    assert_eq!(creates.len(), 2);
    assert_eq!(creates[1].1.dimension, 2);
}

#[tokio::test]
async fn index_an_existing_index_with_the_same_dimension_passes() {
    let index = Arc::new(FakeIndex::default().with_info(INDEX_NAME, info_reply(3)));
    index.fail_create_next(exists_error());
    let (cache, _connector) = cache_with(Arc::new(FakeEmbedder::new()), &index);
    cache
        .async_set_cache(
            "user-1",
            &Json::Text("cached".to_owned()),
            &kwargs_with_message(PROMPT),
        )
        .await;
    assert_eq!(index.stores().len(), 1);
}

#[tokio::test]
async fn index_an_existing_index_with_another_dimension_rejects_the_write() {
    let index = Arc::new(FakeIndex::default().with_info(INDEX_NAME, info_reply(4)));
    index.fail_create_next(exists_error());
    let (cache, _connector) = cache_with(Arc::new(FakeEmbedder::new()), &index);
    let kwargs = kwargs_with_message(PROMPT);
    cache
        .async_set_cache("user-1", &Json::Text("first".to_owned()), &kwargs)
        .await;
    assert!(index.stores().is_empty());
    cache
        .async_set_cache("user-1", &Json::Text("second".to_owned()), &kwargs)
        .await;
    assert_eq!(index.creates().len(), 2);
    assert_eq!(index.stores().len(), 1);
}

#[tokio::test]
async fn index_an_exists_error_is_recognized_in_any_case() {
    let index = Arc::new(FakeIndex::default().with_info(INDEX_NAME, info_reply(3)));
    index.fail_create_next(server_error("INDEX ALREADY EXISTS"));
    let (cache, _connector) = cache_with(Arc::new(FakeEmbedder::new()), &index);
    cache
        .async_set_cache(
            "user-1",
            &Json::Text("cached".to_owned()),
            &kwargs_with_message(PROMPT),
        )
        .await;
    assert_eq!(index.stores().len(), 1);
}

#[tokio::test]
async fn index_other_create_errors_are_not_remembered() {
    let index = Arc::new(FakeIndex::default().with_info(INDEX_NAME, info_reply(3)));
    index.fail_create_next(alternative_error());
    let (cache, _connector) = cache_with(Arc::new(FakeEmbedder::new()), &index);
    let kwargs = kwargs_with_message(PROMPT);
    cache
        .async_set_cache("user-1", &Json::Text("first".to_owned()), &kwargs)
        .await;
    assert!(index.stores().is_empty());
    cache
        .async_set_cache("user-1", &Json::Text("second".to_owned()), &kwargs)
        .await;
    assert_eq!(index.creates().len(), 2);
    assert_eq!(index.stores().len(), 1);
}

#[tokio::test]
async fn store_writes_a_document_for_the_prompt() {
    let embedder = Arc::new(FakeEmbedder::new());
    let index = Arc::new(FakeIndex::default());
    let (cache, _connector) = cache_with(Arc::clone(&embedder), &index);
    cache
        .async_set_cache(
            "user-1",
            &Json::Text("cached".to_owned()),
            &kwargs_with_message(PROMPT),
        )
        .await;
    let stores = index.stores();
    assert_eq!(stores.len(), 1);
    assert_eq!(stores[0].1.prompt, PROMPT);
    assert!(stores[0].2.is_none());
    assert_eq!(embedder.calls()[0].0, PROMPT);
}

#[tokio::test]
async fn store_pipeline_writes_every_pair() {
    let index = Arc::new(FakeIndex::default());
    let (cache, _connector) = cache_with(Arc::new(FakeEmbedder::new()), &index);
    let pairs = vec![
        ("user-1".to_owned(), Json::Text("first".to_owned())),
        ("user-2".to_owned(), Json::Text("second".to_owned())),
    ];
    cache
        .async_set_cache_pipeline(&pairs, &kwargs_with_message(PROMPT))
        .await;
    let tags: Vec<String> = index
        .stores()
        .iter()
        .map(|(_, fields, _)| fields.litellm_cache_key.clone())
        .collect();
    assert_eq!(tags.len(), 2);
    assert!(tags.contains(&scope_tag("user-1")));
    assert!(tags.contains(&scope_tag("user-2")));
}

#[rstest]
#[case::no_ttl(None, None)]
#[case::a_ttl(Some(60), Some(60))]
#[case::zero_reaches_expire(Some(0), Some(0))]
#[tokio::test]
async fn store_passes_the_ttl_to_the_hash_unchanged(
    #[case] ttl: Option<i64>,
    #[case] expected: Option<i64>,
) {
    let index = Arc::new(FakeIndex::default());
    let (cache, _connector) = cache_with(Arc::new(FakeEmbedder::new()), &index);
    let mut kwargs = kwargs_with_message(PROMPT);
    kwargs.ttl = ttl;
    cache
        .async_set_cache("user-1", &Json::Text("cached".to_owned()), &kwargs)
        .await;
    assert_eq!(index.stores()[0].2, expected);
}

#[tokio::test]
async fn store_passes_the_request_metadata_to_the_embedder() {
    let embedder = Arc::new(FakeEmbedder::new());
    let index = Arc::new(FakeIndex::default());
    let (cache, _connector) = cache_with(Arc::clone(&embedder), &index);
    let mut kwargs = kwargs_with_message(PROMPT);
    let metadata = Object::from([("user".to_owned(), Json::Text("u-1".to_owned()))]);
    kwargs.metadata = Some(metadata.clone());
    cache
        .async_set_cache("user-1", &Json::Text("cached".to_owned()), &kwargs)
        .await;
    assert_eq!(embedder.calls()[0].1, Some(metadata));
}

#[tokio::test]
async fn store_stores_the_json_text_of_the_value() {
    let index = Arc::new(FakeIndex::default());
    let (cache, _connector) = cache_with(Arc::new(FakeEmbedder::new()), &index);
    let value = Json::Object(Object::from([(
        "answer".to_owned(),
        Json::Text("42".to_owned()),
    )]));
    cache
        .async_set_cache("user-1", &value, &kwargs_with_message(PROMPT))
        .await;
    assert_eq!(index.stores()[0].1.response, "{\"answer\":\"42\"}");
}

#[rstest]
#[case::no_messages(None)]
#[case::empty_messages(Some(Vec::new()))]
#[tokio::test]
async fn store_without_a_prompt_writes_nothing(#[case] messages: Option<Vec<Message>>) {
    let embedder = Arc::new(FakeEmbedder::new());
    let index = Arc::new(FakeIndex::default());
    let (cache, _connector) = cache_with(Arc::clone(&embedder), &index);
    let mut kwargs = kwargs_without_prompt();
    kwargs.messages = messages;
    cache
        .async_set_cache("user-1", &Json::Text("cached".to_owned()), &kwargs)
        .await;
    assert!(index.stores().is_empty());
    assert!(embedder.calls().is_empty());
}

#[rstest]
#[case::embedding(true)]
#[case::backend(false)]
#[tokio::test]
async fn store_swallows_errors(#[case] embedding: bool) {
    let embedder = Arc::new(FakeEmbedder::new());
    let index = Arc::new(FakeIndex::default());
    let (cache, _connector) = cache_with(Arc::clone(&embedder), &index);
    if embedding {
        embedder.fail();
    } else {
        index.fail_store(alternative_error());
    }
    cache
        .async_set_cache(
            "user-1",
            &Json::Text("cached".to_owned()),
            &kwargs_with_message(PROMPT),
        )
        .await;
    assert!(index.stores().is_empty());
}

#[tokio::test]
async fn lookup_returns_the_decoded_response_and_the_similarity() {
    let index = Arc::new(FakeIndex::default());
    index.set_hits(vec![hit(0.25, "{\"text\":\"cached\"}")]);
    let (cache, _connector) = cache_with(Arc::new(FakeEmbedder::new()), &index);
    let mut metadata = Object::new();
    let value = cache
        .async_get_cache("user-1", &kwargs_with_message(PROMPT), Some(&mut metadata))
        .await;
    assert_eq!(
        value,
        Some(Json::Object(Object::from([(
            "text".to_owned(),
            Json::Text("cached".to_owned())
        )])))
    );
    assert_similarity(&metadata, 0.75_f64);
}

#[rstest]
#[case::below_the_threshold(0.875_f64, false, 0.125_f64)]
#[case::equal_to_the_threshold(0.75_f64, true, 0.25_f64)]
#[tokio::test]
async fn lookup_compares_the_similarity_with_the_threshold(
    #[case] distance: f64,
    #[case] is_hit: bool,
    #[case] similarity: f64,
) {
    let index = Arc::new(FakeIndex::default());
    index.set_hits(vec![hit(distance, "{\"text\":\"cached\"}")]);
    let (cache, _connector) = cache_with(Arc::new(FakeEmbedder::new()), &index);
    let mut metadata = Object::new();
    let value = cache
        .async_get_cache("user-1", &kwargs_with_message(PROMPT), Some(&mut metadata))
        .await;
    assert_eq!(value.is_some(), is_hit);
    assert_similarity(&metadata, similarity);
}

#[tokio::test]
async fn lookup_writes_zero_similarity_without_a_hit() {
    let index = Arc::new(FakeIndex::default());
    let (cache, _connector) = cache_with(Arc::new(FakeEmbedder::new()), &index);
    let mut metadata = Object::new();
    let value = cache
        .async_get_cache("user-1", &kwargs_with_message(PROMPT), Some(&mut metadata))
        .await;
    assert!(value.is_none());
    assert_similarity(&metadata, 0.0_f64);
}

#[tokio::test]
async fn lookup_keeps_the_similarity_when_the_response_does_not_decode() {
    let index = Arc::new(FakeIndex::default());
    index.set_hits(vec![hit(0.25, "not json")]);
    let (cache, _connector) = cache_with(Arc::new(FakeEmbedder::new()), &index);
    let mut metadata = Object::new();
    let value = cache
        .async_get_cache("user-1", &kwargs_with_message(PROMPT), Some(&mut metadata))
        .await;
    assert!(value.is_none());
    assert_similarity(&metadata, 0.75_f64);
}

#[tokio::test]
async fn lookup_embeds_the_prompt_and_runs_the_knn_query() {
    let embedder = Arc::new(FakeEmbedder::new());
    let index = Arc::new(FakeIndex::default());
    let (cache, _connector) = cache_with(Arc::clone(&embedder), &index);
    let metadata = Object::from([("user".to_owned(), Json::Text("u-1".to_owned()))]);
    let mut kwargs = kwargs_with_message(PROMPT);
    kwargs.metadata = Some(metadata.clone());
    cache.async_get_cache("user-1", &kwargs, None).await;
    assert_eq!(embedder.calls()[0].0, PROMPT);
    assert_eq!(embedder.calls()[0].1, Some(metadata));
    let searches = index.searches();
    assert_eq!(searches.len(), 1);
    assert_eq!(searches[0].0, INDEX_NAME);
    assert_eq!(searches[0].1, knn_query("user-1"));
    assert_eq!(searches[0].2, packed_vector());
}

#[tokio::test]
async fn lookup_without_a_prompt_writes_zero_and_skips_the_backend() {
    let embedder = Arc::new(FakeEmbedder::new());
    let index = Arc::new(FakeIndex::default());
    let (cache, connector) = cache_with(Arc::clone(&embedder), &index);
    let mut metadata = Object::new();
    let value = cache
        .async_get_cache("user-1", &kwargs_without_prompt(), Some(&mut metadata))
        .await;
    assert!(value.is_none());
    assert_similarity(&metadata, 0.0_f64);
    assert!(embedder.calls().is_empty());
    assert!(connector.connects().is_empty());
}

#[rstest]
#[case::embedding("embedding")]
#[case::connect("connect")]
#[case::create("create")]
#[case::search("search")]
#[tokio::test]
async fn lookup_errors_become_misses_with_zero_similarity(#[case] failure: &str) {
    let embedder = Arc::new(FakeEmbedder::new());
    let index = Arc::new(FakeIndex::default());
    let (cache, connector) = cache_with(Arc::clone(&embedder), &index);
    match failure {
        "embedding" => embedder.fail(),
        "connect" => connector.fail(alternative_error()),
        "create" => index.fail_create_next(alternative_error()),
        "search" => index.fail_search(alternative_error()),
        _ => panic!("a known failure point"),
    }
    let mut metadata = Object::new();
    let value = cache
        .async_get_cache("user-1", &kwargs_with_message(PROMPT), Some(&mut metadata))
        .await;
    assert!(value.is_none());
    assert_similarity(&metadata, 0.0_f64);
}

#[test]
fn lookup_a_non_numeric_distance_keeps_its_parse_source() {
    let reply = vec![
        Value::Int(1),
        Value::BulkString(b"doc".to_vec()),
        Value::Array(vec![
            Value::BulkString(b"response".to_vec()),
            Value::BulkString(b"{}".to_vec()),
            Value::BulkString(b"vector_distance".to_vec()),
            Value::BulkString(b"not-a-number".to_vec()),
        ]),
    ];
    let Err(error) = parse_search_reply(&reply) else {
        panic!("a non-numeric distance must fail");
    };
    assert!(matches!(&error, ValkeySemanticError::MalformedDistance(_)));
    assert!(error.source().is_some());
}

#[tokio::test]
async fn lookup_without_metadata_returns_the_hit() {
    let index = Arc::new(FakeIndex::default());
    index.set_hits(vec![hit(0.25, "{\"text\":\"cached\"}")]);
    let (cache, _connector) = cache_with(Arc::new(FakeEmbedder::new()), &index);
    let value = cache
        .async_get_cache("user-1", &kwargs_with_message(PROMPT), None)
        .await;
    assert!(value.is_some());
}

#[tokio::test]
async fn info_returns_the_ft_info_reply_from_the_injected_index_without_connecting() {
    let index = Arc::new(FakeIndex::default().with_info(INDEX_NAME, info_reply(3)));
    let (cache, connector) = cache_with_injected_index(&index);
    assert_eq!(cache.index_info().await.expect("the reply"), info_reply(3));
    assert!(connector.connects().is_empty());
}

#[tokio::test]
async fn info_opens_the_connector_once_with_the_resolved_url() {
    let index = Arc::new(FakeIndex::default().with_info(INDEX_NAME, info_reply(3)));
    let (cache, connector) = cache_with(Arc::new(FakeEmbedder::new()), &index);
    assert_eq!(cache.index_info().await.expect("the first"), info_reply(3));
    cache.index_info().await.expect("the second reply");
    assert_eq!(connector.connects(), vec!["redis://:pw@localhost:6379"]);
}

#[tokio::test]
async fn info_propagates_backend_errors() {
    let index = Arc::new(FakeIndex::default());
    index.fail_info(alternative_error());
    let (cache, _connector) = cache_with(Arc::new(FakeEmbedder::new()), &index);
    let result = cache.index_info().await;
    let Err(error) = result else {
        panic!("an FT.INFO failure must surface");
    };
    assert_eq!(error.to_string(), alternative_error().to_string());
}

#[tokio::test]
async fn info_live_index_reads_simple_string_field_names() {
    let server = FakeValkey::start(&[("FT.INFO", SIMPLE_STRING_INFO)]).await;
    let info = live_index(&server)
        .await
        .ft_info(INDEX_NAME)
        .await
        .expect("the parsed FT.INFO reply");
    assert_eq!(extract_index_dim(&info), Some(3));
    let Json::Object(fields) = info else {
        panic!("FT.INFO must parse into an object");
    };
    assert_eq!(
        fields.get("index_name"),
        Some(&Json::Text("idx".to_owned()))
    );
}

#[rstest]
#[case::no_ttl(None)]
#[case::a_ttl(Some(60))]
#[case::zero(Some(0))]
#[tokio::test]
async fn store_live_index_sends_expire_after_the_hash_only_with_a_ttl(#[case] ttl: Option<i64>) {
    let server = FakeValkey::start(&[]).await;
    let fields = doc_mapping("user-1", PROMPT, "\"cached\"", &VECTOR);
    live_index(&server)
        .await
        .store_hash("doc-1", &fields, ttl)
        .await
        .expect("the stored hash");
    let sent = server.received(&[b"HSET", b"EXPIRE"]);
    let names: Vec<&[u8]> = sent.iter().map(|command| command[0].as_slice()).collect();
    let Some(seconds) = ttl else {
        assert_eq!(names, [b"HSET".as_slice()]);
        return;
    };
    assert_eq!(names, [b"HSET".as_slice(), b"EXPIRE".as_slice()]);
    assert_eq!(
        sent[1],
        [
            b"EXPIRE".to_vec(),
            b"doc-1".to_vec(),
            seconds.to_string().into_bytes()
        ]
    );
}

#[rstest]
#[case::one_document(ONE_HIT, vec![Hit { response: "\"cached\"".to_owned(), distance: 0.25 }])]
#[case::empty("*1\r\n:0\r\n", Vec::new())]
#[tokio::test]
async fn lookup_live_index_parses_the_search_reply(
    #[case] reply: &str,
    #[case] expected: Vec<Hit>,
) {
    let server = FakeValkey::start(&[("FT.SEARCH", reply)]).await;
    let hits = live_index(&server)
        .await
        .ft_search(INDEX_NAME, &knn_query("user-1"), &packed_vector())
        .await
        .expect("the parsed hits");
    assert_eq!(hits, expected);
}

#[test]
fn schema_extract_index_dim_scans_past_attributes_without_a_dimension() {
    let info = info_with_attributes(vec![tag_attribute(), vector_attribute(768)]);
    assert_eq!(extract_index_dim(&info), Some(768));
}
